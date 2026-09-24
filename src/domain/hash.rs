//! Deterministic FNV-1a 64 bit hashing used for stable content identities.
//! No randomness and no platform-dependent behaviour.

pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Stable short identity token: `prefix_` + 12 hex chars.
pub fn short_id(prefix: &str, material: &str) -> String {
    format!("{prefix}_{:012x}", fnv1a_64(material.as_bytes()) & 0x000f_ffff_ffff_ffff)
}
