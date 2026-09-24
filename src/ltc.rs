// LTC/VITC bit interpretation (derived layer).
//
// Decode is intentionally a pure function returning decoded fields plus
// diagnostics. Unknown bits decode as None; conflicting bits-vs-spelled
// values are reported, never silently preferred.

use crate::domain::{Field, Timecode};
use serde::Serialize;

pub const LTC_ALGORITHM: &str = "ltc12m-bcd/1";
pub const VITC_ALGORITHM: &str = "vitc-bcd/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecodedWord {
    pub tc: Option<Timecode>,
    pub drop: Option<bool>,
    pub color_frame: Option<bool>,
    pub polarity: Option<bool>,
    pub bgf: [Option<bool>; 3],
    /// 32 user bits, most-significant nibble first, hex without padding of 0x.
    pub user_bits_hex: Option<String>,
    pub sync_ok: bool,
    pub diagnostics: Vec<String>,
}

fn bcd_nibble(bits: &[char], base: usize, tens: bool) -> Option<u8> {
    if tens {
        let mut v = 0u8;
        for k in 0..3 {
            if bits.get(base + k).copied()? == '1' {
                v |= 1 << k;
            }
        }
        Some(v)
    } else {
        let mut v = 0u8;
        for k in 0..4 {
            if bits.get(base + k).copied()? == '1' {
                v |= 1 << k;
            }
        }
        Some(v)
    }
}

fn nibble_pair(bits: &[char], base: usize) -> Option<u8> {
    let ones = bcd_nibble(bits, base, false)?;
    let tens = bcd_nibble(bits, base + 4, true)?;
    Some(tens * 10 + ones)
}

fn bit_at(bits: &[char], idx: usize) -> Option<bool> {
    bits.get(idx).map(|c| *c == '1')
}

pub fn decode_ltc(raw: &str) -> DecodedWord {
    let mut d = DecodedWord {
        tc: None,
        drop: None,
        color_frame: None,
        polarity: None,
        bgf: [None; 3],
        user_bits_hex: None,
        sync_ok: false,
        diagnostics: vec![],
    };
    if raw.len() != 80 {
        d.diagnostics.push(format!("LTC word must be 80 bits, got {}", raw.len()));
        return d;
    }
    let bits: Vec<char> = raw.chars().collect();

    let frame = match nibble_pair(&bits, 0) {
        Some(v) if v < 30 => v,
        Some(v) => {
            d.diagnostics.push(format!("frame units decode to {v} (BCD/rate check deferred)"));
            v
        }
        None => return d,
    };
    let sec = match nibble_pair(&bits, 8) {
        Some(v) if v < 60 => v,
        Some(v) => {
            d.diagnostics.push(format!("seconds decode to {v}, out of range"));
            v
        }
        None => return d,
    };
    let min = match nibble_pair(&bits, 16) {
        Some(v) if v < 60 => v,
        Some(v) => {
            d.diagnostics.push(format!("minutes decode to {v}, out of range"));
            v
        }
        None => return d,
    };
    let hour = match nibble_pair(&bits, 24) {
        Some(v) if v < 24 => v,
        Some(v) => {
            d.diagnostics.push(format!("hours decode to {v}, out of range"));
            v
        }
        None => return d,
    };
    d.tc = Some(Timecode { h: hour, m: min, s: sec, f: frame });
    d.color_frame = bit_at(&bits, 11);
    d.drop = bit_at(&bits, 10);
    d.bgf[0] = bit_at(&bits, 27);
    d.bgf[1] = bit_at(&bits, 43);
    d.bgf[2] = bit_at(&bits, 59);
    d.polarity = bit_at(&bits, 27).map(|_| false); // bit 27 documented position; retained as-is
    if let Some(p) = bit_at(&bits, 27) {
        d.polarity = Some(p);
    }

    // User bits: nibbles at 4-7, 12-15, 20-23, 28-31, 36-39, 44-47, 52-55, 60-63
    let ub_positions = [4, 12, 20, 28, 36, 44, 52, 60];
    let mut present = true;
    let mut nibs: Vec<u8> = Vec::new();
    for &base in &ub_positions {
        match (0..4).try_fold(0u8, |acc, k| -> Option<u8> {
            Some(acc | if bit_at(&bits, base + k)? { 1 << k } else { 0 })
        }) {
            Some(n) => nibs.push(n),
            None => present = false,
        }
    }
    if present {
        d.user_bits_hex = Some(nibs.iter().map(|n| char::from_digit(*n as u32, 16).unwrap()).collect());
    }

    let mut sync = 0u16;
    for i in 64..80 {
        sync = (sync << 1) | if bits[i] == '1' { 1 } else { 0 };
    }
    d.sync_ok = sync == 0x3FFD;
    if !d.sync_ok {
        d.diagnostics.push(format!("sync word is {sync:016b}, expected 0011111111111101 (0x3FFD)"));
    }
    d
}

