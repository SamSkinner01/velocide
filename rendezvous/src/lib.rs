//! VELOCIDE's online lobbies without port forwarding (docs/rendezvous.md).
//!
//! A small public service (the `velocide-rendezvous` binary, on a VPS) lists games, hands out
//! join codes and introduces joiners to hosts so both can open their NATs toward each other
//! (UDP hole punching). When that doesn't work (strict NATs, carrier-grade NAT, or a player
//! who doesn't want to reveal their address) it relays the game's packets. The game keeps its
//! netcode connection end to end: the service only ever sees netcode's packets, never
//! decrypts them, and the host still decides who gets in.
//!
//! - [`proto`]: the wire protocol; [`code`]: join codes; [`crypto`]: keys and MACs.
//! - [`service`]: the service itself, without IO.
//! - [`host`] and [`join`]: the game's side (a host's listing and admissions; a joiner's
//!   attempts), also without IO: the game's transports (`server::transport`,
//!   `client::net::online`) feed them the datagrams from their netcode socket.
//! - [`browse`]: the server browser (owns its own socket).

#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod browse;
pub mod code;
pub mod crypto;
pub mod host;
pub mod join;
pub mod limit;
pub mod proto;
pub mod service;
pub mod wire;

use std::net::{IpAddr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

pub use code::Code;
pub use host::{HostAgent, HostConfig, HostStatus};
pub use join::{JoinAgent, JoinConfig};
pub use proto::HostInfo;
pub use service::{Service, ServiceConfig};

/// What a game transport does with a datagram from its socket ([`HostAgent::on_receive`],
/// [`JoinAgent::on_receive`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inbound<'a> {
    /// A netcode packet from `0`: pass it on to netcode.
    Netcode(SocketAddr),
    /// A relayed netcode packet: pass `payload` to netcode as if it came from `from` (a
    /// [`relay_addr`]).
    Relayed { from: SocketAddr, payload: &'a [u8] },
    /// Handled here, or refused: nothing for netcode.
    Drop,
}

/// How a game transport sends a netcode packet ([`HostAgent::outbound`],
/// [`JoinAgent::outbound`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outbound {
    /// To its address, as usual.
    Direct,
    /// Wrapped for the relay: send these bytes to the service instead.
    Relay(SocketAddr, Vec<u8>),
    /// To a relay session we have no grant for: don't send.
    Drop,
}

/// A request waiting for its reply: resent with the same nonce until answered.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pending {
    pub nonce: u64,
    last: Option<std::time::Duration>,
}

impl Pending {
    pub fn new() -> Self {
        Self {
            nonce: crypto::random_nonzero_u64(),
            last: None,
        }
    }

    /// Time to send (again)? Marks it sent if so.
    pub fn send_now(&mut self, now: std::time::Duration, every: std::time::Duration) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.saturating_sub(last) >= every);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// The service's UDP port.
pub const DEFAULT_PORT: u16 = 47400;

/// The service the game uses unless the player set another: `VELOCIDE_RENDEZVOUS` at compile
/// time (`host:port`; CI sets it from the repository variable of the same name). Empty: this
/// build has no service, online play is off until one is set in the settings.
pub const BUILT_IN_SERVICE: &str = match option_env!("VELOCIDE_RENDEZVOUS") {
    Some(s) => s,
    None => "",
};

/// The prefix of the stand-in addresses relayed peers get ([`relay_addr`]): `100::/64`, the
/// IPv6 discard prefix (RFC 6666), so no real packet ever comes from one.
const RELAY_PREFIX: u128 = 0x0100 << 112;
const RELAY_PREFIX_MASK: u128 = !((1u128 << 64) - 1);
const RELAY_PORT: u16 = 1;

/// The address netcode sees for a peer reached through relay `session`: the transports turn
/// packets to it into relayed packets to the service, and relayed packets from the service
/// into packets from it.
pub fn relay_addr(session: u64) -> SocketAddr {
    SocketAddr::new(
        Ipv6Addr::from(RELAY_PREFIX | session as u128).into(),
        RELAY_PORT,
    )
}

/// The relay session a [`relay_addr`] stands for.
pub fn relay_session(addr: SocketAddr) -> Option<u64> {
    match addr {
        SocketAddr::V6(a) if a.port() == RELAY_PORT => {
            let bits = u128::from(*a.ip());
            let session = bits as u64;
            (bits & RELAY_PREFIX_MASK == RELAY_PREFIX && session != 0).then_some(session)
        }
        _ => None,
    }
}

