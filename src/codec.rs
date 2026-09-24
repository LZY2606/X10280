//! Bit-level LTC / VITC decode (and deterministic encode for the case pack).
//!
//! These are *documented, self-contained* bit layouts used by the workbench:
//! they follow the spirit of SMPTE-12M linear timecode and SMPTE ST 12-2
//! (VITC) BCD packing, but are intentionally simple so samples can be
//! hand-authored, corrupted, and audited bit-by-bit.
//!
//! LTC: 80 bits, sync word on bits 64..80 (LSB-first `0b0011111111111101`,
//! i.e. 0x3FFD). Each BCD digit is 4 bits LSB-first; the frame tens digit
//! only uses 2 value bits, bit 11 is the drop-frame flag, bit 10 color-frame.
//! User ("binary group") bits are 32 bits carried as 4 pairs of nibbles.
//!
//! VITC: 90 bits: two interleaved data bytes per timecode digit
//! (even byte = value nibble LSB-first, odd byte = parity/check bits), a
//! field bit, a leading and trailing `10` sync marker, and an 8-bit
//! even-parity word over the 80 data bits.
//!
//! Unknown / corrupted bits are represented in input strings as `x`.

use crate::smpte::{Rate, Timecode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodeOutput {
    pub tc: Timecode,
    pub user_bits: String, // 8 hex chars as carried, 'x' preserved
    pub bit_quality: f64,  // fraction 0..=1 of decisive/verified bits
    pub unknown_bits: Vec<usize>,
    pub warnings: Vec<String>,
    pub sync_ok: bool,
    pub parity_ok: bool,
}

fn bcd(v: u8) -> [bool; 4] {
    [v & 1 != 0, v & 2 != 0, v & 4 != 0, v & 8 != 0]
}

fn nibble_from(s: &str, lo: usize) -> (Option<u8>, Vec<usize>) {
    let mut unk = Vec::new();
    let mut val = 0u8;
    for k in 0..4 {
        match s.as_bytes()[lo + k] {
            b'1' => val |= 1 << k,
            b'0' => {}
            _ => unk.push(lo + k),
        }
    }
    (if unk.is_empty() { Some(val) } else { None }, unk)
}

// ---------------- LTC ----------------

const LTC_SYNC: [bool; 16] = [
    false, false, true, true, true, true, true, true, true, true, true, true, true, true, false, true,
];

pub fn encode_ltc(tc: &Timecode, user: u32) -> String {
    let mut b = vec!['0'; 80];
    let put = |b: &mut [char], lo: usize, nib: [bool; 4]| {
        for k in 0..4 {
            b[lo + k] = if nib[k] { '1' } else { '0' };
        }
    };
    put(&mut b, 0, bcd(tc.f % 10));
    put(&mut b, 8, bcd(tc.f / 10));
    b[10] = if tc.color_frame { '1' } else { '0' };
    b[11] = if tc.drop_frame { '1' } else { '0' };
    put(&mut b, 16, bcd(tc.s % 10));
    put(&mut b, 24, bcd(tc.s / 10));
    put(&mut b, 32, bcd(tc.m % 10));
    put(&mut b, 40, bcd(tc.m / 10));
    put(&mut b, 48, bcd(tc.h % 10));
    put(&mut b, 56, bcd(tc.h / 10));
    // user bits: nibbles at 4,12,20,28,36,44,52,60 (we spread a 32-bit word)
    for (i, lo) in [4usize, 12, 20, 28, 36, 44, 52, 60].iter().enumerate() {
        let nib = ((user >> (28 - (i * 4))) & 0xF) as u8;
        put(&mut b, *lo, bcd(nib));
    }
    for (k, v) in LTC_SYNC.iter().enumerate() {
        b[64 + k] = if *v { '1' } else { '0' };
    }
    b.into_iter().collect()
}

pub fn decode_ltc(s: &str) -> Result<DecodeOutput, String> {
    if s.len() != 80 {
        return Err(format!("LTC must be 80 bits, got {}", s.len()));
    }
    if !s.bytes().all(|c| matches!(c, b'0' | b'1' | b'x')) {
        return Err("LTC bits must be 0/1/x".into());
    }
    let mut unknown = Vec::new();
    let mut sync_ok = true;
    for (k, want) in LTC_SYNC.iter().enumerate() {
        match s.as_bytes()[64 + k] {
            b'1' if *want => {}
            b'0' if !*want => {}
            b'x' => {
                sync_ok = false;
                unknown.push(64 + k);
            }
            _ => sync_ok = false,
        }
    }
    let mut nibs = [0u8; 8];
    let mut nib_unk = Vec::new();
    for (i, lo) in [0usize, 8, 16, 24, 32, 40, 48, 56].iter().enumerate() {
        let (v, mut u) = nibble_from(s, *lo);
        nib_unk.append(&mut u);
        nibs[i] = v.unwrap_or(0);
    }
    unknown.extend(nib_unk.iter().copied());
    let f = nibs[0] + 10 * nibs[1];
    let sec = nibs[2] + 10 * nibs[3];
    let min = nibs[4] + 10 * nibs[5];
    let hr = nibs[6] + 10 * nibs[7];
    let df = s.as_bytes()[11] == b'1';
    let cf = s.as_bytes()[10] == b'1';
    if s.as_bytes()[10] == b'x' {
        unknown.push(10);
    }
    if s.as_bytes()[11] == b'x' {
        unknown.push(11);
    }
    let mut ub = String::new();
    for lo in [4usize, 12, 20, 28, 36, 44, 52, 60] {
        let (v, _) = nibble_from(s, lo);
        match v {
            Some(n) => ub.push_str(&format!("{:X}", n)),
            None => ub.push('x'),
        }
    }
    let mut warnings = Vec::new();
    if !sync_ok {
        warnings.push("sync word mismatch/unknown".into());
    }
    let total = 80usize;
    let bad = unknown.len();
    let bit_quality = (total - bad) as f64 / total as f64;
    Ok(DecodeOutput {
        tc: Timecode { h: hr, m: min, s: sec, f, drop_frame: df, color_frame: cf, field: None },
        user_bits: ub,
        bit_quality,
        unknown_bits: unknown,
        warnings,
        sync_ok,
        parity_ok: true,
    })
}

// ---------------- VITC ----------------
//
// Deterministic 90-bit layout:
//   bits 0..64  : eight 8-bit blocks. A block is 4 BCD value bits (LSB-first)
//                 followed by 4 complement check bits. Block order:
//                 funits, ftens, sunits, stens, munits, mtens, hunits, htens.
//   bit  64     : color-frame flag
//   bit  65     : drop-frame flag
//   bit  66     : field flag (0=first field, 1=second field)
//   bits 67..79 : 13 reserved bits (always 0)
//   bits 80..82 : 2 leading sync marker bits = 10
//   bits 82..90 : 8-bit even-parity word over bits 0..80 (81 bits), LSB-first
// The 32 user bits are folded into the *reserved* region as a documented
// workbench extension: bits 67..78 carry 12 user bits, and the remaining 20
// user bits are recovered losslessly via the reserved-zero invariant: to keep
// the codec simple and fully round-trippable we instead dedicate 32 whole
// bits by shortening reserved space. Final concrete map:
//   bits 67..98 would be needed, so the definitive layout below uses exactly
//   the documented constants VITC_* and 32 user bits at 67..99 is impossible
//   inside 90 bits. Therefore user bits for VITC are 12 bits (3 hex digits);
//   the other 5 hex digits of the record's user field are preserved verbatim
//   in the imported raw JSON and are *not* claimed to live on the wire.

const VITC_BLOCKS: usize = 8;
const VITC_CF: usize = 64;
const VITC_DF: usize = 65;
const VITC_FIELD: usize = 66;
const VITC_USER: usize = 67; // 12 bits
const VITC_USER_BITS: usize = 12;
const VITC_SYNC: usize = 79; // 10
const VITC_PARITY: usize = 81; // 8 bits through 88
const VITC_TAIL: usize = 89; // final marker 0
const VITC_LEN: usize = 90;

pub fn encode_vitc(tc: &Timecode, user: u32, field: bool) -> String {
    let mut b = vec!['0'; VITC_LEN];
    let digits = [
        tc.f % 10, tc.f / 10, tc.s % 10, tc.s / 10, tc.m % 10, tc.m / 10, tc.h % 10, tc.h / 10,
    ];
    for (i, d) in digits.iter().enumerate() {
        let lo = i * 8;
        let nib = bcd(*d);
        for k in 0..4 {
            b[lo + k] = if nib[k] { '1' } else { '0' };
            b[lo + 4 + k] = if nib[k] { '0' } else { '1' };
        }
    }
    b[VITC_CF] = if tc.color_frame { '1' } else { '0' };
    b[VITC_DF] = if tc.drop_frame { '1' } else { '0' };
    b[VITC_FIELD] = if field { '1' } else { '0' };
    for k in 0..VITC_USER_BITS {
        b[VITC_USER + k] = if (user >> k) & 1 == 1 { '1' } else { '0' };
    }
    b[VITC_SYNC] = '1';
    b[VITC_SYNC + 1] = '0';
    // even parity over bits 0..=VITC_SYNC+1 (81 bits)
    let mut p = 0u8;
    for i in 0..VITC_PARITY {
        if b[i] == '1' {
            p ^= 1;
        }
    }
    for k in 0..8 {
        b[VITC_PARITY + k] = if (p >> k) & 1 == 1 { '1' } else { '0' };
    }
    b[VITC_TAIL] = '0';
    b.into_iter().collect()
}

pub fn decode_vitc(s: &str) -> Result<DecodeOutput, String> {
    if s.len() != VITC_LEN {
        return Err(format!("VITC must be {VITC_LEN} bits, got {}", s.len()));
    }
    if !s.bytes().all(|c| matches!(c, b'0' | b'1' | b'x')) {
        return Err("VITC bits must be 0/1/x".into());
    }
    let mut warnings = Vec::new();
    let mut unknown = Vec::new();
    let mut nibs = [0u8; VITC_BLOCKS];
    for i in 0..VITC_BLOCKS {
        let lo = i * 8;
        let (v, _) = nibble_from(s, lo);
        let mut unk = Vec::new();
        match v {
            Some(val) => nibs[i] = val,
            None => {
                let (_, u) = nibble_from(s, lo);
                unk = u;
            }
        }
        unknown.append(&mut unk);
        let (cv, _) = nibble_from(s, lo + 4);
        if let (Some(val), Some(c)) = (v, cv) {
            if c != (!val & 0xF) {
                warnings.push(format!("check nibble mismatch at digit {i}"));
            }
        }
    }
    let f = nibs[0] + 10 * nibs[1];
    let sec = nibs[2] + 10 * nibs[3];
    let min = nibs[4] + 10 * nibs[5];
    let hr = nibs[6] + 10 * nibs[7];
    let cf = s.as_bytes()[VITC_CF] == b'1';
    let df = s.as_bytes()[VITC_DF] == b'1';
    let field = match s.as_bytes()[VITC_FIELD] {
        b'1' => Some(true),
        b'0' => Some(false),
        _ => {
            unknown.push(VITC_FIELD);
            None
        }
    };
    for i in [VITC_CF, VITC_DF] {
        if s.as_bytes()[i] == b'x' {
            unknown.push(i);
        }
    }
    let mut data_unknown = false;
    let mut p = 0u8;
    for i in 0..VITC_PARITY {
        match s.as_bytes()[i] {
            b'1' => p ^= 1,
            b'x' => data_unknown = true,
            _ => {}
        }
    }
    let (pw, _) = nibble_from(s, VITC_PARITY);
    let stored_p = pw.unwrap_or(0);
    let mut parity_ok = true;
    if !data_unknown && stored_p != p {
        parity_ok = false;
        warnings.push("VITC parity word mismatch".into());
    }
    let sync_ok = s.as_bytes()[VITC_SYNC] == b'1' && s.as_bytes()[VITC_SYNC + 1] == b'0';
    if !sync_ok {
        warnings.push("VITC sync marker mismatch".into());
    }
    let mut ub = String::new();
    for k in (0..VITC_USER_BITS).rev() {
        ub.push(match s.as_bytes()[VITC_USER + k] {
            b'1' => '1',
            b'x' => 'x',
            _ => '0',
        });
    }
    // render 12 user bits as 3 hex digits
    let mut user_hex = String::new();
    for grp in 0..3 {
        let bits4: String = (0..4)
            .map(|k| ub.chars().nth(grp * 4 + k).unwrap_or('0'))
            .collect();
        user_hex.push(match bits4.chars().all(|c| c != 'x') {
            true => char::from_digit(u8::from_str_radix(&bits4, 2).unwrap() as u32, 16).unwrap(),
            false => 'x',
        });
    }
    let bad = unknown.len() + if parity_ok { 0 } else { 8 } + if sync_ok { 0 } else { 2 };
    let bit_quality = (VITC_LEN - bad) as f64 / VITC_LEN as f64;
    Ok(DecodeOutput {
        tc: Timecode { h: hr, m: min, s: sec, f, drop_frame: df, color_frame: cf, field },
        user_bits: user_hex,
        bit_quality,
        unknown_bits: unknown,
        warnings,
        sync_ok,
        parity_ok,
    })
}

/// Decode based on declared media kind; framesummary/log have no bits.
pub fn decode(kind: &str, bits: Option<&str>, _rate: Rate) -> Result<Option<DecodeOutput>, String> {
    match (kind, bits) {
        ("ltc", Some(b)) if !b.is_empty() => Ok(Some(decode_ltc(b)?)),
        ("vitc", Some(b)) if !b.is_empty() => Ok(Some(decode_vitc(b)?)),
        ("ltc", _) | ("vitc", _) => Err(format!("{kind} record requires a non-empty bits string")),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ltc_roundtrip_and_corruption() {
        let tc = Timecode::label(12, 34, 56, 27, true, true);
        let bits = encode_ltc(&tc, 0x12AB34CD);
        let d = decode_ltc(&bits).unwrap();
        assert_eq!(d.tc, tc);
        assert_eq!(d.user_bits, "12AB34CD");
        assert!((d.bit_quality - 1.0).abs() < 1e-9);
        assert!(d.sync_ok);
        let mut corrupt = bits.clone();
        corrupt.replace_range(0..1, "x");
        corrupt.replace_range(70..71, "1"); // break sync
        let d2 = decode_ltc(&corrupt).unwrap();
        assert!(d2.bit_quality < 1.0);
        assert!(!d2.sync_ok);
        assert!(d2.unknown_bits.contains(&0));
    }

    #[test]
    fn vitc_roundtrip_and_field() {
        let tc = Timecode::label(3, 15, 42, 11, false, false);
        let bits = encode_vitc(&tc, 0xABC, true);
        let d = decode_vitc(&bits).unwrap();
        assert_eq!(d.tc, tc);
        assert_eq!(d.tc.field, Some(true));
        assert_eq!(d.user_bits, "ABC");
        assert!(d.parity_ok);
        let mut bad = bits.clone();
        bad.replace_range(2..3, "1"); // flip a data bit without fixing parity
        let d2 = decode_vitc(&bad).unwrap();
        assert!(!d2.parity_ok);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(decode_ltc(&"0".repeat(79)).is_err());
        assert!(decode_vitc(&"1".repeat(91)).is_err());
    }
}
