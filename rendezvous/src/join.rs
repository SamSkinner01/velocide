//! A joiner's side of the service, without IO: ask the service for the game behind a code,
//! then try the ways to the host in turn (its LAN address if we share its public IP, its
//! public address with hole punching, the relay) until netcode connects on one.
//!
//! The client transport owns the socket and one netcode client per attempt: it builds the
//! netcode client for [`JoinAgent::attempt`] (a new one whenever that changes), passes
//! datagrams through [`JoinAgent::on_receive`] and netcode's packets through
//! [`JoinAgent::outbound`], and reports [`JoinAgent::connected`] or
//! [`JoinAgent::attempt_failed`].

use std::net::SocketAddr;
use std::time::Duration;

use crate::code::Code;
use crate::proto::{self, ErrorCode, JoinFailure, Message, RelayGrant, Role, encode, encode_relay};
use crate::service::is_lan_candidate;
use crate::wire::canonical;
use crate::{Inbound, Outbound, Pending, Resolver, relay_addr, relay_session};

const HELLO_RETRY: Duration = Duration::from_millis(400);
const JOIN_RETRY: Duration = Duration::from_millis(600);
const COOKIE_REFRESH: Duration = Duration::from_secs(45);
/// No answer from the service for this long: give up.
const SERVICE_TIMEOUT: Duration = Duration::from_secs(8);
const PUNCH_EVERY: Duration = Duration::from_millis(100);
const REBIND_RETRY: Duration = Duration::from_millis(500);

/// How long each way gets to connect.
const LAN_TIME: Duration = Duration::from_millis(1500);
const DIRECT_TIME: Duration = Duration::from_secs(3);
const RELAY_TIME: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct JoinConfig {
    pub service: String,
    pub code: Code,
    /// The game's `PROTOCOL_VERSION`.
    pub protocol: u32,
    /// Netcode client ids for the LAN, direct and relayed attempts (random, distinct).
    pub client_ids: [u64; 3],
    /// Our LAN address (the game socket's port on our LAN IP).
    pub lan: Option<SocketAddr>,
    /// Hide our address from the host: go through the relay only.
    pub relay_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Lan,
    Direct,
    Relay,
}

/// One way to the host: netcode connects to `addr` (a [`relay_addr`] for the relay) with
/// `client_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub kind: PathKind,
    pub addr: SocketAddr,
    pub client_id: u64,
}

impl Attempt {
    fn time(&self) -> Duration {
        match self.kind {
            PathKind::Lan => LAN_TIME,
            PathKind::Direct => DIRECT_TIME,
            PathKind::Relay => RELAY_TIME,
        }
    }
}

/// Where the join is at (for the connecting screen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinStep {
    Contacting,
    Joining,
    Trying(PathKind),
    Connected(PathKind),
    Failed(String),
}

pub struct JoinAgent {
    config: JoinConfig,
    resolver: Resolver,
    service: Option<SocketAddr>,
    started: Option<Duration>,
    cookie: Option<([u8; 16], Duration)>,
    hello: Option<Pending>,
    join: Option<Pending>,
    join_id: u64,
    relay: Option<RelayGrant>,
    /// The ways still to try, then the one being tried.
    plan: Vec<Attempt>,
    current: Option<(Attempt, Duration)>,
    tried: Vec<PathKind>,
    next_punch: Duration,
    connected: bool,
    failed: Option<String>,
    rebind: Option<Pending>,
    relay_busy: bool,
}

impl JoinAgent {
    pub fn new(config: JoinConfig) -> Self {
        Self {
            resolver: Resolver::start(&config.service),
            config,
            service: None,
            started: None,
            cookie: None,
            hello: None,
            join: None,
            join_id: 0,
            relay: None,
            plan: Vec::new(),
            current: None,
            tried: Vec::new(),
            next_punch: Duration::ZERO,
            connected: false,
            failed: None,
            rebind: None,
            relay_busy: false,
        }
    }

    pub fn step(&self) -> JoinStep {
        if let Some(e) = &self.failed {
            return JoinStep::Failed(e.clone());
        }
        match self.current {
            Some((a, _)) if self.connected => JoinStep::Connected(a.kind),
            Some((a, _)) => JoinStep::Trying(a.kind),
            None if self.cookie.is_some() => JoinStep::Joining,
            None => JoinStep::Contacting,
        }
    }

    /// The attempt netcode should be making now.
    pub fn attempt(&self) -> Option<Attempt> {
        self.current.map(|(a, _)| a)
    }

    /// Why the join failed, if it did.
    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Netcode connected on the current attempt: stop trying others.
    pub fn connected(&mut self) {
        self.connected = true;
        self.plan.clear();
    }

