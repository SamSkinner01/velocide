//! The rendezvous wire protocol (version 1). docs/rendezvous.md "Protocol" is the reference.
//!
//! Every datagram starts with the 4-byte [`MAGIC`] (its first byte, `0xFF`, is never the
//! first byte of a netcode packet, so the game tells the two apart on its one UDP socket), the
//! protocol [`VERSION`] and a message kind. All integers are little-endian. Decoding is
//! strict: a wrong length, an unknown flag, an out-of-range value or a trailing byte make the
//! whole datagram [`DecodeError::Malformed`].

use std::net::SocketAddr;

use crate::code::{CODE_LEN, Code};
use crate::crypto::{
    COOKIE_LEN, CRED_LEN, HOST_ID_LEN, HostId, MAC_LEN, SECRET_LEN, TAG_LEN, message_mac,
};
use crate::wire::{Malformed, Reader, Writer};

pub const MAGIC: [u8; 4] = [0xFF, b'V', b'R', b'Z'];
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 6;
/// Largest control message (fits any path MTU we care about).
pub const MAX_CONTROL_LEN: usize = 1200;
/// Largest relayed payload: netcode's largest packet.
pub const MAX_RELAY_PAYLOAD: usize = 1400;
/// Largest datagram of the protocol (a relayed packet).
pub const MAX_DATAGRAM: usize = HEADER_LEN + 8 + MAX_RELAY_PAYLOAD;
/// `Hello` and `Probe` are padded to this size: the answer to a packet whose sender isn't
/// verified must never be bigger than the packet (no amplification).
pub const PADDED_LEN: usize = 64;

pub const NAME_MAX: usize = 32;
pub const LEVEL_MAX: usize = 32;
pub const BUILD_MAX: usize = 32;
pub const MAX_PLAYERS: u8 = 64;
/// Ban tags a `Register` may carry.
pub const MAX_BANS: usize = 16;
/// Entries per `ListPage` (the largest page is 1115 bytes).
pub const LIST_PAGE_LEN: usize = 14;

/// Cookies are valid in the window they were made in and the next one.
pub const COOKIE_WINDOW_SECS: u64 = 60;

pub fn cookie_window(unix_secs: u64) -> u64 {
    unix_secs / COOKIE_WINDOW_SECS
}

mod kind {
    pub const HELLO: u8 = 0x01;
    pub const COOKIE: u8 = 0x02;
    pub const PROBE: u8 = 0x03;
    pub const PROBE_REPLY: u8 = 0x04;
    pub const ERROR: u8 = 0x05;
    pub const OK: u8 = 0x06;
    pub const REGISTER: u8 = 0x10;
    pub const REGISTERED: u8 = 0x11;
    pub const HEARTBEAT: u8 = 0x12;
    pub const UNREGISTER: u8 = 0x13;
    pub const INTRODUCE: u8 = 0x14;
    pub const INTRO_ACK: u8 = 0x15;
    pub const BAN: u8 = 0x16;
    pub const LIST: u8 = 0x20;
    pub const LIST_PAGE: u8 = 0x21;
    pub const JOIN: u8 = 0x22;
    pub const JOIN_OK: u8 = 0x23;
    pub const JOIN_FAILED: u8 = 0x24;
    pub const RELAY: u8 = 0x30;
    pub const RELAY_BIND: u8 = 0x31;
    pub const RELAY_BOUND: u8 = 0x32;
    pub const RELAY_UNKNOWN: u8 = 0x33;
    pub const PUNCH: u8 = 0x40;
}

const FLAG_PUBLIC: u8 = 1;
const FLAG_RELAY_ONLY: u8 = 2;
const FLAG_IN_MATCH: u8 = 4;

/// Whether a datagram belongs to this protocol (rather than to netcode).
pub fn is_rendezvous(data: &[u8]) -> bool {
    data.starts_with(&MAGIC)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not a rendezvous datagram (no magic).
    NotOurs,
    /// A rendezvous datagram of another protocol version.
    Version(u8),
    Malformed,
}

impl From<Malformed> for DecodeError {
    fn from(_: Malformed) -> Self {
        DecodeError::Malformed
    }
}

/// What a host tells the service about its game (shown in the server browser).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    /// Printable ASCII, at most [`NAME_MAX`] bytes.
    pub name: String,
    /// `shared::lobby::GameMode` index (the service doesn't interpret it).
    pub mode: u8,
    /// Level name: `a-z`, `0-9`, `_` and `-`, at most [`LEVEL_MAX`] bytes.
    pub level: String,
    pub players: u8,
    pub max_players: u8,
    pub in_match: bool,
    /// The game's `PROTOCOL_VERSION`: joiners must have the same.
    pub protocol: u32,
    /// The release the host runs (`build-42`), for "they run another version" messages.
    pub build: String,
    /// Listed in the server browser (otherwise only the code finds it).
    pub public: bool,
    /// Never tell joiners the host's address: everyone goes through the relay.
    pub relay_only: bool,
}

impl Default for HostInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            mode: 0,
            level: String::new(),
            players: 0,
            max_players: 1,
            in_match: false,
            protocol: 0,
            build: String::new(),
            public: false,
            relay_only: false,
        }
    }
}

/// Lets a host get its listing (and code) back after the service lost it (a restart).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resume {
    pub host_id: HostId,
    pub code: Code,
    pub secret: [u8; SECRET_LEN],
}

