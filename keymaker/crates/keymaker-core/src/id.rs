//! Unguessable identifiers.
//!
//! Handles are random 256-bit values rather than signed payloads: the broker
//! that issues them is the same process that redeems them, so there is nothing
//! to verify statelessly, and a value that carries no information cannot leak
//! any.

use std::fmt;
use std::io::Read;

/// Source of randomness, injectable so tests are deterministic.
pub trait Entropy: std::fmt::Debug {
    fn fill(&self, buf: &mut [u8]);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&self, buf: &mut [u8]) {
        let mut f = std::fs::File::open("/dev/urandom").expect("open /dev/urandom");
        f.read_exact(buf).expect("read /dev/urandom");
    }
}

/// Counter-based entropy for tests. Never use outside tests.
#[derive(Debug)]
pub struct SeqEntropy(std::cell::Cell<u64>);

impl SeqEntropy {
    pub fn new() -> Self {
        SeqEntropy(std::cell::Cell::new(0))
    }
}

impl Default for SeqEntropy {
    fn default() -> Self {
        Self::new()
    }
}

impl Entropy for SeqEntropy {
    fn fill(&self, buf: &mut [u8]) {
        let n = self.0.get();
        self.0.set(n + 1);
        buf.fill(0);
        let bytes = n.to_be_bytes();
        let len = buf.len();
        let take = len.min(bytes.len());
        buf[len - take..].copy_from_slice(&bytes[bytes.len() - take..]);
    }
}

/// A 256-bit opaque identifier, rendered as lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(String);

impl Id {
    pub fn generate(e: &dyn Entropy) -> Self {
        let mut raw = [0u8; 32];
        e.fill(&mut raw);
        let mut s = String::with_capacity(64);
        for b in raw {
            s.push_str(&format!("{:02x}", b));
        }
        Id(s)
    }

    /// Parse an id received over the wire. Rejects anything that is not
    /// exactly 64 lowercase hex characters, so a caller cannot smuggle
    /// structure into an identifier.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Some(Id(s.to_string()))
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// First 8 characters, for logs. Never enough to redeem.
    pub fn short(&self) -> &str {
        &self.0[..8]
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_64_hex_chars() {
        let id = Id::generate(&OsEntropy);
        assert_eq!(id.as_str().len(), 64);
        assert!(id.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn os_entropy_does_not_repeat() {
        let a = Id::generate(&OsEntropy);
        let b = Id::generate(&OsEntropy);
        assert_ne!(a, b);
    }

    #[test]
    fn seq_entropy_is_deterministic_and_distinct() {
        let e = SeqEntropy::new();
        let a = Id::generate(&e);
        let b = Id::generate(&e);
        assert_ne!(a, b);
        let e2 = SeqEntropy::new();
        assert_eq!(Id::generate(&e2), a, "same seed must replay");
    }

    #[test]
    fn parse_rejects_anything_that_is_not_plain_hex() {
        let good = "a".repeat(64);
        assert!(Id::parse(&good).is_some());
        assert!(Id::parse(&"A".repeat(64)).is_none(), "uppercase");
        assert!(Id::parse(&"a".repeat(63)).is_none(), "too short");
        assert!(Id::parse(&"a".repeat(65)).is_none(), "too long");
        assert!(
            Id::parse(&format!("{}../", "a".repeat(61))).is_none(),
            "path traversal"
        );
        assert!(Id::parse("").is_none());
    }

    #[test]
    fn short_form_is_a_prefix_and_not_redeemable() {
        let id = Id::generate(&OsEntropy);
        assert_eq!(id.short().len(), 8);
        assert!(id.as_str().starts_with(id.short()));
        assert!(Id::parse(id.short()).is_none());
    }
}
