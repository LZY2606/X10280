// Core domain value types. All quantities carry their unit/coordinate basis;
// original textual representations are retained by the importer.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Frame rate as a rational number of frames per second.
/// `raw` preserves the original spelling (e.g. "30000/1001"); `num`/`den`
/// carry the parsed integer fraction. Frame count is the unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rate {
    pub num: u64,
    pub den: u64,
    #[serde(rename = "raw")]
    pub raw: String,
}

impl Rate {
    pub fn parse(text: &str) -> Result<Rate, String> {
        let text = text.trim();
        let (num, den) = if let Some((a, b)) = text.split_once('/') {
            (
                a.trim().parse::<u64>().map_err(|e| format!("bad rate numerator: {e}"))?,
                b.trim().parse::<u64>().map_err(|e| format!("bad rate denominator: {e}"))?,
            )
        } else {
            (text.parse::<u64>().map_err(|e| format!("bad rate: {e}"))?, 1u64)
        };
        if den == 0 {
            return Err("rate denominator is zero".into());
        }
        if num == 0 {
            return Err("rate numerator is zero".into());
        }
        Ok(Rate { num, den, raw: text.to_string() })
    }

    /// Nominal integer frame rate, nearest integer (30 for 30000/1001).
    pub fn nominal(&self) -> u64 {
        (self.num + self.den / 2) / self.den
    }

    /// SMPTE drop frame is defined for fractional rates rounding to 30/60.
    pub fn drop_frame_supported(&self) -> bool {
        let d = self.nominal();
        (d == 30 || d == 60) && self.num % self.den != 0
    }

    /// Real elapsed time basis: seconds per frame, as a rational (num, den) of seconds.
    pub fn seconds_per_frame(&self) -> (u64, u64) {
        (self.den, self.num)
    }
}

/// SMPTE timecode label HH:MM:SS:FF. Separator is carried by the raw input;
/// `drop` is the parsed drop-frame flag (`;`/`.` implies drop frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timecode {
    pub h: u8,
    pub m: u8,
    pub s: u8,
    pub f: u8,
}

impl Timecode {
    pub fn parse(text: &str) -> Result<(Timecode, bool, char), String> {
        let t = text.trim();
        let mut chars = t.chars();
        let mut nums: Vec<u8> = Vec::new();
        let mut seps: Vec<char> = Vec::new();
        let mut cur = String::new();
        while let Some(c) = chars.next() {
            if c.is_ascii_digit() {
                cur.push(c);
            } else if c == ':' || c == ';' || c == '.' || c == ',' || c == '|' {
                if cur.is_empty() {
                    return Err(format!("malformed timecode `{t}`"));
                }
                nums.push(cur.parse().map_err(|_| format!("malformed timecode `{t}`"))?);
                cur.clear();
                seps.push(c);
            } else {
                return Err(format!("unexpected character `{c}` in timecode `{t}`"));
            }
        }
        if cur.is_empty() {
            return Err(format!("malformed timecode `{t}`"));
        }
        nums.push(cur.parse().map_err(|_| format!("malformed timecode `{t}`"))?);
        if nums.len() != 4 {
            return Err(format!("timecode `{t}` must have H:M:S:F"));
        }
        let tc = Timecode { h: nums[0], m: nums[1], s: nums[2], f: nums[3] };
        let frame_sep = seps.get(2).copied().unwrap_or(':');
        let drop = seps.iter().any(|c| *c == ';' || *c == '.' || *c == '|')
            || frame_sep == ';';
        Ok((tc, drop, frame_sep))
    }

    pub fn format(&self, drop: bool) -> String {
        format!(
            "{:02}:{:02}:{:02}{}{:02}",
            self.h,
            self.m,
            self.s,
            if drop { ';' } else { ':' },
            self.f
        )
    }
}

/// Field flag for VITC/interlaced material. `Unset` is preserved distinctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Field {
    Unset,
    First,
    Second,
}

/// One decoded/imported frame reading. Every field is optional at the type
/// level because LTC, VITC, video summaries and logs carry different subsets;
/// absent fields stay absent rather than being silently zeroed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reading {
    pub ordinal: u64,
    /// Original 80-bit LTC / 90-bit VITC bit string, "" when the source has none.
    #[serde(rename = "bitsRaw")]
    pub bits_raw: String,
    /// Original timecode spelling as submitted ("" if absent).
    #[serde(rename = "tcRaw")]
    pub tc_raw: String,
    pub tc: Option<Timecode>,
    /// Drop-frame flag as stated/spelled; independent of rate validation.
    #[serde(rename = "tcDropSpelled")]
    pub tc_drop_spelled: Option<bool>,
    /// Hex user bits exactly as spelled ("" if absent).
    #[serde(rename = "ubRaw")]
    pub ub_raw: String,
    #[serde(rename = "colorFrame")]
    pub color_frame: Option<bool>,
    pub field: Field,
    /// Frame fingerprint exactly as spelled ("" if absent).
    #[serde(rename = "fpRaw")]
    pub fp_raw: String,
    /// Device clock exactly as spelled ("" if absent). Unit: ISO8601 wall time.
    #[serde(rename = "clockRaw")]
    pub clock_raw: String,
    /// Bit quality 0.0..=1.0 as spelled; unit: dimensionless fraction.
    pub quality: Option<f64>,
}

/// Stable content identifier (hex SHA-256 over canonical JSON of a salted tag).
pub fn stable_id(tag: &str, value: &serde_json::Value) -> String {
    let canonical = canonical_json(value);
    let mut h = Sha256::new();
    h.update(tag.as_bytes());
    h.update(0u8.to_le_bytes());
    h.update(canonical.as_bytes());
    format!("{tag}_{}", &hex::encode(h.finalize())[..24])
}

pub fn hash_json(value: &serde_json::Value) -> String {
    let canonical = canonical_json(value);
    let mut h = Sha256::new();
    h.update(canonical.as_bytes());
    hex::encode(h.finalize())
}

/// Deterministic JSON serialization: object keys sorted recursively.
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), canonical_json(&map[*k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        other => other.to_string(),
    }
}