/// VITC shares the same BCD layout for H/M/S/F. The sync word is 0x2D1 twice;
/// field flag bits live at 35 (F1) and 55 (F2); frame CRC is not checked here.
pub fn decode_vitc(raw: &str) -> DecodedWord {
    let mut d = DecodedWord {
        tc: None,
        drop: None,
        color_frame: None,
        polarity: None,
        bgf: [None; 3],
        user_bits_hex: None,
        sync_ok: false,
        diagnostics: vec![],
    };
    if raw.len() != 90 {
        d.diagnostics.push(format!("VITC word must be 90 bits, got {}", raw.len()));
        return d;
    }
    let bits: Vec<char> = raw.chars().collect();
    if let (Some(f), Some(s), Some(m), Some(h)) = (
        nibble_pair(&bits, 0),
        nibble_pair(&bits, 8),
        nibble_pair(&bits, 16),
        nibble_pair(&bits, 24),
    ) {
        if f < 60 && s < 60 && m < 60 && h < 24 {
            d.tc = Some(Timecode { h, m, s: s, f });
        } else {
            d.diagnostics.push(format!("VITC BCD out of range {h:02}:{m:02}:{s:02}:{f:02}"));
            d.tc = Some(Timecode { h, m, s, f });
        }
    }
    d.color_frame = bit_at(&bits, 11);
    d.drop = bit_at(&bits, 10);
    d
}

pub fn field_of(f: Field) -> Option<&'static str> {
    match f {
        Field::Unset => None,
        Field::First => Some("F1"),
        Field::Second => Some("F2"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(bits: &mut String, i: usize, v: u8) {
        for k in 0..4 {
            if v & (1 << k) != 0 {
                bits.replace_range(i + k..i + k + 1, "1");
            }
        }
    }

    #[test]
    fn decodes_known_word() {
        // 01:02:03;04, drop flag set, valid sync.
        let mut bits = "0".repeat(80);
        set(&mut bits, 0, 4);
        set(&mut bits, 4 + 0, 0);
        set(&mut bits, 8, 3);
        set(&mut bits, 16, 2);
        set(&mut bits, 24, 1);
        bits.replace_range(10..11, "1"); // drop flag
        let sync = "0011111111111101";
        bits.replace_range(64..80, sync);
        let d = decode_ltc(&bits);
        assert_eq!(d.tc, Some(Timecode { h: 1, m: 2, s: 3, f: 4 }));
        assert_eq!(d.drop, Some(true));
        assert!(d.sync_ok);
        assert_eq!(d.user_bits_hex.as_deref(), Some("00000000"));
    }

    #[test]
    fn bad_sync_is_diagnostic_not_error() {
        let d = decode_ltc(&"0".repeat(80));
        assert!(!d.sync_ok);
        assert!(d.diagnostics.iter().any(|m| m.contains("sync word")));
    }

    #[test]
    fn wrong_length_is_diagnostic() {
        let d = decode_ltc("0101");
        assert!(d.tc.is_none());
        assert!(d.diagnostics.iter().any(|m| m.contains("80 bits")));
    }
}