    /// Netcode gave up on the current attempt (refused, timed out): try the next way now.
    pub fn attempt_failed(&mut self, now: Duration) {
        if !self.connected {
            self.next_attempt(now);
        }
    }

    fn fail(&mut self, reason: String) {
        // Once connected, nothing the service (or someone forging it) says ends the game.
        if self.connected {
            return;
        }
        if self.failed.is_none() {
            self.failed = Some(reason);
        }
        self.current = None;
        self.plan.clear();
    }

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
            return Inbound::Drop; // the host's punches
        }
        match self.current {
            Some((a, _)) if a.addr == from && a.kind != PathKind::Relay => Inbound::Netcode(from),
            _ => Inbound::Drop,
        }
    }

    fn service_datagram<'a>(&mut self, now: Duration, data: &'a [u8]) -> Inbound<'a> {
        let Ok(msg) = proto::decode(data) else {
            return Inbound::Drop;
        };
        let nonce = msg.nonce();
        let is = |p: &Option<Pending>| p.is_some_and(|p| p.nonce == nonce && nonce != 0);
        match msg {
            Message::Relay { session, payload } => {
                let ours = self.relay.is_some_and(|g| g.session == session)
                    && self.current.is_some_and(|(a, _)| a.kind == PathKind::Relay);
                if ours {
                    return Inbound::Relayed {
                        from: relay_addr(session),
                        payload,
                    };
                }
            }
            Message::Cookie { cookie, .. } if is(&self.hello) => {
                self.hello = None;
                self.cookie = Some((cookie, now));
            }
            Message::JoinOk {
                join_id,
                host,
                host_lan,
                relay,
                ..
            } if is(&self.join) => {
                self.join = None;
                self.join_id = join_id;
                self.relay = relay;
                let ids = self.config.client_ids;
                let relay_only = self.config.relay_only;
                let mut plan = Vec::new();
                if let Some(addr) = host_lan.filter(is_lan_candidate) {
                    plan.push(Attempt {
                        kind: PathKind::Lan,
                        addr,
                        client_id: ids[0],
                    });
                }
                if let Some(addr) = host.filter(|_| !relay_only) {
                    plan.push(Attempt {
                        kind: PathKind::Direct,
                        addr: canonical(addr),
                        client_id: ids[1],
                    });
                }
                if let Some(g) = relay {
                    plan.push(Attempt {
                        kind: PathKind::Relay,
                        addr: relay_addr(g.session),
                        client_id: ids[2],
                    });
                }
                if plan.is_empty() {
                    self.fail("the online service found the game but no way to reach it".into());
                } else {
                    plan.reverse(); // popped from the back
                    self.plan = plan;
                    self.next_attempt(now);
                }
            }
            Message::JoinFailed {
                reason, host_build, ..
            } if is(&self.join) => {
                let code = self.config.code;
                self.fail(match reason {
                    JoinFailure::NotFound => format!(
                        "no game with code {code} (check the code; the host may have closed it)"
                    ),
                    JoinFailure::VersionMismatch => format!(
                        "that game runs another version ({}): everyone needs the same one",
                        if host_build.is_empty() {
                            "unknown"
                        } else {
                            &host_build
                        }
                    ),
                    JoinFailure::Banned => "the host doesn't let you join this game".into(),
                    JoinFailure::Busy => "the online service is busy; try again in a minute".into(),
                    JoinFailure::RateLimited => {
                        "too many join attempts; wait a few seconds and try again".into()
                    }
                });
            }
            Message::RelayUnknown { session } => {
                if self.relay.is_some_and(|g| g.session == session) && self.rebind.is_none() {
                    self.rebind = Some(Pending::new());
                }
            }
            Message::RelayBound { session, .. } if is(&self.rebind) => {
                if self.relay.is_some_and(|g| g.session == session) {
                    self.rebind = None;
                }
            }
            Message::Error { code, .. } => {
                let ours = is(&self.join) || is(&self.rebind) || nonce == 0;
                match code {
                    ErrorCode::BadCookie if ours => self.cookie = None,
                    // Sent with nonce 0, so forgeable: believed only before the service has
                    // answered us properly (a cookie).
                    ErrorCode::UnsupportedVersion if self.cookie.is_none() => self.fail(
                        "the online service needs a newer version of the game: update it".into(),
                    ),
                    ErrorCode::RateLimited if is(&self.join) => {
                        self.fail("too many join attempts; wait a few seconds and try again".into())
                    }
                    ErrorCode::Busy if is(&self.join) => {
                        self.fail("the online service is busy; try again in a minute".into())
                    }
                    ErrorCode::Refused if is(&self.rebind) => self.rebind = None,
                    ErrorCode::Busy if is(&self.rebind) => {
                        // No room on the relay: this way is out.
                        self.rebind = None;
                        self.relay_busy = true;
                        if self.current.is_some_and(|(a, _)| a.kind == PathKind::Relay)
                            && !self.connected
                        {
                            self.next_attempt(now);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Inbound::Drop
    }

    fn next_attempt(&mut self, now: Duration) {
        if let Some((a, _)) = self.current.take() {
            self.tried.push(a.kind);
        }
        match self.plan.pop() {
            Some(a) => {
                self.current = Some((a, now));
                self.next_punch = now;
                if a.kind == PathKind::Relay {
                    // The relay session opens when we bind it.
                    self.rebind.get_or_insert_with(Pending::new);
                }
            }
            None => {
                let tried = &self.tried;
                let ways = match (
                    tried.contains(&PathKind::Direct) || tried.contains(&PathKind::Lan),
                    tried.contains(&PathKind::Relay),
                ) {
                    (true, true) => "directly or through the relay",
                    (true, false) => "directly (the relay is unavailable)",
                    (false, true) => "through the relay",
                    (false, false) => "",
                };
                self.fail(if self.relay_busy {
                    format!("couldn't reach the host {ways} (the relay is busy; try again soon)")
                } else {
                    format!("couldn't reach the host {ways}")
                });
            }
        }
    }

    /// How to send a netcode packet to `to`.
    pub fn outbound(&self, to: SocketAddr, payload: &[u8]) -> Outbound {
        let Some(session) = relay_session(to) else {
            return Outbound::Direct;
        };
        match (self.service, self.relay) {
            (Some(service), Some(g)) if g.session == session => {
                Outbound::Relay(service, encode_relay(session, payload))
            }
            _ => Outbound::Drop,
        }
    }

    /// Timers: the requests to the service, punches, attempt deadlines, relay rebinds.
    pub fn poll(&mut self, now: Duration, out: &mut Vec<(SocketAddr, Vec<u8>)>) {
        let started = *self.started.get_or_insert(now);
        if self.failed.is_some() {
            return;
        }
        if self.service.is_none() {
            match self.resolver.poll() {
                Some(Ok(addr)) => self.service = Some(canonical(*addr)),
                Some(Err(e)) => {
                    let e = e.clone();
                    self.fail(e);
                    return;
                }
                None => return,
            }
        }
        let Some(service) = self.service else {
            return;
        };
        let waiting = self.current.is_none() && !self.connected;
        if waiting && now.saturating_sub(started) > SERVICE_TIMEOUT {
            self.fail(format!(
                "no answer from the online service ({})",
                self.config.service
            ));
            return;
        }

        // A cookie: needed before joining, and for rebinding the relay later.
        let needs_cookie = waiting || self.rebind.is_some();
        let fresh = self
            .cookie
            .is_some_and(|(_, at)| now.saturating_sub(at) < COOKIE_REFRESH);
        if needs_cookie && !fresh {
            let hello = self.hello.get_or_insert_with(Pending::new);
            if hello.send_now(now, HELLO_RETRY) {
                out.push((service, encode(&Message::Hello { nonce: hello.nonce })));
            }
        }
        let Some((cookie, _)) = self.cookie else {
            return;
        };

        if waiting && self.join_id == 0 {
            let join = self.join.get_or_insert_with(Pending::new);
            if join.send_now(now, JOIN_RETRY) {
                out.push((
                    service,
                    encode(&Message::Join {
                        cookie,
                        nonce: join.nonce,
                        code: self.config.code,
                        protocol: self.config.protocol,
                        client_ids: self.config.client_ids,
                        lan: self.config.lan.filter(is_lan_candidate),
                        relay_only: self.config.relay_only,
                    }),
                ));
            }
        }

        if let Some((attempt, since)) = self.current
            && !self.connected
        {
            if now.saturating_sub(since) >= attempt.time() {
                self.next_attempt(now);
            } else if attempt.kind != PathKind::Relay && now >= self.next_punch {
                out.push((
                    attempt.addr,
                    encode(&Message::Punch {
                        join_id: self.join_id,
                    }),
                ));
                self.next_punch = now + PUNCH_EVERY;
            }
        }

        if let (Some(rebind), Some(grant)) = (&mut self.rebind, self.relay)
            && rebind.send_now(now, REBIND_RETRY)
        {
            out.push((
                service,
                encode(&Message::RelayBind {
                    cookie,
                    nonce: rebind.nonce,
                    session: grant.session,
                    role: Role::Joiner,
                    grant_expires: grant.expires,
                    host_id: grant.host_id,
                    cred: grant.cred,
                }),
            ));
        }
    }
}
