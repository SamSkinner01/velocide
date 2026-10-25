//! This machine's LAN address, for the lobby's "friends join with ..." line.
//!
//! A UDP socket `connect`ed to an outside address makes the OS pick the outgoing interface
//! (and so our address on that network) without sending a single packet. Machines without a
//! default route try a few private ranges; if nothing works the caller shows a generic hint.

use std::net::{IpAddr, SocketAddr, UdpSocket};

/// Probe targets: a public address first, then private ranges (offline LANs).
const PROBES: [&str; 4] = [
    "8.8.8.8:80",
    "192.168.254.254:9",
    "10.254.254.254:9",
    "172.31.254.254:9",
];

/// The address other machines on the LAN reach us at, if it can be determined.
pub fn local_lan_ip() -> Option<IpAddr> {
    PROBES.iter().find_map(|target| {
        let target: SocketAddr = target.parse().ok()?;
        let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
        socket.connect(target).ok()?;
        let ip = socket.local_addr().ok()?.ip();
        (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
    })
}

/// "FRIENDS JOIN WITH 192.168.1.20:47420" (or a generic hint).
pub fn join_hint(port: u16) -> String {
    match local_lan_ip() {
        Some(ip) => format!("FRIENDS JOIN WITH {}", SocketAddr::new(ip, port)),
        None => format!("FRIENDS JOIN WITH YOUR IP AND PORT {port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_always_names_the_port() {
        assert!(join_hint(47421).contains("47421"));
        if let Some(ip) = local_lan_ip() {
            assert!(!ip.is_loopback());
        }
    }
}
