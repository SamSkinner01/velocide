//! A host's side of the service, without IO: the listing (register, heartbeats), the
//! introductions of joiners (hole punching, relay grants) and the admission gate.
//!
//! The game's server transport owns the UDP socket netcode uses; it passes every datagram it
//! receives through [`HostAgent::on_receive`] and every packet netcode sends through
//! [`HostAgent::outbound`], sends what [`HostAgent::poll`] produces, and asks
//! [`HostAgent::admit`] before letting a new netcode connection in.
//!
//! **Admission** (with [`HostConfig::gate`]): a packet from the internet reaches netcode only
//! if it comes from a joiner the service introduced (its IP, or its relay session), and a
//! connection is only accepted with one of the three netcode client ids that joiner
//! announced, once. So a join code really is the key: knowing the host's address isn't
//! enough, and a kicked (banned) joiner can't come back by address either. Loopback (the
//! host's own client) and, if trusted, the LAN skip the gate.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::code::Code;
use crate::crypto::{HostId, SECRET_LEN, TAG_LEN, ct_eq, message_mac};
use crate::proto::{
    self, ErrorCode, HostInfo, Introduction, Message, RelayGrant, Resume, Role, encode,
    encode_relay, introduce_signed_part,
};
use crate::wire::canonical;
use crate::{Inbound, Outbound, Pending, Resolver, relay_addr, relay_session};

const HELLO_RETRY: Duration = Duration::from_millis(500);
/// Cookies are valid 60–120 s: get a new one well before.
const COOKIE_REFRESH: Duration = Duration::from_secs(45);
const REGISTER_RETRY: Duration = Duration::from_secs(1);
const HEARTBEAT: Duration = Duration::from_secs(2);
/// A changed player count is sent early, but not more often than this.
const HEARTBEAT_MIN_GAP: Duration = Duration::from_millis(500);
/// No answer for this long: the service is unreachable (keep trying).
const OFFLINE_AFTER: Duration = Duration::from_secs(8);
/// After Busy / RateLimited.
const BACKOFF: Duration = Duration::from_secs(10);
const RESOLVE_RETRY: Duration = Duration::from_secs(30);
/// An introduction not used by then is forgotten.
const INTRO_TTL: Duration = Duration::from_secs(30);
const PUNCH_EVERY: Duration = Duration::from_millis(100);
const PUNCHES: u8 = 15;
const REBIND_RETRY: Duration = Duration::from_millis(500);
const BAN_RETRY: Duration = Duration::from_secs(1);
/// Join ids remembered to ignore resent introductions.
const SEEN_JOINS: usize = 64;
const MAX_INTROS: usize = 32;
/// Joins in flight from one joiner (by ban tag, i.e. public IP) kept at once.
const MAX_INTROS_PER_JOINER: usize = 3;
const MAX_BANS: usize = proto::MAX_BANS;
const MAX_BANNED_IPS: usize = 1024;

#[derive(Clone, Debug)]
pub struct HostConfig {
    /// `host:port` of the service.
    pub service: String,
    pub info: HostInfo,
    /// Our LAN address (given to joiners behind the same public IP).
    pub lan: Option<SocketAddr>,
    /// Admit only introduced joiners from the internet (see the module docs).
    pub gate: bool,
    /// Loopback skips the gate (the host's own client; off only in tests).
    pub trust_loopback: bool,
    /// The LAN (private IPv4 addresses) skips the gate.
    pub trust_lan: bool,
}

/// For the lobby screen.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum HostStatus {
    #[default]
    Connecting,
    Online {
        code: Code,
    },
    /// Trying again by itself.
    Unreachable(String),
}

struct Registration {
    host_id: HostId,
    code: Code,
    secret: [u8; SECRET_LEN],
}

struct Intro {
    client_ids: [u64; 3],
    joiner: Option<SocketAddr>,
    joiner_lan: Option<SocketAddr>,
    relay: Option<u64>,
    ban_tag: [u8; TAG_LEN],
    same_network: bool,
    received: Duration,
    /// The one address of the joiner's IP this introduction lets through, once seen (its NAT
    /// may use another port toward us than toward the service).
    bound: Option<SocketAddr>,
}