/// Permission to use one end of a relay session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelayGrant {
    pub session: u64,
    /// Unix seconds.
    pub expires: u64,
    /// The listing the session belongs to (the credential covers it).
    pub host_id: HostId,
    pub cred: [u8; CRED_LEN],
}

/// Which end of a relay session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Host = 0,
    Joiner = 1,
}

impl Role {
    fn from_u8(v: u8) -> Result<Role, Malformed> {
        match v {
            0 => Ok(Role::Host),
            1 => Ok(Role::Joiner),
            _ => Err(Malformed),
        }
    }

    pub fn other(self) -> Role {
        match self {
            Role::Host => Role::Joiner,
            Role::Joiner => Role::Host,
        }
    }
}

/// The service tells a host that someone with its code wants in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Introduction {
    pub host_id: HostId,
    pub join_id: u64,
    /// The netcode client ids the joiner will use for its LAN, direct and relayed attempts:
    /// the host admits only these (docs/rendezvous.md "Admission").
    pub client_ids: [u64; 3],
    /// The joiner's public address (absent when either side wants relay only).
    pub joiner: Option<SocketAddr>,
    /// Its LAN address, when it is behind the same public IP as the host.
    pub joiner_lan: Option<SocketAddr>,
    pub relay: Option<RelayGrant>,
    /// What to ban the joiner by.
    pub ban_tag: [u8; TAG_LEN],
    /// The joiner shares the host's public IP (same household or carrier NAT): its ban tag
    /// would ban the host's own network, so the host doesn't send it.
    pub same_network: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    pub code: Code,
    pub name: String,
    pub mode: u8,
    pub level: String,
    pub players: u8,
    pub max_players: u8,
    pub in_match: bool,
    pub relay_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// Get a new cookie (`Hello`) and send the request again.
    BadCookie = 1,
    RateLimited = 2,
    /// The host's listing is gone (the service restarted, or it expired): register again.
    UnknownHost = 3,
    UnsupportedVersion = 4,
    /// The service is at a limit (too many games, relays or listings from one address).
    Busy = 5,
    /// The request was refused (bad secret, bad credential, bad values).
    Refused = 6,
}

