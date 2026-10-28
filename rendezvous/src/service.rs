//! The rendezvous service, without IO: feed it datagrams with [`Service::handle`], call
//! [`Service::tick`] regularly, send what it puts in the outbox. The binary
//! (`src/bin/velocide-rendezvous.rs`) is the UDP loop around it; tests drive it directly.
//!
//! Security rules it keeps (docs/rendezvous.md "Security"):
//! - A sender whose address isn't proven (no valid cookie) gets at most one reply no bigger
//!   than its datagram, from a global budget: the service can't be used to reflect or
//!   amplify traffic at someone else.
//! - Anything that creates state (listings, joins, relays) needs a cookie, and is limited per
//!   source IP and in total.
//! - Hosts prove ownership of their listing with a secret; relay ends are bound with
//!   credentials; both are MACs under the service key, checked in constant time.
//! - Relayed traffic only flows between the two ends of a session, within per-session,
//!   global and daily byte budgets.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::code::Code;
use crate::crypto::{HostId, Keys, SECRET_LEN, TAG_LEN, ct_eq, random_bytes, random_nonzero_u64};
use crate::limit::{IpLimiter, IpRates, TokenBucket};
use crate::proto::{
    self, DecodeError, ErrorCode, HostInfo, Introduction, JoinFailure, LIST_PAGE_LEN, ListEntry,
    Message, RelayGrant, Resume, Role, cookie_window, encode, encode_introduce, encode_relay,
};
use crate::wire::canonical;

/// A listing without a heartbeat for this long is dropped (hosts send one every 2 s).
pub const LISTING_TIMEOUT: Duration = Duration::from_secs(10);
/// How often an unacknowledged introduction is sent again, and how many times in all.
pub const INTRO_RESEND: Duration = Duration::from_millis(250);
pub const INTRO_SENDS: u8 = 12;
/// A relay session without traffic for this long is closed.
pub const RELAY_IDLE: Duration = Duration::from_secs(30);
/// How long relay credentials are valid (a session can't outlive them).
pub const RELAY_GRANT_SECS: u64 = 24 * 3600;
/// Pending introductions per listing (joins in flight).
pub const MAX_PENDING_INTROS: usize = 16;
/// Bans a listing keeps (the oldest go first).
pub const MAX_LISTING_BANS: usize = 64;
/// Replies to `Register` and `Join` are remembered this long, so a retried request (its reply
/// was lost) gets the same answer instead of a second listing or join.
pub const RECENT_TTL: Duration = Duration::from_secs(15);
pub const MAX_RECENT: usize = 8192;

/// Limits; the defaults suit a small VPS.
#[derive(Clone, Debug)]
pub struct ServiceConfig {
    /// What `Probe` answers (the release the service runs).
    pub version: String,
    pub max_listings: usize,
    pub max_listings_per_ip: usize,
    pub max_relays: usize,
    pub max_relays_per_host: usize,
    pub max_relays_per_ip: usize,
    /// Relayed bytes per second toward each end of a session, and the burst (level loads).
    pub relay_rate: u64,
    pub relay_burst: u64,
    /// Relayed bytes per second over all sessions.
    pub relay_total_rate: u64,
    /// Relayed bytes per UTC day over all sessions (caps the VPS's traffic bill).
    pub relay_daily_bytes: u64,
    /// Replies per second to senders whose address isn't proven (`Hello`, `Probe`, errors).
    pub unverified_rate: u64,
    pub ip_rates: IpRates,
    pub max_tracked_ips: usize,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            version: String::from("dev"),
            max_listings: 1024,
            max_listings_per_ip: 4,
            max_relays: 512,
            max_relays_per_host: 16,
            max_relays_per_ip: 8,
            relay_rate: 64 * 1024,
            relay_burst: 512 * 1024,
            relay_total_rate: 2 * 1024 * 1024,
            relay_daily_bytes: 20 * 1000 * 1000 * 1000,
            unverified_rate: 2000,
            ip_rates: IpRates::default(),
            max_tracked_ips: 65_536,
        }
    }
}

/// Counters (logged by the binary).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceStats {
    pub packets: u64,
    pub malformed: u64,
    pub bad_cookies: u64,
    pub rate_limited: u64,
    pub refused: u64,
    pub unverified_dropped: u64,
    pub registrations: u64,
    pub joins: u64,
    pub join_failures: u64,
    pub relayed_packets: u64,
    pub relayed_bytes: u64,
    pub relay_dropped: u64,
    pub listings: usize,
    pub relays: usize,
}