impl Intro {
    /// Whether `addr` is a way this joiner may reach us: its relay session, its LAN address,
    /// or one address of its public IP (the first seen; not every port of it: netcode keeps
    /// state for each address that asks to connect, so one introduction opens one address).
    fn matches(&self, addr: SocketAddr) -> bool {
        relay_session(addr).is_some_and(|s| Some(s) == self.relay)
            || self.joiner_lan == Some(addr)
            || match self.bound {
                Some(bound) => bound == addr,
                None => self.joiner.is_some_and(|j| j.ip() == addr.ip()),
            }
    }
}

struct Admitted {
    ban_tag: [u8; TAG_LEN],
    same_network: bool,
    addr: SocketAddr,
}

struct Punch {
    to: SocketAddr,
    join_id: u64,
    left: u8,
    next: Duration,
}

pub struct HostAgent {
    config: HostConfig,
    resolver: Resolver,
    resolve_failed_at: Option<Duration>,
    service: Option<SocketAddr>,
    status: HostStatus,
    cookie: Option<([u8; 16], Duration)>,
    hello: Option<Pending>,
    registration: Option<Registration>,
    /// The service has our listing (else: register, resuming `registration` if we have one).
    registered: bool,
    /// The service has answered one of our requests (nonce-checked) at least once.
    verified: bool,
    register: Option<Pending>,
    heartbeat: Option<Pending>,
    last_heartbeat: Option<Duration>,
    last_reply: Option<Duration>,
    backoff_until: Duration,
    info_changed: bool,
    intros: Vec<Intro>,
    seen_joins: VecDeque<u64>,
    acks: Vec<u64>,
    admitted: HashMap<u64, Admitted>,
    relays: HashMap<u64, RelayGrant>,
    rebinds: HashMap<u64, Pending>,
    punches: Vec<Punch>,
    bans: Vec<[u8; TAG_LEN]>,
    /// Bans of joiners behind our own public IP: kept here only (at the service they would
    /// ban our own network) and not applied to the LAN way in.
    local_bans: Vec<[u8; TAG_LEN]>,
    ban_requests: Vec<([u8; TAG_LEN], Pending)>,
    banned_ips: HashSet<IpAddr>,
    started: Option<Duration>,
}

impl HostAgent {
    pub fn new(config: HostConfig) -> Self {
        Self {
            resolver: Resolver::start(&config.service),
            config,
            resolve_failed_at: None,
            service: None,
            status: HostStatus::Connecting,
            cookie: None,
            hello: None,
            registration: None,
            registered: false,
            verified: false,
            register: None,
            heartbeat: None,
            last_heartbeat: None,
            last_reply: None,
            backoff_until: Duration::ZERO,
            info_changed: false,
            intros: Vec::new(),
            seen_joins: VecDeque::new(),
            acks: Vec::new(),
            admitted: HashMap::new(),
            relays: HashMap::new(),
            rebinds: HashMap::new(),
            punches: Vec::new(),
            bans: Vec::new(),
            local_bans: Vec::new(),
            ban_requests: Vec::new(),
            banned_ips: HashSet::new(),
            started: None,
        }
    }

    pub fn status(&self) -> &HostStatus {
        &self.status
    }

    pub fn service_addr(&self) -> Option<SocketAddr> {
        self.service
    }

    /// Updates what the browser shows (sent with the next heartbeat).
    pub fn set_info(&mut self, info: HostInfo) {
        if info != self.config.info {
            self.config.info = info;
            self.info_changed = true;
        }
    }

    /// Loopback, and with `trust_lan` our own LAN: the /24 of our LAN address (not every
    /// private address: carrier NATs and VPNs use them too).
    fn trusted(&self, addr: SocketAddr) -> bool {
        let ip = canonical(addr).ip();
        let same_lan = match (ip, self.config.lan.map(|l| l.ip())) {
            (IpAddr::V4(ip), Some(IpAddr::V4(ours))) => {
                let ([a, b, c, _], [x, y, z, _]) = (ip.octets(), ours.octets());
                ip.is_private() && (a, b, c) == (x, y, z)
            }
            _ => false,
        };
        (self.config.trust_loopback && ip.is_loopback()) || (self.config.trust_lan && same_lan)
    }

