//! Raw SMPTE LTC/VITC bit decoding.
//!
//! The workbench preserves the *original bits* exactly as imported. Decoding is
//! a derived operation that reports which bits were confident, uncertain ('x'),
//! or inconsistent (illegal BCD). Nothing is silently repaired.
//!
//! Bit positions below follow SMPTE 12-1 (LTC) and SMPTE 12-2 (VITC). LTC is
//! 80 bits per frame; VITC is 90 bits per frame (two sync words + CRC).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Wire {
    Ltc,
    Vitc,
}

impl Wire {
    pub fn parse(s: &str) -> Result<Wire, String> {
        match s.trim().to_ascii_uppercase().as_str() {
            "LTC" => Ok(Wire::Ltc),
            "VITC" => Ok(Wire::Vitc),
            other => Err(format!("wire kind must be LTC or VITC, got {other:?}")),
        }
    }
    pub fn bit_len(self) -> usize {
        match self {
            Wire::Ltc => 80,
            Wire::Vitc => 90,
        }
    }
}

/// One decoded frame, retaining all SMPTE 12 fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoded {
    pub wire: Wire,
    pub bits: String,
    pub confident: Vec<bool>,
    pub units_frames: u8,
    pub tens_frames: u8,
    pub units_seconds: u8,
    pub tens_seconds: u8,
    pub units_minutes: u8,
    pub tens_minutes: u8,
    pub units_hours: u8,
    pub tens_hours: u8,
    /// Eight user bits (4-bit nibbles), positions 4..80.
    pub user: [u8; 8],
    pub drop_frame: bool,
    pub color_frame: bool,
    /// VITC field mark (LTC has no field mark; stored false).
    pub field_mark: bool,
    pub bgf0: bool,
    pub bgf1: bool,
    pub parity: bool,
    pub sync_ok: bool,
    pub diagnostics: Vec<String>,
}

impl Decoded {
    pub fn frame_bcd(&self) -> u8 { self.tens_frames * 10 + self.units_frames }
    pub fn seconds_bcd(&self) -> u8 { self.tens_seconds * 10 + self.units_seconds }
    pub fn minutes_bcd(&self) -> u8 { self.tens_minutes * 10 + self.units_minutes }
    pub fn hours_bcd(&self) -> u8 { self.tens_hours * 10 + self.units_hours }

    /// 0.0..=1.0 share of bits read confidently.
    pub fn quality(&self) -> f64 {
        let good = self.confident.iter().filter(|c| **c).count();
        good as f64 / self.confident.len() as f64
    }
}

fn nibble(bits: &[u8], confident: &[bool], positions: [usize; 4]) -> (u8, bool, Vec<usize>) {
    let mut v = 0u8;
    let mut all_conf = true;
    let mut uncertain = Vec::new();
    for (i, &p) in positions.iter().enumerate() {
        match bits[p] {
            b'1' => v |= 1 << i,
            b'0' => {}
            _ => {
                all_conf = false;
                uncertain.push(p);
            }
        }
        if !confident[p] {
            all_conf = false;
            if !uncertain.contains(&p) {
                uncertain.push(p);
            }
        }
    }
    (v, all_conf, uncertain)
}

fn valid_bcd(v: u8, max: u8) -> bool {
    (v & 0x0f) <= 9 && (v >> 4) <= max
}

fn bit_at(bits: &[u8], confident: &[bool], p: usize) -> Option<bool> {
    if !confident[p] {
        return None;
    }
    Some(bits[p] == b'1')
}

