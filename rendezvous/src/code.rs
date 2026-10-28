//! Join codes: 8 symbols of Crockford base32 (40 random bits), shown as `ABCD-EFGH`.
//!
//! A code is the key to a lobby that isn't listed publicly, so it comes from the OS random
//! source and the service limits how fast one address may try codes (docs/rendezvous.md
//! "Security").

use std::fmt;

/// Symbols in a code.
pub const CODE_LEN: usize = 8;

/// Crockford's base32: digits and letters without I, L, O and U.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A valid join code (always 8 symbols of [`ALPHABET`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Code([u8; CODE_LEN]);

impl Code {
    /// A new random code.
    pub fn random() -> Code {
        let bits = crate::crypto::random_bytes::<5>();
        let mut value = bits.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64);
        let mut out = [0u8; CODE_LEN];
        for slot in out.iter_mut().rev() {
            *slot = symbol((value & 31) as u8);
            value >>= 5;
        }
        Code(out)
    }

    /// What a player typed: case and dashes/spaces don't matter, and the letters people
    /// confuse with digits are read as those digits (O → 0, I and L → 1).
    pub fn parse(text: &str) -> Option<Code> {
        let mut out = [0u8; CODE_LEN];
        let mut n = 0;
        for c in text.chars() {
            if c == '-' || c.is_whitespace() {
                continue;
            }
            let c = match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            };
            let slot = out.get_mut(n)?;
            if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) {
                return None;
            }
            *slot = c as u8;
            n += 1;
        }
        (n == CODE_LEN).then_some(Code(out))
    }

    /// From the wire: exactly the 8 canonical symbols.
    pub fn from_bytes(bytes: [u8; CODE_LEN]) -> Option<Code> {
        bytes
            .iter()
            .all(|b| ALPHABET.contains(b))
            .then_some(Code(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; CODE_LEN] {
        &self.0
    }
}

fn symbol(v: u8) -> u8 {
    ALPHABET.get(v as usize & 31).copied().unwrap_or(b'0')
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Always ASCII (checked on construction).
        let s = std::str::from_utf8(&self.0).map_err(|_| fmt::Error)?;
        let (a, b) = s.split_at(CODE_LEN / 2);
        write!(f, "{a}-{b}")
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Code({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_is_forgiving_and_display_round_trips() {
        let code = Code::parse("abcd-efgh").unwrap();
        assert_eq!(code.to_string(), "ABCD-EFGH");
        assert_eq!(Code::parse(" AB CD EF GH "), Some(code));
        assert_eq!(Code::parse("0000-1111"), Code::parse("oOoO-iIlL"));
        assert_eq!(Code::parse(&code.to_string()), Some(code));
        assert_eq!(Code::parse("ABCD-EFG"), None, "too short");
        assert_eq!(Code::parse("ABCD-EFGHJ"), None, "too long");
        assert_eq!(Code::parse("ABCD-EFGU"), None, "U is not a symbol");
        assert_eq!(Code::parse("ABCD-EFGÉ"), None);
        assert_eq!(Code::from_bytes(*b"ABCDEFGH"), Some(code));
        assert_eq!(
            Code::from_bytes(*b"abcdefgh"),
            None,
            "the wire form is canonical"
        );
    }

    #[test]
    fn random_codes_are_valid_and_differ() {
        let codes: std::collections::BTreeSet<Code> = (0..256).map(|_| Code::random()).collect();
        assert_eq!(codes.len(), 256);
        for code in &codes {
            assert_eq!(Code::from_bytes(*code.as_bytes()), Some(*code));
        }
        // Every symbol position varies.
        for i in 0..CODE_LEN {
            let symbols: std::collections::BTreeSet<u8> =
                codes.iter().map(|c| c.as_bytes()[i]).collect();
            assert!(symbols.len() > 16, "position {i}: {symbols:?}");
        }
    }
}
