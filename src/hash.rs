//! Deterministic identity hashing (FNV-1a, 64-bit).
//!
//! Identities must be identical across machines, rebuilds and export/import
//! round trips, so we never use randomness or process-local state for ids.

/// FNV-1a 64-bit over the exact bytes supplied.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Stable short identity: `<prefix>_<12 hex chars of FNV-1a>`.
pub fn id(prefix: &str, material: &str) -> String {
    let h = fnv1a64(material.as_bytes());
    format!("{prefix}_{:012x}", h & 0x000f_ffff_ffff_ffff)
}