/// Decode a raw bit string. `bits` uses '0'/'1' for confident reads and 'x'
/// (or any other char) for an uncertain/errored bit.
pub fn decode(wire: Wire, bits: &str) -> Result<Decoded, String> {
    let want = wire.bit_len();
    let clean: Vec<u8> = bits.chars().map(|c| c as u8).collect();
    if clean.len() != want {
        return Err(format!(
            "{wire:?} expects {want} bits, got {len}",
            len = clean.len()
        ));
    }
    let mut confident = Vec::with_capacity(want);
    for &c in &clean {
        confident.push(matches!(c, b'0' | b'1'));
    }

    let mut d = Decoded {
        wire,
        bits: bits.to_string(),
        confident,
        units_frames: 0,
        tens_frames: 0,
        units_seconds: 0,
        tens_seconds: 0,
        units_minutes: 0,
        tens_minutes: 0,
        units_hours: 0,
        tens_hours: 0,
        user: [0; 8],
        drop_frame: false,
        color_frame: false,
        field_mark: false,
        bgf0: false,
        bgf1: false,
        parity: false,
        sync_ok: false,
        diagnostics: Vec::new(),
    };
    let b = &clean;
    let c = &d.confident;

    let (u, _, _) = nibble(b, c, [0, 1, 2, 3]);
    d.units_frames = u;
    let (t, _, _) = nibble(b, c, [8, 9]);
    d.tens_frames = t;
    let (u, _, _) = nibble(b, c, [16, 17, 18, 19]);
    d.units_seconds = u;
    let (t, _, _) = nibble(b, c, [24, 25, 26]);
    d.tens_seconds = t;
    let (u, _, _) = nibble(b, c, [32, 33, 34, 35]);
    d.units_minutes = u;
    let (t, _, _) = nibble(b, c, [40, 41, 42]);
    d.tens_minutes = t;
    let (u, _, _) = nibble(b, c, [48, 49, 50, 51]);
    d.units_hours = u;
    let (t, _, _) = nibble(b, c, [56, 57]);
    d.tens_hours = t;

    let user_pos: [[usize; 4]; 8] = [
        [4, 5, 6, 7],
        [12, 13, 14, 15],
        [20, 21, 22, 23],
        [28, 29, 30, 31],
        [36, 37, 38, 39],
        [44, 45, 46, 47],
        [52, 53, 54, 55],
        [60, 61, 62, 63],
    ];
    for (i, pos) in user_pos.iter().enumerate() {
        let (v, _, _) = nibble(b, c, *pos);
        d.user[i] = v;
    }

    match wire {
        Wire::Ltc => {
            d.drop_frame = bit_at(b, c, 10).unwrap_or(false);
            d.color_frame = bit_at(b, c, 11).unwrap_or(false);
            // Binary group flags: bit 27 = BGF0, bit 43 = BGF1, bit 58 = BGF2.
            d.bgf0 = bit_at(b, c, 27).unwrap_or(false);
            d.bgf1 = bit_at(b, c, 43).unwrap_or(false);
            d.parity = bit_at(b, c, 27).unwrap_or(false);
            // Sync word bits 64..79 = 0011 1111 1111 1101 (0x3FFD, LSB first).
            let mut sync = String::new();
            for p in 64..80 {
                sync.push(if c[p] { b[p] as char } else { 'x' });
            }
            d.sync_ok = sync == "0011111111111101";
        }
        Wire::Vitc => {
            // SMPTE 12-2 layout differences.
            d.drop_frame = bit_at(b, c, 10).unwrap_or(false);
            d.color_frame = bit_at(b, c, 11).unwrap_or(false);
            // VITC field-mark (which field this is); no equivalent in LTC.
            d.field_mark = bit_at(b, c, 26).unwrap_or(false);
            d.bgf0 = bit_at(b, c, 27).unwrap_or(false);
            d.bgf1 = bit_at(b, c, 43).unwrap_or(false);
            // VITC carries two 8-bit sync words (0x9B, LSB-first ordering here).
            let s1: String = (0..8).map(|p| if c[p] { b[p] as char } else { 'x' }).collect();
            let s2: String = (82..90).map(|p| if c[p] { b[p] as char } else { 'x' }).collect();
            d.sync_ok = s1 == "10011011" && s2 == "10011011";
        }
    }

    // BCD validation diagnostics (value retained in place; not dropped).
    let checks = [
        (d.units_frames, 2u8, "frame-units"),
        (d.tens_frames, 0, "frame-tens"),
        (d.units_seconds, 5, "second-units"),
        (d.tens_seconds, 0, "second-tens"),
        (d.units_minutes, 5, "minute-units"),
        (d.tens_minutes, 0, "minute-tens"),
        (d.units_hours, 3, "hour-units"),
        (d.tens_hours, 0, "hour-tens"),
    ];
    for (v, hi, name) in checks {
        if !valid_bcd(v, hi) {
            d.diagnostics
                .push(format!("illegal BCD in {name}: 0x{v:X} retained, not normalised"));
        }
    }
    let uncertain_count = d.confident.iter().filter(|c| !**c).count();
    if uncertain_count > 0 {
        d.diagnostics
            .push(format!("{uncertain_count} uncertain bit(s) read as 0 with quality loss"));
    }
    if !d.sync_ok {
        d.diagnostics.push("sync word missing or corrupt".to_string());
    }

    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(mut bits: Vec<char>, pos: &[usize], val: &[char]) -> String {
        for (p, v) in pos.iter().zip(val) {
            bits[*p] = *v;
        }
        bits.into_iter().collect()
    }

    #[test]
    fn decodes_ltc_minute_boundary() {
        // 00:01:00;02 with DF bit set, proper sync.
        let mut bits = vec!['0'; 80];
        // units frames = 2 -> bits 1
        bits[1] = '1';
        // tens frames 0
        // DF bit 10 = 1, color frame bit 11 = 1
        bits[10] = '1';
        bits[11] = '1';
        // units minutes = 1 -> bit 32
        bits[32] = '1';
        // user nibble 3 = 0xA -> bits 28,30
        bits[28] = '1';
        bits[30] = '1';
        for p in 64..80 {
            bits[p] = "0011111111111101".as_bytes()[p - 64] as char;
        }
        let s: String = bits.iter().collect();
        let d = decode(Wire::Ltc, &s).unwrap();
        assert_eq!(d.frame_bcd(), 2);
        assert_eq!(d.minutes_bcd(), 1);
        assert!(d.drop_frame);
        assert!(d.color_frame);
        assert_eq!(d.user[3], 0xA);
        assert!(d.sync_ok);
        assert_eq!(d.quality(), 1.0);
    }

    #[test]
    fn uncertain_bits_lower_quality_and_diagnose() {
        let mut s = "0".repeat(80);
        s.replace_range(0..1, "x");
        let d = decode(Wire::Ltc, &s).unwrap();
        assert!(d.quality() < 1.0);
        assert!(!d.diagnostics.is_empty());
    }

    #[test]
    fn wrong_length_is_hard_error() {
        assert!(decode(Wire::Ltc, &"0".repeat(79)).is_err());
        assert!(decode(Wire::Vitc, &"1".repeat(90)).is_ok());
    }

    #[test]
    fn illegal_bcd_is_retained_with_diagnostic() {
        let mut bits = vec!['0'; 80];
        bits[2] = '1';
        bits[3] = '1'; // units frames = 12 (illegal)
        let s: String = bits.iter().collect();
        let d = decode(Wire::Ltc, &s).unwrap();
        assert_eq!(d.units_frames, 0b1100);
        assert!(d.diagnostics.iter().any(|x| x.contains("illegal BCD")));
    }
}
