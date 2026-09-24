//! Deterministic, content-addressed identifiers.
//!
//! IDs are derived solely from canonical content so that export -> delete
//! -> import into a fresh database preserves every identity, ordering and
//! provenance relation. No randomness and no wall clock participate in IDs.

use sha2::{Digest, Sha256};

pub fn digest(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

pub fn id(prefix: &str, parts: &[&[u8]]) -> String {
    let full = digest(parts);
    format!("{}_{}", prefix, &full[..20])
}

pub fn of_json(prefix: &str, value: &serde_json::Value) -> String {
    let canon = canonical(value);
    id(prefix, &[canon.as_bytes()])
}

/// Canonical JSON encoding with sorted object keys and no insignificant
/// whitespace, so equal values always hash equally.
pub fn canonical(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                out.push_str(&canonical(&map[*key]));
            }
            out.push('}');
            out
        }
        serde_json::Value::Array(items) => {
            let mut out = String::from("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&canonical(item));
            }
            out.push(']');
            out
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}