/// Datagrams to send: (destination, bytes).
pub type Outbox = Vec<(SocketAddr, Vec<u8>)>;

struct Listing {
    code: Code,
    secret: [u8; SECRET_LEN],
    addr: SocketAddr,
    lan: Option<SocketAddr>,
    info: HostInfo,
    last_seen: Duration,
    bans: VecDeque<[u8; TAG_LEN]>,
    intros: Vec<PendingIntro>,
}

struct PendingIntro {
    join_id: u64,
    datagram: Vec<u8>,
    next_send: Duration,
    sends_left: u8,
}

struct Relay {
    host_id: HostId,
    /// Bound address of each end.
    host: Option<SocketAddr>,
    joiner: Option<SocketAddr>,
    /// The joiner's IP (per-IP limits).
    joiner_ip: Option<IpAddr>,
    /// Traffic toward each end.
    toward_host: TokenBucket,
    toward_joiner: TokenBucket,
    last_active: Duration,
    /// Unix seconds: the end of its credentials.
    expires: u64,
}

impl Relay {
    fn end_mut(&mut self, role: Role) -> &mut Option<SocketAddr> {
        match role {
            Role::Host => &mut self.host,
            Role::Joiner => &mut self.joiner,
        }
    }

    /// Which end `from` is bound as.
    fn role_of(&self, from: SocketAddr) -> Option<Role> {
        if self.host == Some(from) {
            Some(Role::Host)
        } else if self.joiner == Some(from) {
            Some(Role::Joiner)
        } else {
            None
        }
    }
}

/// LAN candidates are only passed on if they look like one: a private IPv4 address and a
/// non-system port (a host can't make joiners send to arbitrary local services).
pub fn is_lan_candidate(addr: &SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(ip) => ip.is_private() && addr.port() >= 1024,
        IpAddr::V6(_) => false,
    }
}

pub struct Service {
    keys: Keys,
    config: ServiceConfig,
    listings: HashMap<HostId, Listing>,
    codes: HashMap<Code, HostId>,
    relays: HashMap<u64, Relay>,
    limiter: IpLimiter,
    unverified: TokenBucket,
    relay_total: TokenBucket,
    relay_day: u64,
    relay_day_bytes: u64,
    /// Replies to recent `Register`/`Join` requests by (sender, nonce).
    recent: HashMap<(SocketAddr, u64), (Duration, Vec<u8>)>,
    last_prune: Duration,
    stats: ServiceStats,
}

impl Service {
    pub fn new(master_key: &[u8; 32], config: ServiceConfig, now: Duration) -> Self {
        let unverified = config.unverified_rate as f64;
        let total = config.relay_total_rate as f64;
        Self {
            keys: Keys::derive(master_key),
            limiter: IpLimiter::new(config.ip_rates, config.max_tracked_ips),
            unverified: TokenBucket::new(unverified * 2.0, unverified, now),
            relay_total: TokenBucket::new(total, total, now),
            relay_day: 0,
            relay_day_bytes: 0,
            config,
            listings: HashMap::new(),
            codes: HashMap::new(),
            relays: HashMap::new(),
            recent: HashMap::new(),
            last_prune: now,
            stats: ServiceStats::default(),
        }
    }

    pub fn stats(&self) -> ServiceStats {
        let mut s = self.stats.clone();
        s.listings = self.listings.len();
        s.relays = self.relays.len();
        s
    }