    /// Sorts a received datagram: ours (handled here), relayed netcode, or netcode, which the
    /// gate may refuse.
    pub fn on_receive<'a>(
        &mut self,
        now: Duration,
        from: SocketAddr,
        data: &'a [u8],
    ) -> Inbound<'a> {
        let from = canonical(from);
        if Some(from) == self.service {
            return self.service_datagram(now, data);
        }
        if proto::is_rendezvous(data) {
            // A joiner's punch (it only had to get through the NATs) or junk.
            return Inbound::Drop;
        }
        if self.banned_ips.contains(&from.ip()) && !self.trusted(from) {
            return Inbound::Drop;
        }
        if !self.config.gate {
            return Inbound::Netcode(from);
        }
        let known = self.trusted(from)
            || self.admitted.values().any(|a| a.addr == from)
            || self.intros.iter_mut().any(|i| {
                let ok = i.matches(from);
                if ok && relay_session(from).is_none() && i.joiner_lan != Some(from) {
                    i.bound = Some(from);
                }
                ok
            });
        if known {
            Inbound::Netcode(from)
        } else {
            Inbound::Drop
        }
    }

    fn service_datagram<'a>(&mut self, now: Duration, data: &'a [u8]) -> Inbound<'a> {
        let Ok(msg) = proto::decode(data) else {
            return Inbound::Drop;
        };
        match msg {
            Message::Relay { session, payload } => {
                if self.relays.contains_key(&session) {
                    return Inbound::Relayed {
                        from: relay_addr(session),
                        payload,
                    };
                }
            }
            Message::Introduce { intro, mac } => self.introduced(now, data, intro, mac),
            msg => self.reply(now, msg),
        }
        Inbound::Drop
    }

    fn reply(&mut self, now: Duration, msg: Message) {
        let nonce = msg.nonce();
        let is = |p: &Option<Pending>| p.is_some_and(|p| p.nonce == nonce && nonce != 0);
        match msg {
            Message::Cookie { cookie, .. } if is(&self.hello) => {
                self.hello = None;
                self.cookie = Some((cookie, now));
                self.last_reply = Some(now);
                self.verified = true;
            }
            Message::Registered {
                host_id,
                code,
                secret,
                ..
            } if is(&self.register) => {
                self.register = None;
                self.registration = Some(Registration {
                    host_id,
                    code,
                    secret,
                });
                self.registered = true;
                self.status = HostStatus::Online { code };
                self.last_reply = Some(now);
                self.last_heartbeat = Some(now);
            }
            Message::Ok { .. } if is(&self.heartbeat) => {
                self.heartbeat = None;
                self.last_reply = Some(now);
                if let Some(r) = &self.registration {
                    self.status = HostStatus::Online { code: r.code };
                }
            }
            Message::Ok { nonce } => {
                self.ban_requests.retain(|(_, p)| p.nonce != nonce);
            }
            Message::RelayBound { session, .. } => {
                if self.rebinds.get(&session).is_some_and(|p| p.nonce == nonce) {
                    self.rebinds.remove(&session);
                }
            }
            Message::RelayUnknown { session } => {
                if self.relays.contains_key(&session) && !self.rebinds.contains_key(&session) {
                    self.rebinds.insert(session, Pending::new());
                }
            }
            Message::Error { code, .. } => self.error(now, nonce, code),
            _ => {}
        }
    }

    fn error(&mut self, now: Duration, nonce: u64, code: ErrorCode) {
        let ours = |p: &Option<Pending>| p.is_some_and(|p| p.nonce == nonce);
        let any_request = ours(&self.register)
            || ours(&self.heartbeat)
            || self.ban_requests.iter().any(|(_, p)| p.nonce == nonce)
            || self.rebinds.values().any(|p| p.nonce == nonce);
        // UnsupportedVersion answers whatever we sent, with nonce 0 (the service can't read
        // another version's nonce), so anyone could forge it: only believed before the service
        // has ever answered us properly.
        let unsupported = code == ErrorCode::UnsupportedVersion && nonce == 0 && !self.verified;
        if !any_request && !unsupported {
            return;
        }
        if code == ErrorCode::UnsupportedVersion && !unsupported {
            return;
        }
        self.last_reply = Some(now);
        match code {
            ErrorCode::BadCookie => self.cookie = None,
            ErrorCode::UnknownHost => {
                // The service forgot us (restart, or our heartbeats got lost): register again,
                // keeping the code. It lost the relay sessions too: bind ours again now
                // rather than when one of them next carries a packet.
                self.heartbeat = None;
                self.register = None;
                self.registered = false;
                for session in self.relays.keys() {
                    self.rebinds.entry(*session).or_insert_with(Pending::new);
                }
            }
            ErrorCode::Refused => {
                if ours(&self.register) || ours(&self.heartbeat) {
                    // Our listing is someone else's now (or the key changed): start over.
                    self.registration = None;
                    self.registered = false;
                    self.register = None;
                    self.heartbeat = None;
                    self.status = HostStatus::Connecting;
                }
                self.rebinds.retain(|_, p| p.nonce != nonce);
                self.ban_requests.retain(|(_, p)| p.nonce != nonce);
            }
            ErrorCode::Busy | ErrorCode::RateLimited => {
                self.backoff_until = now + BACKOFF;
                if ours(&self.register) {
                    self.register = None;
                    self.status = HostStatus::Unreachable(if code == ErrorCode::Busy {
                        "the online service is full right now; retrying".into()
                    } else {
                        "the online service is rate limiting us; retrying".into()
                    });
                }
            }
            ErrorCode::UnsupportedVersion => {
                self.backoff_until = now + RESOLVE_RETRY;
                self.status = HostStatus::Unreachable(
                    "the online service needs a newer version of the game".into(),
                );
            }
        }
    }

    fn introduced(&mut self, now: Duration, data: &[u8], intro: Introduction, mac: [u8; 16]) {
        let Some(reg) = &self.registration else {
            return;
        };
        let authentic = intro.host_id == reg.host_id
            && introduce_signed_part(data)
                .is_some_and(|body| ct_eq(&mac, &message_mac(&reg.secret, body)));
        if !authentic {
            return;
        }
        // Acknowledge every copy (the first ack may have been lost), act on the first.
        if !self.acks.contains(&intro.join_id) {
            self.acks.push(intro.join_id);
        }
        if self.seen_joins.contains(&intro.join_id) {
            return;
        }
        if self.seen_joins.len() >= SEEN_JOINS {
            self.seen_joins.pop_front();
        }
        self.seen_joins.push_back(intro.join_id);
        if self.intros.len() >= MAX_INTROS {
            // Make room at the expense of whoever has the most joins in flight (a flood from
            // one address can't push out everyone else's).
            let mut counts: HashMap<[u8; TAG_LEN], usize> = HashMap::new();
            for i in &self.intros {
                *counts.entry(i.ban_tag).or_default() += 1;
            }
            let busiest = counts
                .into_iter()
                .max_by_key(|(_, n)| *n)
                .map(|(tag, _)| tag);
            if let Some(at) = self.intros.iter().position(|i| Some(i.ban_tag) == busiest) {
                self.intros.remove(at);
            }
        }
        let same_joiner = self
            .intros
            .iter()
            .filter(|i| i.ban_tag == intro.ban_tag)
            .count();
        if same_joiner >= MAX_INTROS_PER_JOINER
            && let Some(at) = self.intros.iter().position(|i| i.ban_tag == intro.ban_tag)
        {
            self.intros.remove(at);
        }
        for to in [intro.joiner, intro.joiner_lan].into_iter().flatten() {
            self.punches.push(Punch {
                to,
                join_id: intro.join_id,
                left: PUNCHES,
                next: now,
            });
        }
        if let Some(grant) = intro.relay {
            self.relays.insert(grant.session, grant);
        }
        self.intros.push(Intro {
            client_ids: intro.client_ids,
            joiner: intro.joiner,
            joiner_lan: intro.joiner_lan,
            relay: intro.relay.map(|g| g.session),
            ban_tag: intro.ban_tag,
            same_network: intro.same_network,
            received: now,
            bound: None,
        });
    }

    /// A new netcode connection (`client_id` from `addr`) is about to be accepted: may it?
    /// Uses up the joiner's introduction.
    pub fn admit(&mut self, client_id: u64, addr: SocketAddr) -> bool {
        let addr = canonical(addr);
        let found = self
            .intros
            .iter()
            .position(|i| i.client_ids.contains(&client_id) && i.matches(addr));
        if let Some(i) = found {
            let intro = self.intros.remove(i);
            let via_lan = intro.joiner_lan == Some(addr);
            if self.bans.contains(&intro.ban_tag)
                || (!via_lan && self.local_bans.contains(&intro.ban_tag))
            {
                return false;
            }
            self.admitted.insert(
                client_id,
                Admitted {
                    ban_tag: intro.ban_tag,
                    same_network: intro.same_network,
                    addr,
                },
            );
            return true;
        }
        if self.banned_ips.contains(&addr.ip()) && !self.trusted(addr) {
            return false;
        }
        !self.config.gate || self.trusted(addr)
    }

    /// The connection is gone.
    pub fn disconnected(&mut self, client_id: u64) {
        self.admitted.remove(&client_id);
    }

    /// Bans the joiner behind connection `client_id` from this game: the service refuses its
    /// joins, and the gate its address.
    pub fn ban(&mut self, client_id: u64) {
        let Some(a) = self.admitted.get(&client_id) else {
            return;
        };
        if relay_session(a.addr).is_none()
            && !self.trusted(a.addr)
            && self.banned_ips.len() < MAX_BANNED_IPS
        {
            self.banned_ips.insert(a.addr.ip());
        }
        let tag = a.ban_tag;
        // Its other joins in flight (another attempt, the relay) are void too.
        self.intros.retain(|i| i.ban_tag != tag);
        // Behind our own public IP the tag would ban our own network at the service: it is
        // kept here, for the ways in other than the LAN (a player on our own LAN can always
        // come back over it: the LAN skips the gate).
        if a.same_network {
            if relay_session(a.addr).is_none() && !self.trusted(a.addr) {
                self.banned_ips.insert(a.addr.ip());
            }
            if !self.local_bans.contains(&tag) {
                if self.local_bans.len() >= MAX_BANNED_IPS {
                    self.local_bans.remove(0);
                }
                self.local_bans.push(tag);
            }
            return;
        }
        if !self.bans.contains(&tag) {
            if self.bans.len() >= MAX_BANS {
                self.bans.remove(0);
            }
            self.bans.push(tag);
            self.ban_requests.push((tag, Pending::new()));
        }
    }

    /// How to send a netcode packet to `to`.
    pub fn outbound(&self, to: SocketAddr, payload: &[u8]) -> Outbound {
        let Some(session) = relay_session(to) else {
            return Outbound::Direct;
        };
        match self.service {
            Some(service) if self.relays.contains_key(&session) => {
                Outbound::Relay(service, encode_relay(session, payload))
            }
            _ => Outbound::Drop,
        }
    }

    /// Timers: the listing, acknowledgements, punches, relay rebinds.
    pub fn poll(&mut self, now: Duration, out: &mut Vec<(SocketAddr, Vec<u8>)>) {
        let started = *self.started.get_or_insert(now);
        if self.service.is_none() {
            self.resolve(now);
        }
        let Some(service) = self.service else {
            return;
        };
        self.punch(now, out);
        self.intros
            .retain(|i| now.saturating_sub(i.received) < INTRO_TTL);
        // Relay grants stay while an introduction or a connection uses them.
        let (intros, admitted) = (&self.intros, &self.admitted);
        self.relays.retain(|session, _| {
            intros.iter().any(|i| i.relay == Some(*session))
                || admitted
                    .values()
                    .any(|a| relay_session(a.addr) == Some(*session))
        });
        let relays = &self.relays;
        self.rebinds
            .retain(|session, _| relays.contains_key(session));

        let silent_since = self.last_reply.unwrap_or(started);
        if now.saturating_sub(silent_since) > OFFLINE_AFTER
            && !matches!(self.status, HostStatus::Unreachable(_))
        {
            self.status = HostStatus::Unreachable("no answer from the online service".into());
            // Start over with a fresh cookie (our address may have changed).
            self.cookie = None;
        }
        if now < self.backoff_until {
            return;
        }
        let cookie = match self.cookie {
            Some((cookie, at)) if now.saturating_sub(at) < COOKIE_REFRESH => cookie,
            _ => {
                let hello = self.hello.get_or_insert_with(Pending::new);
                if hello.send_now(now, HELLO_RETRY) {
                    out.push((service, encode(&Message::Hello { nonce: hello.nonce })));
                }
                // An old cookie still works while we wait for the new one.
                match self.cookie {
                    Some((cookie, _)) => cookie,
                    None => return,
                }
            }
        };

        for join_id in self.acks.drain(..) {
            if let Some(reg) = &self.registration {
                out.push((
                    service,
                    encode(&Message::IntroAck {
                        cookie,
                        host_id: reg.host_id,
                        secret: reg.secret,
                        join_id,
                    }),
                ));
            }
        }

        match &self.registration {
            Some(reg) if self.registered => {
                let due = self
                    .last_heartbeat
                    .is_none_or(|t| now.saturating_sub(t) >= HEARTBEAT)
                    || (self.info_changed
                        && self
                            .last_heartbeat
                            .is_none_or(|t| now.saturating_sub(t) >= HEARTBEAT_MIN_GAP));
                if due {
                    // One nonce per heartbeat; a new one replaces an unanswered one.
                    let heartbeat = Pending::new();
                    self.heartbeat = Some(heartbeat);
                    self.last_heartbeat = Some(now);
                    self.info_changed = false;
                    out.push((
                        service,
                        encode(&Message::Heartbeat {
                            cookie,
                            nonce: heartbeat.nonce,
                            host_id: reg.host_id,
                            secret: reg.secret,
                            info: self.config.info.clone(),
                        }),
                    ));
                }
            }
            _ => {
                let register = self.register.get_or_insert_with(Pending::new);
                if register.send_now(now, REGISTER_RETRY) {
                    let resume = self.registration.as_ref().map(|r| Resume {
                        host_id: r.host_id,
                        code: r.code,
                        secret: r.secret,
                    });
                    self.info_changed = false;
                    out.push((
                        service,
                        encode(&Message::Register {
                            cookie,
                            nonce: register.nonce,
                            resume,
                            lan: self.config.lan,
                            info: self.config.info.clone(),
                            bans: self.bans.clone(),
                        }),
                    ));
                }
            }
        }

        if let Some(reg) = &self.registration {
            for (tag, pending) in &mut self.ban_requests {
                if pending.send_now(now, BAN_RETRY) {
                    out.push((
                        service,
                        encode(&Message::Ban {
                            cookie,
                            nonce: pending.nonce,
                            host_id: reg.host_id,
                            secret: reg.secret,
                            tag: *tag,
                        }),
                    ));
                }
            }
            for (session, pending) in &mut self.rebinds {
                let Some(grant) = self.relays.get(session) else {
                    continue;
                };
                if pending.send_now(now, REBIND_RETRY) {
                    out.push((
                        service,
                        encode(&Message::RelayBind {
                            cookie,
                            nonce: pending.nonce,
                            session: *session,
                            role: Role::Host,
                            grant_expires: grant.expires,
                            host_id: grant.host_id,
                            cred: grant.cred,
                        }),
                    ));
                }
            }
        }
    }

    fn resolve(&mut self, now: Duration) {
        if let Some(failed) = self.resolve_failed_at {
            if now.saturating_sub(failed) < RESOLVE_RETRY {
                return;
            }
            self.resolve_failed_at = None;
            self.resolver = Resolver::start(&self.config.service);
        }
        match self.resolver.poll() {
            Some(Ok(addr)) => self.service = Some(canonical(*addr)),
            Some(Err(e)) => {
                self.status = HostStatus::Unreachable(e.clone());
                self.resolve_failed_at = Some(now);
            }
            None => {}
        }
    }

    fn punch(&mut self, now: Duration, out: &mut Vec<(SocketAddr, Vec<u8>)>) {
        for p in &mut self.punches {
            if p.left > 0 && now >= p.next {
                out.push((p.to, encode(&Message::Punch { join_id: p.join_id })));
                p.left -= 1;
                p.next = now + PUNCH_EVERY;
            }
        }
        self.punches.retain(|p| p.left > 0);
    }

    /// Leaving: takes the listing down (one datagram, best effort).
    pub fn shutdown(&mut self, out: &mut Vec<(SocketAddr, Vec<u8>)>) {
        if let (Some(service), Some(reg), Some((cookie, _))) =
            (self.service, &self.registration, self.cookie)
        {
            out.push((
                service,
                encode(&Message::Unregister {
                    cookie,
                    nonce: crate::crypto::random_nonzero_u64(),
                    host_id: reg.host_id,
                    secret: reg.secret,
                }),
            ));
        }
        self.registration = None;
        self.registered = false;
    }
}
