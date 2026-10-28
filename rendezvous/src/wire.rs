//! Bounded binary reader and writer for the rendezvous protocol.
//!
//! Every read is length-checked against the datagram and returns [`Malformed`] instead of
//! panicking; nothing is allocated beyond what the datagram itself holds (strings are at most
//! 255 bytes by construction).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// The bytes don't form a valid message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

pub type Result<T> = std::result::Result<T, Malformed>;

const ADDR_NONE: u8 = 0;
const ADDR_V4: u8 = 4;
const ADDR_V6: u8 = 6;

/// Reads little-endian fields from a byte slice.
pub struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn remaining(&self) -> usize {
        self.data.len()
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.data.len() < n {
            return Err(Malformed);
        }
        let (head, tail) = self.data.split_at(n);
        self.data = tail;
        Ok(head)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// 0 or 1; anything else is malformed.
    pub fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Malformed),
        }
    }

    pub fn addr(&mut self) -> Result<Option<SocketAddr>> {
        let ip: IpAddr = match self.u8()? {
            ADDR_NONE => return Ok(None),
            ADDR_V4 => Ipv4Addr::from(self.array::<4>()?).into(),
            ADDR_V6 => Ipv6Addr::from(self.array::<16>()?).into(),
            _ => return Err(Malformed),
        };
        Ok(Some(SocketAddr::new(ip, self.u16()?)))
    }

    /// A length-prefixed string of at most `max` bytes of printable ASCII (space to `~`).
    pub fn text(&mut self, max: usize) -> Result<String> {
        let len = self.u8()? as usize;
        if len > max {
            return Err(Malformed);
        }
        let bytes = self.bytes(len)?;
        if !bytes.iter().all(|b| (0x20..=0x7e).contains(b)) {
            return Err(Malformed);
        }
        // Printable ASCII is valid UTF-8.
        String::from_utf8(bytes.to_vec()).map_err(|_| Malformed)
    }

    /// The unread bytes (consumes the reader).
    pub fn rest(self) -> &'a [u8] {
        self.data
    }

    /// Errors unless every byte was read.
    pub fn finish(self) -> Result<()> {
        if self.data.is_empty() {
            Ok(())
        } else {
            Err(Malformed)
        }
    }
}

