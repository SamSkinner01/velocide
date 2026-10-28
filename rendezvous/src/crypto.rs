//! Keys, MACs and randomness.
//!
//! The service keeps one 32-byte master key (a file that survives restarts) and derives a
//! key per purpose from it. Everything it hands out (address cookies, host secrets, relay
//! credentials, ban tags) is an HMAC-SHA256 under one of those keys, so the service can check
//! them after a restart without having stored them: a deploy doesn't end lobbies or relayed
//! games (docs/rendezvous.md "Restarts").

use std::net::{IpAddr, SocketAddr};

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::code::Code;
use crate::wire::{addr_bytes, ip_bytes};

type HmacSha256 = Hmac<Sha256>;

pub const KEY_LEN: usize = 32;
pub const COOKIE_LEN: usize = 16;
pub const SECRET_LEN: usize = 32;
pub const CRED_LEN: usize = 16;
pub const TAG_LEN: usize = 16;
pub const MAC_LEN: usize = 16;
pub const HOST_ID_LEN: usize = 16;

pub type HostId = [u8; HOST_ID_LEN];

/// HMAC-SHA256 of the concatenated `parts` under `key`.
fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    // HMAC accepts keys of any length: `new_from_slice` can't fail.
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return [0; 32];
    };
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

fn truncated<const N: usize>(full: [u8; 32]) -> [u8; N] {
    let mut out = [0u8; N];
    let n = N.min(32);
    if let (Some(dst), Some(src)) = (out.get_mut(..n), full.get(..n)) {
        dst.copy_from_slice(src);
    }
    out
}

/// Constant-time equality (secrets and MACs; different lengths are unequal).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

/// `N` bytes from the operating system's cryptographic random source.
///
/// # Panics
/// If the OS has no random source to give (the service can't run safely without one).
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    #[allow(clippy::expect_used)]
    getrandom::fill(&mut out).expect("the OS random source failed");
    out
}

pub fn random_u64() -> u64 {
    u64::from_le_bytes(random_bytes())
}

/// A random u64 that isn't 0 (0 means "none" on the wire).
pub fn random_nonzero_u64() -> u64 {
    loop {
        let v = random_u64();
        if v != 0 {
            return v;
        }
    }
}

/// The per-purpose keys of a service.
#[derive(Clone)]
pub struct Keys {
    cookie: [u8; KEY_LEN],
    host: [u8; KEY_LEN],
    relay: [u8; KEY_LEN],
    ban: [u8; KEY_LEN],
}

impl Keys {
    pub fn derive(master: &[u8; KEY_LEN]) -> Keys {
        let derive = |label: &str| hmac(master, &[b"velocide-rendezvous/v1/", label.as_bytes()]);
        Keys {
            cookie: derive("cookie"),
            host: derive("host"),
            relay: derive("relay"),
            ban: derive("ban"),
        }
    }

    /// Proves the bearer receives packets at `addr` (sent there in a `Cookie`). `window` is
    /// the time window it belongs to ([`crate::proto::cookie_window`]).
    pub fn cookie(&self, addr: SocketAddr, window: u64) -> [u8; COOKIE_LEN] {
        truncated(hmac(
            &self.cookie,
            &[b"cookie", &addr_bytes(addr), &window.to_le_bytes()],
        ))
    }

    /// The secret a host proves it owns its listing with.
    pub fn host_secret(&self, host_id: &HostId, code: &Code) -> [u8; SECRET_LEN] {
        hmac(&self.host, &[b"host", host_id, code.as_bytes()])
    }

    /// Lets the holder bind `role`'s end of relay `session` until `expires` (Unix seconds).
    pub fn relay_cred(
        &self,
        session: u64,
        role: u8,
        expires: u64,
        host_id: &HostId,
    ) -> [u8; CRED_LEN] {
        truncated(hmac(
            &self.relay,
            &[
                b"relay",
                &session.to_le_bytes(),
                &[role],
                &expires.to_le_bytes(),
                host_id,
            ],
        ))
    }

    /// What a host bans a joiner by: names the joiner's IP for this host only, without
    /// revealing it.
    pub fn ban_tag(&self, host_id: &HostId, ip: IpAddr) -> [u8; TAG_LEN] {
        truncated(hmac(&self.ban, &[b"ban", host_id, &ip_bytes(ip)]))
    }
}

/// Authenticates a service → host message with the host's secret.
pub fn message_mac(secret: &[u8; SECRET_LEN], body: &[u8]) -> [u8; MAC_LEN] {
    truncated(hmac(secret, &[b"service-to-host", body]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_values_depend_on_every_input() {
        let keys = Keys::derive(&[1; 32]);
        let other = Keys::derive(&[2; 32]);
        let a: SocketAddr = "1.2.3.4:5".parse().unwrap();
        let b: SocketAddr = "1.2.3.4:6".parse().unwrap();
        assert_eq!(keys.cookie(a, 7), keys.cookie(a, 7));
        assert_ne!(keys.cookie(a, 7), keys.cookie(b, 7));
        assert_ne!(keys.cookie(a, 7), keys.cookie(a, 8));
        assert_ne!(keys.cookie(a, 7), other.cookie(a, 7));

        let code = Code::parse("ABCD-EFGH").unwrap();
        let code2 = Code::parse("ABCD-EFGJ").unwrap();
        assert_ne!(
            keys.host_secret(&[0; 16], &code),
            keys.host_secret(&[1; 16], &code)
        );
        assert_ne!(
            keys.host_secret(&[0; 16], &code),
            keys.host_secret(&[0; 16], &code2)
        );

        let c = keys.relay_cred(9, 0, 100, &[0; 16]);
        assert_ne!(c, keys.relay_cred(9, 1, 100, &[0; 16]));
        assert_ne!(c, keys.relay_cred(8, 0, 100, &[0; 16]));
        assert_ne!(c, keys.relay_cred(9, 0, 101, &[0; 16]));
        assert_ne!(c, keys.relay_cred(9, 0, 100, &[1; 16]));

        let ip = a.ip();
        assert_ne!(keys.ban_tag(&[0; 16], ip), keys.ban_tag(&[1; 16], ip));
        assert_ne!(
            keys.ban_tag(&[0; 16], ip),
            keys.ban_tag(&[0; 16], "1.2.3.5".parse().unwrap())
        );
        // Purposes are separated: the same inputs under different keys differ.
        assert_ne!(keys.cookie, keys.relay);
        assert_ne!(keys.host, keys.ban);
    }

    #[test]
    fn constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn random_values_differ() {
        assert_ne!(random_bytes::<16>(), random_bytes::<16>());
        assert_ne!(random_nonzero_u64(), 0);
    }
}