    /// Handles one datagram from `from`. `now` is monotonic time, `unix` wall-clock seconds.
    pub fn handle(
        &mut self,
        now: Duration,
        unix: u64,
        from: SocketAddr,
        data: &[u8],
        out: &mut Outbox,
    ) {
        let from = canonical(from);
        self.stats.packets += 1;
        let msg = match proto::decode(data) {
            Ok(msg) => msg,
            Err(DecodeError::Version(_)) => {
                let reply = encode(&Message::Error {
                    nonce: 0,
                    code: ErrorCode::UnsupportedVersion,
                });
                self.reply_unverified(now, from, data.len(), reply, out);
                return;
            }
            Err(_) => {
                self.stats.malformed += 1;
                return;
            }
        };
        match msg {
            Message::Hello { nonce } => {
                let cookie = self.keys.cookie(from, cookie_window(unix));
                let reply = encode(&Message::Cookie {
                    nonce,
                    cookie,
                    observed: from,
                });
                self.reply_unverified(now, from, data.len(), reply, out);
            }
            Message::Probe { nonce } => {
                let reply = encode(&Message::ProbeReply {
                    nonce,
                    version: self.config.version.clone(),
                });
                self.reply_unverified(now, from, data.len(), reply, out);
            }
            Message::Relay { session, payload } => {
                self.relay(now, unix, from, session, payload, data.len(), out);
            }
            msg => {
                // Replies (Cookie, Ok, ...) and host↔joiner punches have no business here.
                let Some(cookie) = msg.cookie() else {
                    self.stats.malformed += 1;
                    return;
                };
                if !self.cookie_ok(from, cookie, unix) {
                    self.stats.bad_cookies += 1;
                    let reply = encode(&Message::Error {
                        nonce: msg.nonce(),
                        code: ErrorCode::BadCookie,
                    });
                    self.reply_unverified(now, from, data.len(), reply, out);
                    return;
                }
                // From here on the sender is known to receive at `from`.
                if !self.limiter.allow_control(from.ip(), now) {
                    self.stats.rate_limited += 1;
                    if msg.nonce() != 0 {
                        self.error(from, msg.nonce(), ErrorCode::RateLimited, out);
                    }
                    return;
                }
                self.request(now, unix, from, msg, out);
            }
        }
    }

    /// Timers: resends introductions, expires listings and relays, prunes tables.
    pub fn tick(&mut self, now: Duration, unix: u64, out: &mut Outbox) {
        for l in self.listings.values_mut() {
            for intro in l.intros.iter_mut() {
                if now >= intro.next_send && intro.sends_left > 0 {
                    out.push((l.addr, intro.datagram.clone()));
                    intro.sends_left -= 1;
                    intro.next_send = now + INTRO_RESEND;
                }
            }
            l.intros.retain(|i| i.sends_left > 0);
        }
        let codes = &mut self.codes;
        self.listings.retain(|_, l| {
            let keep = now.saturating_sub(l.last_seen) < LISTING_TIMEOUT;
            if !keep {
                codes.remove(&l.code);
            }
            keep
        });
        self.relays
            .retain(|_, r| now.saturating_sub(r.last_active) < RELAY_IDLE && unix < r.expires);
        if now.saturating_sub(self.last_prune) >= Duration::from_secs(10) {
            self.last_prune = now;
            self.limiter.prune(now);
            self.recent
                .retain(|_, (at, _)| now.saturating_sub(*at) < RECENT_TTL);
        }
    }

    fn cookie_ok(&self, from: SocketAddr, cookie: &[u8; 16], unix: u64) -> bool {
        let window = cookie_window(unix);
        ct_eq(cookie, &self.keys.cookie(from, window))
            || (window > 0 && ct_eq(cookie, &self.keys.cookie(from, window - 1)))
    }

    /// A reply to an unproven sender: never bigger than its datagram, within a global budget.
    fn reply_unverified(
        &mut self,
        now: Duration,
        to: SocketAddr,
        request_len: usize,
        reply: Vec<u8>,
        out: &mut Outbox,
    ) {
        if reply.len() <= request_len && self.unverified.take(now, 1.0) {
            out.push((to, reply));
        } else {
            self.stats.unverified_dropped += 1;
        }
    }

    fn error(&mut self, to: SocketAddr, nonce: u64, code: ErrorCode, out: &mut Outbox) {
        if code == ErrorCode::Refused {
            self.stats.refused += 1;
        }
        out.push((to, encode(&Message::Error { nonce, code })));
    }

    fn listings_from(&self, ip: IpAddr) -> usize {
        self.listings.values().filter(|l| l.addr.ip() == ip).count()
    }

    /// The listing `host_id`, if `secret` proves the sender owns it.
    fn owned(&self, host_id: &HostId, secret: &[u8; SECRET_LEN]) -> Result<(), ErrorCode> {
        match self.listings.get(host_id) {
            None => Err(ErrorCode::UnknownHost),
            Some(l) if ct_eq(&l.secret, secret) => Ok(()),
            Some(_) => Err(ErrorCode::Refused),
        }
    }

    fn remember(&mut self, now: Duration, from: SocketAddr, nonce: u64, reply: &[u8]) {
        if self.recent.len() >= MAX_RECENT {
            self.recent
                .retain(|_, (at, _)| now.saturating_sub(*at) < RECENT_TTL);
        }
        if self.recent.len() < MAX_RECENT {
            self.recent.insert((from, nonce), (now, reply.to_vec()));
        }
    }