/// Builds a datagram.
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            buf: Vec::with_capacity(n),
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    pub fn addr(&mut self, addr: Option<SocketAddr>) {
        match addr {
            None => self.u8(ADDR_NONE),
            Some(SocketAddr::V4(a)) => {
                self.u8(ADDR_V4);
                self.bytes(&a.ip().octets());
                self.u16(a.port());
            }
            Some(SocketAddr::V6(a)) => {
                self.u8(ADDR_V6);
                self.bytes(&a.ip().octets());
                self.u16(a.port());
            }
        }
    }

    /// Writes `text` cut to `max` bytes with anything outside printable ASCII replaced by `?`,
    /// so the result always decodes with [`Reader::text`].
    pub fn text(&mut self, text: &str, max: usize) {
        let max = max.min(u8::MAX as usize);
        let clean: Vec<u8> = text
            .bytes()
            .map(|b| if (0x20..=0x7e).contains(&b) { b } else { b'?' })
            .take(max)
            .collect();
        self.u8(clean.len() as u8);
        self.bytes(&clean);
    }

    /// Appends zeros up to `len` bytes in total (no-op if already that long).
    pub fn pad_to(&mut self, len: usize) {
        if self.buf.len() < len {
            self.buf.resize(len, 0);
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

/// The address with IPv4-mapped IPv6 turned into plain IPv4, so one peer always has one
/// spelling (keys, comparisons and MACs depend on it).
pub fn canonical(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V6(a) => SocketAddr::new(a.ip().to_canonical(), a.port()),
        v4 => v4,
    }
}

/// Fixed-size bytes of an address for MAC inputs (`canonical` first).
pub fn addr_bytes(addr: SocketAddr) -> [u8; 19] {
    let mut out = [0u8; 19];
    let addr = canonical(addr);
    let (tag, ip): (u8, [u8; 16]) = match addr.ip() {
        IpAddr::V4(ip) => (ADDR_V4, ip.to_ipv6_mapped().octets()),
        IpAddr::V6(ip) => (ADDR_V6, ip.octets()),
    };
    let mut w = Writer::with_capacity(19);
    w.u8(tag);
    w.bytes(&ip);
    w.u16(addr.port());
    out.copy_from_slice(&w.into_vec());
    out
}

/// [`addr_bytes`] without the port.
pub fn ip_bytes(ip: IpAddr) -> [u8; 17] {
    let mut out = [0u8; 17];
    let (tag, octets): (u8, [u8; 16]) = match ip.to_canonical() {
        IpAddr::V4(ip) => (ADDR_V4, ip.to_ipv6_mapped().octets()),
        IpAddr::V6(ip) => (ADDR_V6, ip.octets()),
    };
    let mut w = Writer::with_capacity(17);
    w.u8(tag);
    w.bytes(&octets);
    out.copy_from_slice(&w.into_vec());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut w = Writer::default();
        w.u8(7);
        w.u16(0xBEEF);
        w.u32(0xDEAD_BEEF);
        w.u64(u64::MAX - 3);
        w.bool(true);
        w.addr(Some("1.2.3.4:5".parse().unwrap()));
        w.addr(Some("[::1]:6".parse().unwrap()));
        w.addr(None);
        w.text("Hi there", 32);
        let data = w.into_vec();
        let mut r = Reader::new(&data);
        assert_eq!(r.u8(), Ok(7));
        assert_eq!(r.u16(), Ok(0xBEEF));
        assert_eq!(r.u32(), Ok(0xDEAD_BEEF));
        assert_eq!(r.u64(), Ok(u64::MAX - 3));
        assert_eq!(r.bool(), Ok(true));
        assert_eq!(r.addr(), Ok(Some("1.2.3.4:5".parse().unwrap())));
        assert_eq!(r.addr(), Ok(Some("[::1]:6".parse().unwrap())));
        assert_eq!(r.addr(), Ok(None));
        assert_eq!(r.text(32).as_deref(), Ok("Hi there"));
        r.finish().unwrap();
    }

    #[test]
    fn short_and_bad_input_is_malformed() {
        assert_eq!(Reader::new(&[1]).u16(), Err(Malformed));
        assert_eq!(Reader::new(&[2]).bool(), Err(Malformed));
        assert_eq!(Reader::new(&[5, 0, 0]).addr(), Err(Malformed));
        assert_eq!(Reader::new(&[4, 1, 2, 3]).addr(), Err(Malformed));
        // Longer than allowed, non-printable, truncated.
        assert_eq!(Reader::new(&[3, b'a', b'b', b'c']).text(2), Err(Malformed));
        assert_eq!(Reader::new(&[1, 0x07]).text(8), Err(Malformed));
        assert_eq!(Reader::new(&[5, b'a']).text(8), Err(Malformed));
        assert_eq!(Reader::new(&[1, 2]).finish(), Err(Malformed));
    }

    #[test]
    fn writer_text_always_decodes() {
        let mut w = Writer::default();
        w.text("héllo\n\u{7f}wörld and a lot more text than fits", 12);
        let data = w.into_vec();
        let text = Reader::new(&data).text(12).unwrap();
        assert_eq!(text.len(), 12);
        assert!(text.bytes().all(|b| (0x20..=0x7e).contains(&b)));
    }

    #[test]
    fn mapped_addresses_are_canonical() {
        let mapped: SocketAddr = "[::ffff:1.2.3.4]:9".parse().unwrap();
        let plain: SocketAddr = "1.2.3.4:9".parse().unwrap();
        assert_eq!(canonical(mapped), plain);
        assert_eq!(addr_bytes(mapped), addr_bytes(plain));
        assert_ne!(addr_bytes(plain), addr_bytes("1.2.3.4:10".parse().unwrap()));
        assert_eq!(ip_bytes(mapped.ip()), ip_bytes(plain.ip()));
    }
}