impl ErrorCode {
    fn from_u8(v: u8) -> Result<Self, Malformed> {
        Ok(match v {
            1 => Self::BadCookie,
            2 => Self::RateLimited,
            3 => Self::UnknownHost,
            4 => Self::UnsupportedVersion,
            5 => Self::Busy,
            6 => Self::Refused,
            _ => return Err(Malformed),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinFailure {
    NotFound = 1,
    VersionMismatch = 2,
    Banned = 3,
    Busy = 4,
    RateLimited = 5,
}

impl JoinFailure {
    fn from_u8(v: u8) -> Result<Self, Malformed> {
        Ok(match v {
            1 => Self::NotFound,
            2 => Self::VersionMismatch,
            3 => Self::Banned,
            4 => Self::Busy,
            5 => Self::RateLimited,
            _ => return Err(Malformed),
        })
    }
}

/// A decoded datagram. C = the game (host or joiner), S = the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message<'a> {
    /// C→S, padded to [`PADDED_LEN`]: asks for a cookie.
    Hello {
        nonce: u64,
    },
    /// S→C: proves the address; also tells the sender how the service sees it.
    Cookie {
        nonce: u64,
        cookie: [u8; COOKIE_LEN],
        observed: SocketAddr,
    },
    /// C→S, padded: health check (deploys).
    Probe {
        nonce: u64,
    },
    ProbeReply {
        nonce: u64,
        version: String,
    },
    Error {
        nonce: u64,
        code: ErrorCode,
    },
    Ok {
        nonce: u64,
    },
    Register {
        cookie: [u8; COOKIE_LEN],
        nonce: u64,
        resume: Option<Resume>,
        lan: Option<SocketAddr>,
        info: HostInfo,
        bans: Vec<[u8; TAG_LEN]>,
    },
    Registered {
        nonce: u64,
        host_id: HostId,
        code: Code,
        secret: [u8; SECRET_LEN],
    },
    Heartbeat {
        cookie: [u8; COOKIE_LEN],
        nonce: u64,
        host_id: HostId,
        secret: [u8; SECRET_LEN],
        info: HostInfo,
    },
    Unregister {
        cookie: [u8; COOKIE_LEN],
        nonce: u64,
        host_id: HostId,
        secret: [u8; SECRET_LEN],
    },
    /// S→C (host), MACed with the host's secret ([`encode_introduce`]).
    Introduce {
        intro: Introduction,
        mac: [u8; MAC_LEN],
    },
    IntroAck {
        cookie: [u8; COOKIE_LEN],
        host_id: HostId,
        secret: [u8; SECRET_LEN],
        join_id: u64,
    },
    Ban {
        cookie: [u8; COOKIE_LEN],
        nonce: u64,
        host_id: HostId,
        secret: [u8; SECRET_LEN],
        tag: [u8; TAG_LEN],
    },
    List {
        cookie: [u8; COOKIE_LEN],
        nonce: u64,
        protocol: u32,
        page: u16,
    },
    ListPage {
        nonce: u64,
        page: u16,
        pages: u16,
        total: u32,
        entries: Vec<ListEntry>,
    },
    Join {
        cookie: [u8; COOKIE_LEN],
        nonce: u64,
        code: Code,
        protocol: u32,
        client_ids: [u64; 3],
        lan: Option<SocketAddr>,
        relay_only: bool,
    },
    JoinOk {
        nonce: u64,
        join_id: u64,
        host: Option<SocketAddr>,
        host_lan: Option<SocketAddr>,
        relay: Option<RelayGrant>,
    },
    JoinFailed {
        nonce: u64,
        reason: JoinFailure,
        /// The host's build, for [`JoinFailure::VersionMismatch`] (else empty).
        host_build: String,
    },
    /// Both ways: a netcode packet through relay session `session`.
    Relay {
        session: u64,
        payload: &'a [u8],
    },
    RelayBind {
        cookie: [u8; COOKIE_LEN],
        nonce: u64,
        session: u64,
        role: Role,
        grant_expires: u64,
        host_id: HostId,
        cred: [u8; CRED_LEN],
    },
    RelayBound {
        nonce: u64,
        session: u64,
    },
    /// S→C: the service doesn't know this session for the sender: bind it again.
    RelayUnknown {
        session: u64,
    },
    /// C↔C (host and joiner, directly): opens the NATs on the way. Carries nothing else.
    Punch {
        join_id: u64,
    },
}

impl Message<'_> {
    /// The address cookie of a request that needs one.
    pub fn cookie(&self) -> Option<&[u8; COOKIE_LEN]> {
        match self {
            Message::Register { cookie, .. }
            | Message::Heartbeat { cookie, .. }
            | Message::Unregister { cookie, .. }
            | Message::IntroAck { cookie, .. }
            | Message::Ban { cookie, .. }
            | Message::List { cookie, .. }
            | Message::Join { cookie, .. }
            | Message::RelayBind { cookie, .. } => Some(cookie),
            _ => None,
        }
    }

    /// The request nonce a reply must echo (0 if none).
    pub fn nonce(&self) -> u64 {
        match self {
            Message::Hello { nonce }
            | Message::Cookie { nonce, .. }
            | Message::Probe { nonce }
            | Message::ProbeReply { nonce, .. }
            | Message::Error { nonce, .. }
            | Message::Ok { nonce }
            | Message::Register { nonce, .. }
            | Message::Registered { nonce, .. }
            | Message::Heartbeat { nonce, .. }
            | Message::Unregister { nonce, .. }
            | Message::Ban { nonce, .. }
            | Message::List { nonce, .. }
            | Message::ListPage { nonce, .. }
            | Message::Join { nonce, .. }
            | Message::JoinOk { nonce, .. }
            | Message::JoinFailed { nonce, .. }
            | Message::RelayBind { nonce, .. }
            | Message::RelayBound { nonce, .. } => *nonce,
            _ => 0,
        }
    }
}

fn header(kind: u8, capacity: usize) -> Writer {
    let mut w = Writer::with_capacity(capacity);
    w.bytes(&MAGIC);
    w.u8(VERSION);
    w.u8(kind);
    w
}

/// A level name as the protocol allows it (`a-z`, `0-9`, `_`, `-`).
pub fn level_name(name: &str) -> String {
    name.chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(LEVEL_MAX)
        .collect()
}

fn put_info(w: &mut Writer, info: &HostInfo) {
    w.text(&info.name, NAME_MAX);
    w.u8(info.mode);
    w.text(&level_name(&info.level), LEVEL_MAX);
    w.u8(info.players.min(MAX_PLAYERS));
    w.u8(info.max_players.min(MAX_PLAYERS));
    let mut flags = 0;
    if info.public {
        flags |= FLAG_PUBLIC;
    }
    if info.relay_only {
        flags |= FLAG_RELAY_ONLY;
    }
    if info.in_match {
        flags |= FLAG_IN_MATCH;
    }
    w.u8(flags);
    w.u32(info.protocol);
    w.text(&info.build, BUILD_MAX);
}

fn is_level_name(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn get_info(r: &mut Reader) -> Result<HostInfo, Malformed> {
    let name = r.text(NAME_MAX)?;
    let mode = r.u8()?;
    let level = r.text(LEVEL_MAX)?;
    if !is_level_name(&level) {
        return Err(Malformed);
    }
    let players = r.u8()?;
    let max_players = r.u8()?;
    if players > MAX_PLAYERS || max_players > MAX_PLAYERS {
        return Err(Malformed);
    }
    let flags = r.u8()?;
    if flags & !(FLAG_PUBLIC | FLAG_RELAY_ONLY | FLAG_IN_MATCH) != 0 {
        return Err(Malformed);
    }
    Ok(HostInfo {
        name,
        mode,
        level,
        players,
        max_players,
        in_match: flags & FLAG_IN_MATCH != 0,
        protocol: r.u32()?,
        build: r.text(BUILD_MAX)?,
        public: flags & FLAG_PUBLIC != 0,
        relay_only: flags & FLAG_RELAY_ONLY != 0,
    })
}

fn put_grant(w: &mut Writer, grant: &Option<RelayGrant>) {
    match grant {
        None => w.u8(0),
        Some(g) => {
            w.u8(1);
            w.u64(g.session);
            w.u64(g.expires);
            w.bytes(&g.host_id);
            w.bytes(&g.cred);
        }
    }
}

fn get_grant(r: &mut Reader) -> Result<Option<RelayGrant>, Malformed> {
    if !r.bool()? {
        return Ok(None);
    }
    let session = r.u64()?;
    if session == 0 {
        return Err(Malformed);
    }
    Ok(Some(RelayGrant {
        session,
        expires: r.u64()?,
        host_id: r.array()?,
        cred: r.array()?,
    }))
}

fn get_code(r: &mut Reader) -> Result<Code, Malformed> {
    Code::from_bytes(r.array::<CODE_LEN>()?).ok_or(Malformed)
}

fn put_intro_body(w: &mut Writer, intro: &Introduction) {
    w.bytes(&intro.host_id);
    w.u64(intro.join_id);
    for id in intro.client_ids {
        w.u64(id);
    }
    w.addr(intro.joiner);
    w.addr(intro.joiner_lan);
    put_grant(w, &intro.relay);
    w.bytes(&intro.ban_tag);
    w.bool(intro.same_network);
}

/// An `Introduce`, MACed with the host's secret so nobody else can make a host admit or punch
/// toward anyone.
pub fn encode_introduce(intro: &Introduction, secret: &[u8; SECRET_LEN]) -> Vec<u8> {
    let mut w = header(kind::INTRODUCE, 160);
    put_intro_body(&mut w, intro);
    let mac = message_mac(secret, w.as_slice().get(HEADER_LEN..).unwrap_or_default());
    w.bytes(&mac);
    w.into_vec()
}

/// The part of an `Introduce` datagram its MAC covers (everything between the header and the
/// MAC), if `data` is long enough.
pub fn introduce_signed_part(data: &[u8]) -> Option<&[u8]> {
    data.get(HEADER_LEN..data.len().checked_sub(MAC_LEN)?)
}

/// A relayed packet.
pub fn encode_relay(session: u64, payload: &[u8]) -> Vec<u8> {
    let mut w = header(kind::RELAY, HEADER_LEN + 8 + payload.len());
    w.u64(session);
    w.bytes(payload);
    w.into_vec()
}

/// Encodes a message. `Introduce` needs the host's secret: use [`encode_introduce`] (this
/// encodes it with the MAC it carries).
pub fn encode(msg: &Message) -> Vec<u8> {
    match msg {
        Message::Hello { nonce } => {
            let mut w = header(kind::HELLO, PADDED_LEN);
            w.u64(*nonce);
            w.pad_to(PADDED_LEN);
            w.into_vec()
        }
        Message::Cookie {
            nonce,
            cookie,
            observed,
        } => {
            let mut w = header(kind::COOKIE, 48);
            w.u64(*nonce);
            w.bytes(cookie);
            w.addr(Some(*observed));
            w.into_vec()
        }
        Message::Probe { nonce } => {
            let mut w = header(kind::PROBE, PADDED_LEN);
            w.u64(*nonce);
            w.pad_to(PADDED_LEN);
            w.into_vec()
        }
        Message::ProbeReply { nonce, version } => {
            let mut w = header(kind::PROBE_REPLY, 48);
            w.u64(*nonce);
            w.text(version, BUILD_MAX);
            w.into_vec()
        }
        Message::Error { nonce, code } => {
            let mut w = header(kind::ERROR, 15);
            w.u64(*nonce);
            w.u8(*code as u8);
            w.into_vec()
        }
        Message::Ok { nonce } => {
            let mut w = header(kind::OK, 14);
            w.u64(*nonce);
            w.into_vec()
        }
        Message::Register {
            cookie,
            nonce,
            resume,
            lan,
            info,
            bans,
        } => {
            let mut w = header(kind::REGISTER, 256);
            w.bytes(cookie);
            w.u64(*nonce);
            match resume {
                None => w.u8(0),
                Some(r) => {
                    w.u8(1);
                    w.bytes(&r.host_id);
                    w.bytes(r.code.as_bytes());
                    w.bytes(&r.secret);
                }
            }
            w.addr(*lan);
            put_info(&mut w, info);
            let bans = bans.get(..bans.len().min(MAX_BANS)).unwrap_or_default();
            w.u8(bans.len() as u8);
            for tag in bans {
                w.bytes(tag);
            }
            w.into_vec()
        }
        Message::Registered {
            nonce,
            host_id,
            code,
            secret,
        } => {
            let mut w = header(kind::REGISTERED, 80);
            w.u64(*nonce);
            w.bytes(host_id);
            w.bytes(code.as_bytes());
            w.bytes(secret);
            w.into_vec()
        }
        Message::Heartbeat {
            cookie,
            nonce,
            host_id,
            secret,
            info,
        } => {
            let mut w = header(kind::HEARTBEAT, 200);
            w.bytes(cookie);
            w.u64(*nonce);
            w.bytes(host_id);
            w.bytes(secret);
            put_info(&mut w, info);
            w.into_vec()
        }
        Message::Unregister {
            cookie,
            nonce,
            host_id,
            secret,
        } => {
            let mut w = header(kind::UNREGISTER, 80);
            w.bytes(cookie);
            w.u64(*nonce);
            w.bytes(host_id);
            w.bytes(secret);
            w.into_vec()
        }
        Message::Introduce { intro, mac } => {
            let mut w = header(kind::INTRODUCE, 160);
            put_intro_body(&mut w, intro);
            w.bytes(mac);
            w.into_vec()
        }
        Message::IntroAck {
            cookie,
            host_id,
            secret,
            join_id,
        } => {
            let mut w = header(kind::INTRO_ACK, 80);
            w.bytes(cookie);
            w.bytes(host_id);
            w.bytes(secret);
            w.u64(*join_id);
            w.into_vec()
        }
        Message::Ban {
            cookie,
            nonce,
            host_id,
            secret,
            tag,
        } => {
            let mut w = header(kind::BAN, 96);
            w.bytes(cookie);
            w.u64(*nonce);
            w.bytes(host_id);
            w.bytes(secret);
            w.bytes(tag);
            w.into_vec()
        }
        Message::List {
            cookie,
            nonce,
            protocol,
            page,
        } => {
            let mut w = header(kind::LIST, 40);
            w.bytes(cookie);
            w.u64(*nonce);
            w.u32(*protocol);
            w.u16(*page);
            w.into_vec()
        }
        Message::ListPage {
            nonce,
            page,
            pages,
            total,
            entries,
        } => {
            let mut w = header(kind::LIST_PAGE, MAX_CONTROL_LEN);
            w.u64(*nonce);
            w.u16(*page);
            w.u16(*pages);
            w.u32(*total);
            let entries = entries
                .get(..entries.len().min(LIST_PAGE_LEN))
                .unwrap_or_default();
            w.u8(entries.len() as u8);
            for e in entries {
                w.bytes(e.code.as_bytes());
                w.text(&e.name, NAME_MAX);
                w.u8(e.mode);
                w.text(&level_name(&e.level), LEVEL_MAX);
                w.u8(e.players.min(MAX_PLAYERS));
                w.u8(e.max_players.min(MAX_PLAYERS));
                let mut flags = 0;
                if e.relay_only {
                    flags |= FLAG_RELAY_ONLY;
                }
                if e.in_match {
                    flags |= FLAG_IN_MATCH;
                }
                w.u8(flags);
            }
            w.into_vec()
        }
        Message::Join {
            cookie,
            nonce,
            code,
            protocol,
            client_ids,
            lan,
            relay_only,
        } => {
            let mut w = header(kind::JOIN, 96);
            w.bytes(cookie);
            w.u64(*nonce);
            w.bytes(code.as_bytes());
            w.u32(*protocol);
            for id in client_ids {
                w.u64(*id);
            }
            w.addr(*lan);
            w.bool(*relay_only);
            w.into_vec()
        }
        Message::JoinOk {
            nonce,
            join_id,
            host,
            host_lan,
            relay,
        } => {
            let mut w = header(kind::JOIN_OK, 96);
            w.u64(*nonce);
            w.u64(*join_id);
            w.addr(*host);
            w.addr(*host_lan);
            put_grant(&mut w, relay);
            w.into_vec()
        }
        Message::JoinFailed {
            nonce,
            reason,
            host_build,
        } => {
            let mut w = header(kind::JOIN_FAILED, 48);
            w.u64(*nonce);
            w.u8(*reason as u8);
            w.text(host_build, BUILD_MAX);
            w.into_vec()
        }
        Message::Relay { session, payload } => encode_relay(*session, payload),
        Message::RelayBind {
            cookie,
            nonce,
            session,
            role,
            grant_expires,
            host_id,
            cred,
        } => {
            let mut w = header(kind::RELAY_BIND, 80);
            w.bytes(cookie);
            w.u64(*nonce);
            w.u64(*session);
            w.u8(*role as u8);
            w.u64(*grant_expires);
            w.bytes(host_id);
            w.bytes(cred);
            w.into_vec()
        }
        Message::RelayBound { nonce, session } => {
            let mut w = header(kind::RELAY_BOUND, 22);
            w.u64(*nonce);
            w.u64(*session);
            w.into_vec()
        }
        Message::RelayUnknown { session } => {
            let mut w = header(kind::RELAY_UNKNOWN, 14);
            w.u64(*session);
            w.into_vec()
        }
        Message::Punch { join_id } => {
            let mut w = header(kind::PUNCH, 14);
            w.u64(*join_id);
            w.into_vec()
        }
    }
}

/// Decodes a datagram. Never panics; see the module docs for what is rejected.
pub fn decode(data: &[u8]) -> Result<Message<'_>, DecodeError> {
    if !is_rendezvous(data) {
        return Err(DecodeError::NotOurs);
    }
    if data.len() > MAX_DATAGRAM {
        return Err(DecodeError::Malformed);
    }
    let mut r = Reader::new(data);
    r.bytes(MAGIC.len())?;
    let version = r.u8()?;
    if version != VERSION {
        return Err(DecodeError::Version(version));
    }
    let kind = r.u8()?;
    // Relayed packets are the only ones longer than a control message.
    if kind != kind::RELAY && data.len() > MAX_CONTROL_LEN {
        return Err(DecodeError::Malformed);
    }
    let msg = match kind {
        kind::HELLO | kind::PROBE => {
            let nonce = r.u64()?;
            // Padding: at least up to PADDED_LEN, all zeros.
            if data.len() < PADDED_LEN || r.rest().iter().any(|b| *b != 0) {
                return Err(DecodeError::Malformed);
            }
            return Ok(if kind == kind::HELLO {
                Message::Hello { nonce }
            } else {
                Message::Probe { nonce }
            });
        }
        kind::COOKIE => Message::Cookie {
            nonce: r.u64()?,
            cookie: r.array()?,
            observed: r.addr()?.ok_or(Malformed)?,
        },
        kind::PROBE_REPLY => Message::ProbeReply {
            nonce: r.u64()?,
            version: r.text(BUILD_MAX)?,
        },
        kind::ERROR => Message::Error {
            nonce: r.u64()?,
            code: ErrorCode::from_u8(r.u8()?)?,
        },
        kind::OK => Message::Ok { nonce: r.u64()? },
        kind::REGISTER => {
            let cookie = r.array()?;
            let nonce = r.u64()?;
            let resume = if r.bool()? {
                Some(Resume {
                    host_id: r.array()?,
                    code: get_code(&mut r)?,
                    secret: r.array()?,
                })
            } else {
                None
            };
            let lan = r.addr()?;
            let info = get_info(&mut r)?;
            let n = r.u8()? as usize;
            if n > MAX_BANS {
                return Err(DecodeError::Malformed);
            }
            let mut bans = Vec::with_capacity(n);
            for _ in 0..n {
                bans.push(r.array()?);
            }
            Message::Register {
                cookie,
                nonce,
                resume,
                lan,
                info,
                bans,
            }
        }
        kind::REGISTERED => Message::Registered {
            nonce: r.u64()?,
            host_id: r.array()?,
            code: get_code(&mut r)?,
            secret: r.array()?,
        },
        kind::HEARTBEAT => Message::Heartbeat {
            cookie: r.array()?,
            nonce: r.u64()?,
            host_id: r.array()?,
            secret: r.array()?,
            info: get_info(&mut r)?,
        },
        kind::UNREGISTER => Message::Unregister {
            cookie: r.array()?,
            nonce: r.u64()?,
            host_id: r.array()?,
            secret: r.array()?,
        },
        kind::INTRODUCE => {
            let host_id = r.array::<HOST_ID_LEN>()?;
            let join_id = r.u64()?;
            let client_ids = [r.u64()?, r.u64()?, r.u64()?];
            let intro = Introduction {
                host_id,
                join_id,
                client_ids,
                joiner: r.addr()?,
                joiner_lan: r.addr()?,
                relay: get_grant(&mut r)?,
                ban_tag: r.array()?,
                same_network: r.bool()?,
            };
            Message::Introduce {
                intro,
                mac: r.array()?,
            }
        }
        kind::INTRO_ACK => Message::IntroAck {
            cookie: r.array()?,
            host_id: r.array()?,
            secret: r.array()?,
            join_id: r.u64()?,
        },
        kind::BAN => Message::Ban {
            cookie: r.array()?,
            nonce: r.u64()?,
            host_id: r.array()?,
            secret: r.array()?,
            tag: r.array()?,
        },
        kind::LIST => Message::List {
            cookie: r.array()?,
            nonce: r.u64()?,
            protocol: r.u32()?,
            page: r.u16()?,
        },
        kind::LIST_PAGE => {
            let nonce = r.u64()?;
            let page = r.u16()?;
            let pages = r.u16()?;
            let total = r.u32()?;
            let n = r.u8()? as usize;
            if n > LIST_PAGE_LEN {
                return Err(DecodeError::Malformed);
            }
            let mut entries = Vec::with_capacity(n);
            for _ in 0..n {
                let code = get_code(&mut r)?;
                let name = r.text(NAME_MAX)?;
                let mode = r.u8()?;
                let level = r.text(LEVEL_MAX)?;
                if !is_level_name(&level) {
                    return Err(DecodeError::Malformed);
                }
                let players = r.u8()?;
                let max_players = r.u8()?;
                if players > MAX_PLAYERS || max_players > MAX_PLAYERS {
                    return Err(DecodeError::Malformed);
                }
                let flags = r.u8()?;
                if flags & !(FLAG_RELAY_ONLY | FLAG_IN_MATCH) != 0 {
                    return Err(DecodeError::Malformed);
                }
                entries.push(ListEntry {
                    code,
                    name,
                    mode,
                    level,
                    players,
                    max_players,
                    in_match: flags & FLAG_IN_MATCH != 0,
                    relay_only: flags & FLAG_RELAY_ONLY != 0,
                });
            }
            Message::ListPage {
                nonce,
                page,
                pages,
                total,
                entries,
            }
        }
        kind::JOIN => Message::Join {
            cookie: r.array()?,
            nonce: r.u64()?,
            code: get_code(&mut r)?,
            protocol: r.u32()?,
            client_ids: [r.u64()?, r.u64()?, r.u64()?],
            lan: r.addr()?,
            relay_only: r.bool()?,
        },
        kind::JOIN_OK => Message::JoinOk {
            nonce: r.u64()?,
            join_id: r.u64()?,
            host: r.addr()?,
            host_lan: r.addr()?,
            relay: get_grant(&mut r)?,
        },
        kind::JOIN_FAILED => Message::JoinFailed {
            nonce: r.u64()?,
            reason: JoinFailure::from_u8(r.u8()?)?,
            host_build: r.text(BUILD_MAX)?,
        },
        kind::RELAY => {
            let session = r.u64()?;
            let payload = r.rest();
            if session == 0 || payload.is_empty() || payload.len() > MAX_RELAY_PAYLOAD {
                return Err(DecodeError::Malformed);
            }
            return Ok(Message::Relay { session, payload });
        }
        kind::RELAY_BIND => Message::RelayBind {
            cookie: r.array()?,
            nonce: r.u64()?,
            session: r.u64()?,
            role: Role::from_u8(r.u8()?)?,
            grant_expires: r.u64()?,
            host_id: r.array()?,
            cred: r.array()?,
        },
        kind::RELAY_BOUND => Message::RelayBound {
            nonce: r.u64()?,
            session: r.u64()?,
        },
        kind::RELAY_UNKNOWN => Message::RelayUnknown { session: r.u64()? },
        kind::PUNCH => Message::Punch { join_id: r.u64()? },
        _ => return Err(DecodeError::Malformed),
    };
    r.finish()?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> HostInfo {
        HostInfo {
            name: "SAM'S GAME".into(),
            mode: 1,
            level: "knell_yard".into(),
            players: 3,
            max_players: 8,
            in_match: true,
            protocol: 11,
            build: "build-42".into(),
            public: true,
            relay_only: false,
        }
    }

    fn samples() -> Vec<Message<'static>> {
        let code = Code::parse("ABCD-EFGH").unwrap();
        let grant = Some(RelayGrant {
            session: 77,
            expires: 1_900_000_000,
            host_id: [2; 16],
            cred: [9; 16],
        });
        let a: SocketAddr = "1.2.3.4:5".parse().unwrap();
        let entry = ListEntry {
            code,
            name: "X".into(),
            mode: 2,
            level: "spire_of_wicks".into(),
            players: 1,
            max_players: 4,
            in_match: false,
            relay_only: true,
        };
        vec![
            Message::Hello { nonce: 1 },
            Message::Cookie {
                nonce: 1,
                cookie: [3; 16],
                observed: a,
            },
            Message::Probe { nonce: 2 },
            Message::ProbeReply {
                nonce: 2,
                version: "build-42 (abcdef0)".into(),
            },
            Message::Error {
                nonce: 3,
                code: ErrorCode::Busy,
            },
            Message::Ok { nonce: 4 },
            Message::Register {
                cookie: [1; 16],
                nonce: 5,
                resume: Some(Resume {
                    host_id: [2; 16],
                    code,
                    secret: [4; 32],
                }),
                lan: Some("192.168.1.2:47420".parse().unwrap()),
                info: info(),
                bans: vec![[5; 16], [6; 16]],
            },
            Message::Register {
                cookie: [1; 16],
                nonce: 5,
                resume: None,
                lan: None,
                info: HostInfo::default(),
                bans: vec![],
            },
            Message::Registered {
                nonce: 6,
                host_id: [2; 16],
                code,
                secret: [4; 32],
            },
            Message::Heartbeat {
                cookie: [1; 16],
                nonce: 7,
                host_id: [2; 16],
                secret: [4; 32],
                info: info(),
            },
            Message::Unregister {
                cookie: [1; 16],
                nonce: 8,
                host_id: [2; 16],
                secret: [4; 32],
            },
            Message::Introduce {
                intro: Introduction {
                    host_id: [2; 16],
                    join_id: 9,
                    client_ids: [10, 11, 12],
                    joiner: Some(a),
                    joiner_lan: None,
                    relay: grant,
                    ban_tag: [7; 16],
                    same_network: false,
                },
                mac: [8; 16],
            },
            Message::IntroAck {
                cookie: [1; 16],
                host_id: [2; 16],
                secret: [4; 32],
                join_id: 9,
            },
            Message::Ban {
                cookie: [1; 16],
                nonce: 10,
                host_id: [2; 16],
                secret: [4; 32],
                tag: [7; 16],
            },
            Message::List {
                cookie: [1; 16],
                nonce: 11,
                protocol: 11,
                page: 0,
            },
            Message::ListPage {
                nonce: 11,
                page: 0,
                pages: 1,
                total: 14,
                entries: vec![entry; LIST_PAGE_LEN],
            },
            Message::Join {
                cookie: [1; 16],
                nonce: 12,
                code,
                protocol: 11,
                client_ids: [1, 2, 3],
                lan: None,
                relay_only: true,
            },
            Message::JoinOk {
                nonce: 12,
                join_id: 9,
                host: Some(a),
                host_lan: None,
                relay: grant,
            },
            Message::JoinFailed {
                nonce: 12,
                reason: JoinFailure::VersionMismatch,
                host_build: "build-41".into(),
            },
            Message::Relay {
                session: 77,
                payload: &[1, 2, 3],
            },
            Message::RelayBind {
                cookie: [1; 16],
                nonce: 13,
                session: 77,
                role: Role::Joiner,
                grant_expires: 1_900_000_000,
                host_id: [2; 16],
                cred: [9; 16],
            },
            Message::RelayBound {
                nonce: 13,
                session: 77,
            },
            Message::RelayUnknown { session: 77 },
            Message::Punch { join_id: 9 },
        ]
    }

    #[test]
    fn every_message_round_trips() {
        for msg in samples() {
            let data = encode(&msg);
            assert!(
                data.len() <= MAX_CONTROL_LEN,
                "{msg:?}: {} bytes",
                data.len()
            );
            assert_eq!(decode(&data), Ok(msg.clone()), "{msg:?}");
        }
    }

    #[test]
    fn trailing_and_truncated_datagrams_are_rejected() {
        for msg in samples() {
            let data = encode(&msg);
            let padded = matches!(msg, Message::Hello { .. } | Message::Probe { .. });
            if !padded && !matches!(msg, Message::Relay { .. }) {
                let mut longer = data.clone();
                longer.push(0);
                assert_eq!(decode(&longer), Err(DecodeError::Malformed), "{msg:?}");
            }
            for cut in 0..data.len() {
                let short = &data[..cut];
                if matches!(msg, Message::Relay { .. }) && cut > HEADER_LEN + 8 {
                    continue; // a shorter payload is still a payload
                }
                assert!(decode(short).is_err(), "{msg:?} cut at {cut}");
            }
        }
    }

    #[test]
    fn padding_must_be_zero_and_present() {
        let data = encode(&Message::Hello { nonce: 1 });
        assert_eq!(data.len(), PADDED_LEN);
        let mut bad = data.clone();
        bad[PADDED_LEN - 1] = 1;
        assert_eq!(decode(&bad), Err(DecodeError::Malformed));
        assert_eq!(decode(&data[..PADDED_LEN - 1]), Err(DecodeError::Malformed));
    }

    #[test]
    fn foreign_and_other_versions() {
        assert_eq!(decode(&[1, 2, 3]), Err(DecodeError::NotOurs));
        let mut data = encode(&Message::Ok { nonce: 1 });
        data[4] = 2;
        assert_eq!(decode(&data), Err(DecodeError::Version(2)));
    }

    #[test]
    fn values_out_of_range_are_rejected() {
        // A level name with capitals (the encoder never writes one: patch it in).
        let mut data = encode(&Message::Heartbeat {
            cookie: [0; 16],
            nonce: 1,
            host_id: [0; 16],
            secret: [0; 32],
            info: info(),
        });
        let level_at = HEADER_LEN + 16 + 8 + 16 + 32 + 1 + info().name.len() + 1 + 1;
        assert_eq!(data[level_at], b'k');
        data[level_at] = b'K';
        assert_eq!(decode(&data), Err(DecodeError::Malformed));
        // An unknown flag bit.
        let mut data = encode(&Message::Heartbeat {
            cookie: [0; 16],
            nonce: 1,
            host_id: [0; 16],
            secret: [0; 32],
            info: info(),
        });
        // flags byte: after name, mode, level, players, max.
        let flags_at = HEADER_LEN + 16 + 8 + 16 + 32 + 1 + info().name.len() + 1 + 1 + 10 + 2;
        assert_eq!(data[flags_at], FLAG_PUBLIC | FLAG_IN_MATCH);
        data[flags_at] |= 0x80;
        assert_eq!(decode(&data), Err(DecodeError::Malformed));
        // Relay session 0 and empty payloads.
        assert_eq!(decode(&encode_relay(0, &[1])), Err(DecodeError::Malformed));
        assert_eq!(decode(&encode_relay(5, &[])), Err(DecodeError::Malformed));
        assert_eq!(
            decode(&encode_relay(5, &[0; MAX_RELAY_PAYLOAD + 1])),
            Err(DecodeError::Malformed)
        );
        assert!(decode(&encode_relay(5, &[0; MAX_RELAY_PAYLOAD])).is_ok());
    }

    #[test]
    fn any_host_info_encodes_validly() {
        let info = HostInfo {
            name: "Ünïcode\tname that is much longer than thirty-two bytes".into(),
            level: "Weird Level/Name+That's Very Long Indeed".into(),
            players: 200,
            max_players: 255,
            ..info()
        };
        let msg = Message::Heartbeat {
            cookie: [0; 16],
            nonce: 1,
            host_id: [0; 16],
            secret: [0; 32],
            info,
        };
        let Ok(Message::Heartbeat { info, .. }) = decode(&encode(&msg)) else {
            panic!("didn't decode");
        };
        assert_eq!(info.level, "weird_level_name_that_s_very_lon");
        assert_eq!(info.name.len(), NAME_MAX);
        assert_eq!((info.players, info.max_players), (MAX_PLAYERS, MAX_PLAYERS));
    }

    #[test]
    fn introduce_mac_covers_the_body() {
        let intro = Introduction {
            host_id: [2; 16],
            join_id: 9,
            client_ids: [10, 11, 12],
            joiner: None,
            joiner_lan: None,
            relay: None,
            ban_tag: [7; 16],
            same_network: true,
        };
        let secret = [5; 32];
        let data = encode_introduce(&intro, &secret);
        let Ok(Message::Introduce { mac, .. }) = decode(&data) else {
            panic!("not an introduce");
        };
        let body = introduce_signed_part(&data).unwrap();
        assert_eq!(mac, message_mac(&secret, body));
        assert_ne!(mac, message_mac(&[6; 32], body));
    }

    #[test]
    fn random_bytes_never_panic() {
        // A cheap fuzz: random datagrams, and every sample with random corruptions.
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let samples: Vec<Vec<u8>> = samples().iter().map(encode).collect();
        for i in 0..200_000 {
            let mut data = if i % 2 == 0 {
                samples[(next() as usize) % samples.len()].clone()
            } else {
                let len = (next() % 1500) as usize;
                let mut d: Vec<u8> = (0..len).map(|_| next() as u8).collect();
                if len >= 6 && next() % 2 == 0 {
                    d[..4].copy_from_slice(&MAGIC);
                    d[4] = VERSION;
                }
                d
            };
            for _ in 0..(next() % 4) {
                if !data.is_empty() {
                    let at = (next() as usize) % data.len();
                    data[at] = next() as u8;
                }
            }
            if next() % 4 == 0 {
                let cut = (next() as usize) % (data.len() + 1);
                data.truncate(cut);
            }
            let _ = decode(&data);
        }
    }
}