    /// The earlier reply to the same request, if it is a retry.
    fn recalled(&self, now: Duration, from: SocketAddr, nonce: u64) -> Option<Vec<u8>> {
        self.recent
            .get(&(from, nonce))
            .filter(|(at, _)| now.saturating_sub(*at) < RECENT_TTL)
            .map(|(_, reply)| reply.clone())
    }

    fn request(
        &mut self,
        now: Duration,
        unix: u64,
        from: SocketAddr,
        msg: Message,
        out: &mut Outbox,
    ) {
        match msg {
            Message::Register {
                nonce,
                resume,
                lan,
                info,
                bans,
                ..
            } => self.register(now, from, nonce, resume, lan, info, bans, out),
            Message::Heartbeat {
                nonce,
                host_id,
                secret,
                info,
                ..
            } => self.heartbeat(now, from, nonce, &host_id, &secret, info, out),
            Message::Unregister {
                nonce,
                host_id,
                secret,
                ..
            } => match self.owned(&host_id, &secret) {
                Ok(()) => {
                    if let Some(l) = self.listings.remove(&host_id) {
                        self.codes.remove(&l.code);
                    }
                    self.relays.retain(|_, r| r.host_id != host_id);
                    out.push((from, encode(&Message::Ok { nonce })));
                }
                Err(code) => self.error(from, nonce, code, out),
            },
            Message::IntroAck {
                host_id,
                secret,
                join_id,
                ..
            } => {
                if self.owned(&host_id, &secret).is_ok()
                    && let Some(l) = self.listings.get_mut(&host_id)
                {
                    l.intros.retain(|i| i.join_id != join_id);
                }
            }
            Message::Ban {
                nonce,
                host_id,
                secret,
                tag,
                ..
            } => match self.owned(&host_id, &secret) {
                Ok(()) => {
                    if let Some(l) = self.listings.get_mut(&host_id) {
                        add_ban(&mut l.bans, tag);
                    }
                    out.push((from, encode(&Message::Ok { nonce })));
                }
                Err(code) => self.error(from, nonce, code, out),
            },
            Message::List {
                nonce,
                protocol,
                page,
                ..
            } => self.list(from, nonce, protocol, page, out),
            Message::Join {
                nonce,
                code,
                protocol,
                client_ids,
                lan,
                relay_only,
                ..
            } => self.join(
                now, unix, from, nonce, code, protocol, client_ids, lan, relay_only, out,
            ),
            Message::RelayBind {
                nonce,
                session,
                role,
                grant_expires,
                host_id,
                cred,
                ..
            } => self.relay_bind(
                now,
                unix,
                from,
                nonce,
                session,
                role,
                grant_expires,
                &host_id,
                &cred,
                out,
            ),
            _ => self.stats.malformed += 1,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn register(
        &mut self,
        now: Duration,
        from: SocketAddr,
        nonce: u64,
        resume: Option<Resume>,
        lan: Option<SocketAddr>,
        info: HostInfo,
        bans: Vec<[u8; TAG_LEN]>,
        out: &mut Outbox,
    ) {
        if resume.is_none()
            && let Some(reply) = self.recalled(now, from, nonce)
        {
            out.push((from, reply));
            return;
        }
        // A resumed listing keeps its id and code if it proves they were given to it.
        let mut identity = match resume {
            Some(r) => {
                if !ct_eq(&r.secret, &self.keys.host_secret(&r.host_id, &r.code)) {
                    self.error(from, nonce, ErrorCode::Refused, out);
                    return;
                }
                Some((r.host_id, r.code, r.secret))
            }
            None => None,
        };
        // After a restart someone else may have been given the code meanwhile.
        if let Some((host_id, code, _)) = &identity
            && self.codes.get(code).is_some_and(|other| other != host_id)
        {
            identity = None;
        }
        let existing = identity
            .as_ref()
            .and_then(|(host_id, _, _)| self.listings.get(host_id))
            .map(|l| l.addr.ip());
        let moves_ip = existing != Some(from.ip());
        if (existing.is_none() && self.listings.len() >= self.config.max_listings)
            || (moves_ip && self.listings_from(from.ip()) >= self.config.max_listings_per_ip)
        {
            self.error(from, nonce, ErrorCode::Busy, out);
            return;
        }
        let (host_id, code, secret) = match identity {
            Some(identity) => identity,
            None => match self.new_identity() {
                Some(identity) => identity,
                None => {
                    self.error(from, nonce, ErrorCode::Busy, out);
                    return;
                }
            },
        };
        let lan = lan.filter(is_lan_candidate);
        let listing = self.listings.entry(host_id).or_insert_with(|| Listing {
            code,
            secret,
            addr: from,
            lan,
            info: HostInfo::default(),
            last_seen: now,
            bans: VecDeque::new(),
            intros: Vec::new(),
        });
        listing.addr = from;
        listing.lan = lan;
        listing.info = normalized(info);
        listing.last_seen = now;
        for tag in bans {
            add_ban(&mut listing.bans, tag);
        }
        self.codes.insert(code, host_id);
        self.stats.registrations += 1;
        let reply = encode(&Message::Registered {
            nonce,
            host_id,
            code,
            secret,
        });
        self.remember(now, from, nonce, &reply);
        out.push((from, reply));
    }

    fn new_identity(&self) -> Option<(HostId, Code, [u8; SECRET_LEN])> {
        let host_id: HostId = random_bytes();
        (0..16)
            .map(|_| Code::random())
            .find(|code| !self.codes.contains_key(code))
            .map(|code| (host_id, code, self.keys.host_secret(&host_id, &code)))
    }

    #[allow(clippy::too_many_arguments)]
    fn heartbeat(
        &mut self,
        now: Duration,
        from: SocketAddr,
        nonce: u64,
        host_id: &HostId,
        secret: &[u8; SECRET_LEN],
        info: HostInfo,
        out: &mut Outbox,
    ) {
        if let Err(code) = self.owned(host_id, secret) {
            self.error(from, nonce, code, out);
            return;
        }
        let moves_ip = self
            .listings
            .get(host_id)
            .is_some_and(|l| l.addr.ip() != from.ip());
        if moves_ip && self.listings_from(from.ip()) >= self.config.max_listings_per_ip {
            self.error(from, nonce, ErrorCode::Busy, out);
            return;
        }
        if let Some(l) = self.listings.get_mut(host_id) {
            // The host's NAT may have given it a new port (or it moved networks).
            l.addr = from;
            l.info = normalized(info);
            l.last_seen = now;
        }
        out.push((from, encode(&Message::Ok { nonce })));
    }

    fn list(&mut self, from: SocketAddr, nonce: u64, protocol: u32, page: u16, out: &mut Outbox) {
        let mut matching: Vec<&Listing> = self
            .listings
            .values()
            .filter(|l| l.info.public && l.info.protocol == protocol)
            .collect();
        // Fullest games first, then by name; the code makes the order total, so pages don't
        // overlap.
        matching.sort_by(|a, b| {
            b.info
                .players
                .cmp(&a.info.players)
                .then_with(|| a.info.name.cmp(&b.info.name))
                .then_with(|| a.code.cmp(&b.code))
        });
        let total = matching.len();
        let pages = total.div_ceil(LIST_PAGE_LEN).clamp(1, u16::MAX as usize);
        let entries: Vec<ListEntry> = matching
            .iter()
            .skip((page as usize).saturating_mul(LIST_PAGE_LEN))
            .take(LIST_PAGE_LEN)
            .map(|l| ListEntry {
                code: l.code,
                name: l.info.name.clone(),
                mode: l.info.mode,
                level: l.info.level.clone(),
                players: l.info.players,
                max_players: l.info.max_players,
                in_match: l.info.in_match,
                relay_only: l.info.relay_only,
            })
            .collect();
        out.push((
            from,
            encode(&Message::ListPage {
                nonce,
                page,
                pages: pages as u16,
                total: total.min(u32::MAX as usize) as u32,
                entries,
            }),
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn join(
        &mut self,
        now: Duration,
        unix: u64,
        from: SocketAddr,
        nonce: u64,
        code: Code,
        protocol: u32,
        client_ids: [u64; 3],
        lan: Option<SocketAddr>,
        joiner_relay_only: bool,
        out: &mut Outbox,
    ) {
        if let Some(reply) = self.recalled(now, from, nonce) {
            out.push((from, reply));
            return;
        }
        let fail = |this: &mut Self, reason: JoinFailure, host_build: String, out: &mut Outbox| {
            this.stats.join_failures += 1;
            let reply = encode(&Message::JoinFailed {
                nonce,
                reason,
                host_build,
            });
            this.remember(now, from, nonce, &reply);
            out.push((from, reply));
        };
        // Every attempt guesses a code: few per IP.
        if !self.limiter.allow_join(from.ip(), now) {
            self.stats.rate_limited += 1;
            out.push((
                from,
                encode(&Message::JoinFailed {
                    nonce,
                    reason: JoinFailure::RateLimited,
                    host_build: String::new(),
                }),
            ));
            return;
        }
        self.stats.joins += 1;
        let Some(host_id) = self.codes.get(&code).copied() else {
            fail(self, JoinFailure::NotFound, String::new(), out);
            return;
        };
        let Some(listing) = self.listings.get(&host_id) else {
            fail(self, JoinFailure::NotFound, String::new(), out);
            return;
        };
        if listing.info.protocol != protocol {
            let build = listing.info.build.clone();
            fail(self, JoinFailure::VersionMismatch, build, out);
            return;
        }
        let tag = self.keys.ban_tag(&host_id, from.ip());
        if listing.bans.iter().any(|b| ct_eq(b, &tag)) {
            fail(self, JoinFailure::Banned, String::new(), out);
            return;
        }
        if listing.intros.len() >= MAX_PENDING_INTROS {
            fail(self, JoinFailure::Busy, String::new(), out);
            return;
        }
        let relay_only = joiner_relay_only || listing.info.relay_only;
        let host_addr = listing.addr;
        let same_nat = host_addr.ip() == from.ip();
        let host_lan = listing.lan.filter(|_| same_nat);
        let relay = self.open_relay(now, unix, host_id, host_addr, from);
        if relay.is_none() && relay_only {
            fail(self, JoinFailure::Busy, String::new(), out);
            return;
        }
        let grant = |keys: &Keys, role: Role| {
            relay.map(|(session, expires)| RelayGrant {
                session,
                expires,
                host_id,
                cred: keys.relay_cred(session, role as u8, expires, &host_id),
            })
        };
        let join_id = random_nonzero_u64();
        let intro = Introduction {
            host_id,
            join_id,
            client_ids,
            joiner: (!relay_only).then_some(from),
            joiner_lan: lan.filter(|a| same_nat && is_lan_candidate(a)),
            relay: grant(&self.keys, Role::Host),
            ban_tag: tag,
        };
        let joiner_grant = grant(&self.keys, Role::Joiner);
        let Some(listing) = self.listings.get_mut(&host_id) else {
            return;
        };
        let datagram = encode_introduce(&intro, &listing.secret);
        out.push((listing.addr, datagram.clone()));
        listing.intros.push(PendingIntro {
            join_id,
            datagram,
            next_send: now + INTRO_RESEND,
            sends_left: INTRO_SENDS - 1,
        });
        let reply = encode(&Message::JoinOk {
            nonce,
            join_id,
            host: (!relay_only).then_some(host_addr),
            host_lan,
            relay: joiner_grant,
        });
        self.remember(now, from, nonce, &reply);
        out.push((from, reply));
    }

    fn relay_count(&self, host_id: &HostId, joiner_ip: Option<IpAddr>) -> Option<()> {
        let c = &self.config;
        let per_host = self
            .relays
            .values()
            .filter(|r| r.host_id == *host_id)
            .count();
        let per_ip = joiner_ip.map_or(0, |ip| {
            self.relays
                .values()
                .filter(|r| r.joiner_ip == Some(ip))
                .count()
        });
        (self.relays.len() < c.max_relays
            && per_host < c.max_relays_per_host
            && per_ip < c.max_relays_per_ip)
            .then_some(())
    }

    fn new_relay(&self, now: Duration, host_id: HostId, expires: u64) -> Relay {
        let rate = self.config.relay_rate as f64;
        let burst = self.config.relay_burst as f64;
        Relay {
            host_id,
            host: None,
            joiner: None,
            joiner_ip: None,
            toward_host: TokenBucket::new(burst, rate, now),
            toward_joiner: TokenBucket::new(burst, rate, now),
            last_active: now,
            expires,
        }
    }

    /// A relay session between a host and a joiner, if the limits allow one.
    fn open_relay(
        &mut self,
        now: Duration,
        unix: u64,
        host_id: HostId,
        host: SocketAddr,
        joiner: SocketAddr,
    ) -> Option<(u64, u64)> {
        self.relay_count(&host_id, Some(joiner.ip()))?;
        let session = loop {
            let s = random_nonzero_u64();
            if !self.relays.contains_key(&s) {
                break s;
            }
        };
        let expires = unix.saturating_add(RELAY_GRANT_SECS);
        let mut relay = self.new_relay(now, host_id, expires);
        relay.host = Some(host);
        relay.joiner = Some(joiner);
        relay.joiner_ip = Some(joiner.ip());
        self.relays.insert(session, relay);
        Some((session, expires))
    }

    /// (Re)binds one end of a relay session to the sender: after the service restarted, or
    /// when the end's NAT gave it a new address.
    #[allow(clippy::too_many_arguments)]
    fn relay_bind(
        &mut self,
        now: Duration,
        unix: u64,
        from: SocketAddr,
        nonce: u64,
        session: u64,
        role: Role,
        grant_expires: u64,
        host_id: &HostId,
        cred: &[u8; 16],
        out: &mut Outbox,
    ) {
        let valid = session != 0
            && grant_expires > unix
            && ct_eq(
                cred,
                &self
                    .keys
                    .relay_cred(session, role as u8, grant_expires, host_id),
            );
        if !valid {
            self.error(from, nonce, ErrorCode::Refused, out);
            return;
        }
        let joiner_ip = (role == Role::Joiner).then_some(from.ip());
        if !self.relays.contains_key(&session) {
            if self.relay_count(host_id, joiner_ip).is_none() {
                self.error(from, nonce, ErrorCode::Busy, out);
                return;
            }
            let relay = self.new_relay(now, *host_id, grant_expires);
            self.relays.insert(session, relay);
        }
        let Some(relay) = self.relays.get_mut(&session) else {
            return;
        };
        if relay.host_id != *host_id {
            // Can't happen with a valid credential (the session id is part of it).
            self.error(from, nonce, ErrorCode::Refused, out);
            return;
        }
        *relay.end_mut(role) = Some(from);
        if relay.joiner_ip.is_none() {
            relay.joiner_ip = joiner_ip;
        }
        relay.last_active = now;
        out.push((from, encode(&Message::RelayBound { nonce, session })));
    }

    #[allow(clippy::too_many_arguments)]
    fn relay(
        &mut self,
        now: Duration,
        unix: u64,
        from: SocketAddr,
        session: u64,
        payload: &[u8],
        len: usize,
        out: &mut Outbox,
    ) {
        let Some(role) = self.relays.get(&session).and_then(|r| r.role_of(from)) else {
            // Unknown here: tell the sender so it binds again (not bigger than its packet).
            let reply = encode(&Message::RelayUnknown { session });
            self.reply_unverified(now, from, len, reply, out);
            return;
        };
        let day = unix / 86_400;
        if day != self.relay_day {
            self.relay_day = day;
            self.relay_day_bytes = 0;
        }
        let Some(relay) = self.relays.get_mut(&session) else {
            return;
        };
        let (to, bucket) = match role {
            Role::Host => (relay.joiner, &mut relay.toward_joiner),
            Role::Joiner => (relay.host, &mut relay.toward_host),
        };
        let Some(to) = to else {
            self.stats.relay_dropped += 1;
            return;
        };
        let bytes = len as u64;
        let within_day =
            self.relay_day_bytes.saturating_add(bytes) <= self.config.relay_daily_bytes;
        if !within_day
            || !bucket.take(now, bytes as f64)
            || !self.relay_total.take(now, bytes as f64)
        {
            self.stats.relay_dropped += 1;
            return;
        }
        relay.last_active = now;
        self.relay_day_bytes += bytes;
        self.stats.relayed_packets += 1;
        self.stats.relayed_bytes += bytes;
        out.push((to, encode_relay(session, payload)));
    }
}

fn add_ban(bans: &mut VecDeque<[u8; TAG_LEN]>, tag: [u8; TAG_LEN]) {
    if bans.iter().any(|b| ct_eq(b, &tag)) {
        return;
    }
    if bans.len() >= MAX_LISTING_BANS {
        bans.pop_front();
    }
    bans.push_back(tag);
}

/// Tidies what a host says about itself before showing it to others.
fn normalized(mut info: HostInfo) -> HostInfo {
    let name = info.name.split_whitespace().collect::<Vec<_>>().join(" ");
    info.name = if name.is_empty() {
        String::from("VELOCIDE GAME")
    } else {
        name
    };
    info.players = info.players.min(info.max_players.max(1));
    info
}
