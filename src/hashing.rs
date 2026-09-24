//! Deterministic stable identities.
//!
//! IDs are content-derived (FNV-1a 64-bit, hex) so that:
//! * re-running the same replay from the same inputs reproduces the *same* ids,
//! * export → delete DB → import keeps identity/order/source relations,
//! * no randomness is needed for domain object identity.

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Default, Clone)]
pub struct Hasher {
    state: u64,
}

impl Hasher {
    pub fn new() -> Self {
        Hasher { state: OFFSET }
    }
    pub fn feed(&mut self, bytes: &[u8]) -> &mut Self {
        for &b in bytes {
            self.state ^= b as u64;
            self.state = self.state.wrapping_mul(PRIME);
        }
        self
    }
    pub fn feed_str(&mut self, s: &str) -> &mut Self {
        self.feed(s.as_bytes())
    }
    pub fn feed_sep(&mut self) -> &mut Self {
        self.feed(&[0x1f])
    }
    pub fn finish(&self) -> u64 {
        self.state
    }
    pub fn id(parts: &[&str]) -> String {
        let mut h = Hasher::new();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                h.feed_sep();
            }
            h.feed_str(p);
        }
        format!("{:016x}", h.finish())
    }
}

/// Fingerprint of canonical JSON-ish parameter content.
pub fn fingerprint(parts: &[&str]) -> String {
    Hasher::id(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_and_distinct() {
        assert_eq!(Hasher::id(&["a", "b"]), Hasher::id(&["a", "b"]));
        assert_ne!(Hasher::id(&["a", "b"]), Hasher::id(&["a", "c"]));
        assert_ne!(Hasher::id(&["ab"]), Hasher::id(&["a", "b"]));
    }
}