/// Splits `host:port` (or `host`, with [`DEFAULT_PORT`]).
pub fn parse_service(name: &str) -> Result<(String, u16), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("no online service is set".into());
    }
    if name.len() > 255 {
        return Err("the online service's address is too long".into());
    }
    if let Ok(addr) = name.parse::<SocketAddr>() {
        return Ok((addr.ip().to_string(), addr.port()));
    }
    match name.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("'{name}' has a bad port"))?;
            Ok((host.to_string(), port))
        }
        _ => Ok((name.to_string(), DEFAULT_PORT)),
    }
}

/// Looks the service up without blocking the caller (DNS can take seconds).
pub struct Resolver {
    rx: Option<Receiver<Result<SocketAddr, String>>>,
    result: Option<Result<SocketAddr, String>>,
}

impl Resolver {
    pub fn start(name: &str) -> Resolver {
        let (host, port) = match parse_service(name) {
            Ok(parts) => parts,
            Err(e) => {
                return Resolver {
                    rx: None,
                    result: Some(Err(e)),
                };
            }
        };
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Resolver {
                rx: None,
                result: Some(Ok(SocketAddr::new(ip, port))),
            };
        }
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("rendezvous-dns".into())
            .spawn(move || {
                let result = (host.as_str(), port)
                    .to_socket_addrs()
                    .map_err(|e| format!("can't look up the online service '{host}': {e}"))
                    .and_then(|addrs| {
                        // The game's sockets are IPv4.
                        addrs.into_iter().find(SocketAddr::is_ipv4).ok_or_else(|| {
                            format!("the online service '{host}' has no IPv4 address")
                        })
                    });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => Resolver {
                rx: Some(rx),
                result: None,
            },
            Err(e) => Resolver {
                rx: None,
                result: Some(Err(format!("can't start the DNS lookup: {e}"))),
            },
        }
    }

    /// The result, once there is one.
    pub fn poll(&mut self) -> Option<&Result<SocketAddr, String>> {
        if self.result.is_none()
            && let Some(rx) = &self.rx
        {
            match rx.try_recv() {
                Ok(result) => self.result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.result = Some(Err("the DNS lookup failed".into()));
                }
            }
        }
        self.result.as_ref()
    }
}

/// This machine's address on its LAN, if it can be determined.
///
/// A UDP socket `connect`ed to an outside address makes the OS pick the outgoing interface
/// (and so our address on that network) without sending a single packet. Machines without a
/// default route try a few private ranges.
pub fn local_lan_ip() -> Option<IpAddr> {
    const PROBES: [&str; 4] = [
        "8.8.8.8:80",
        "192.168.254.254:9",
        "10.254.254.254:9",
        "172.31.254.254:9",
    ];
    PROBES.iter().find_map(|target| {
        let target: SocketAddr = target.parse().ok()?;
        let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
        socket.connect(target).ok()?;
        let ip = socket.local_addr().ok()?.ip();
        (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_addresses_round_trip_and_are_never_real() {
        for session in [1u64, 42, u64::MAX, 0x0123_4567_89ab_cdef] {
            let addr = relay_addr(session);
            assert_eq!(relay_session(addr), Some(session));
        }
        assert_eq!(relay_session("1.2.3.4:1".parse().unwrap()), None);
        assert_eq!(relay_session("[::1]:1".parse().unwrap()), None);
        assert_eq!(relay_session("[100::5]:2".parse().unwrap()), None, "port");
        assert_eq!(
            relay_session("[100:0:0:1::5]:1".parse().unwrap()),
            None,
            "prefix"
        );
        assert_eq!(
            relay_session("[100::]:1".parse().unwrap()),
            None,
            "session 0"
        );
    }

    #[test]
    fn service_names() {
        assert_eq!(
            parse_service("play.example.com:47401"),
            Ok(("play.example.com".into(), 47401))
        );
        assert_eq!(
            parse_service(" play.example.com "),
            Ok(("play.example.com".into(), DEFAULT_PORT))
        );
        assert_eq!(parse_service("1.2.3.4:9"), Ok(("1.2.3.4".into(), 9)));
        assert_eq!(parse_service("::1"), Ok(("::1".into(), DEFAULT_PORT)));
        assert!(parse_service("").is_err());
        assert!(parse_service("host:99999").is_err());
    }

    #[test]
    fn resolver_takes_addresses_directly() {
        let mut r = Resolver::start("127.0.0.1:5");
        assert_eq!(r.poll(), Some(&Ok("127.0.0.1:5".parse().unwrap())));
        let mut r = Resolver::start("localhost:6");
        let start = std::time::Instant::now();
        while r.poll().is_none() && start.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            r.poll().and_then(|r| r.as_ref().ok()).map(|a| a.port()),
            Some(6)
        );
    }
}
