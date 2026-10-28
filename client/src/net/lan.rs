//! This machine's LAN address, for the lobby's "friends join with ..." line (the probe itself
//! is `rendezvous::local_lan_ip`, which online hosting uses too). If it can't be determined
//! the caller shows a generic hint.

use std::net::SocketAddr;

pub use rendezvous::local_lan_ip;

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
