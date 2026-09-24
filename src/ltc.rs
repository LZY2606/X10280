//! LTC (SMPTE 12M) 80-bit frame encode/decode.
//! Bit layout (LSB-first within the 80-bit word):
//!   0-3 frame units, 4-7 user1, 8-9 frame tens, 10 drop flag, 11 color flag,
//!  12-15 user2, 16-19 sec units, 20-23 user3, 24-26 sec tens, 27 field mark,
//!  28-31 user4, 32-35 min units, 36-39 user5, 40-42 min tens, 43 bgf0,
//!  44-47 user6, 48-51 hour units, 52-55 user7, 56-57 hour tens, 58 bgf1,
//!  59 biphase correction, 60-63 user8, 64-79 sync word 0x3FFD (LSB-first).
use crate::timecode::Timecode;
use serde::{Deserialize, Serialize};

pub const SYNC_WORD: u16 = 0x3FFD;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LtcFrame {
    pub tc: Timecode,
    pub drop_frame: bool,
    pub color_frame: bool,
    pub field_mark: bool,
    pub bgf0: bool,
    pub bgf1: bool,
    pub biphase_correction: bool,
    /// Eight 4-bit user groups packed into 32 bits, group 1 in the low nibble.
    pub user_bits: u32,
}

fn get(w: u128, bit: u8) -> bool {
    (w >> bit) & 1 == 1
}
fn getn(w: u128, bit: u8, n: u8) -> u64 {
    ((w >> bit) & ((1u128 << n) - 1)) as u64
}
fn setn(w: &mut u128, bit: u8, n: u8, v: u64) {
    *w |= ((v as u128) & ((1u128 << n) - 1)) << bit;
}
fn setb(w: &mut u128, bit: u8, v: bool) {
    if v {
        *w |= 1u128 << bit;
    }
}

fn tens(v: u8) -> u8 {
    v / 10
}
fn units(v: u8) -> u8 {
    v % 10
}

pub fn encode(f: &LtcFrame) -> u128 {
    let mut w: u128 = 0;
    setn(&mut w, 0, 4, units(f.tc.f) as u64);
    setn(&mut w, 8, 2, tens(f.tc.f) as u64);
    setb(&mut w, 10, f.drop_frame);
    setb(&mut w, 11, f.color_frame);
    setn(&mut w, 16, 4, units(f.tc.s) as u64);
    setn(&mut w, 24, 3, tens(f.tc.s) as u64);
    setb(&mut w, 27, f.field_mark);
    setn(&mut w, 32, 4, units(f.tc.m) as u64);
    setn(&mut w, 40, 3, tens(f.tc.m) as u64);
    setb(&mut w, 43, f.bgf0);
    setn(&mut w, 48, 4, units(f.tc.h) as u64);
    setn(&mut w, 56, 2, tens(f.tc.h) as u64);
    setb(&mut w, 58, f.bgf1);
    setb(&mut w, 59, f.biphase_correction);
    for g in 0..8u8 {
        let nib = ((f.user_bits >> (4 * g)) & 0xF) as u64;
        setn(&mut w, 4 + 8 * g, 4, nib);
    }
    setn(&mut w, 64, 16, SYNC_WORD as u64);
    w
}

/// Decode an 80-bit LTC word. Returns Err with a diagnostic if the sync word
/// does not match or a digit is out of range; nothing is silently dropped.
pub fn decode(w: u128) -> Result<LtcFrame, String> {
    let sync = getn(w, 64, 16) as u16;
    if sync != SYNC_WORD {
        return Err(format!("sync word mismatch: got 0x{sync:04X}, want 0x{SYNC_WORD:04X}"));
    }
    let fu = getn(w, 0, 4) as u8;
    let ft = getn(w, 8, 2) as u8;
    let su = getn(w, 16, 4) as u8;
    let st = getn(w, 24, 3) as u8;
    let mu = getn(w, 32, 4) as u8;
    let mt = getn(w, 40, 3) as u8;
    let hu = getn(w, 48, 4) as u8;
    let ht = getn(w, 56, 2) as u8;
    if fu > 9 || su > 9 || mu > 9 || hu > 9 {
        return Err(format!(
            "BCD digit out of range: f{fu} s{su} m{mu} h{hu}"
        ));
    }
    let mut user_bits: u32 = 0;
    for g in 0..8u8 {
        let nib = getn(w, 4 + 8 * g, 4) as u32;
        user_bits |= nib << (4 * g);
    }
    Ok(LtcFrame {
        tc: Timecode::new(ht * 10 + hu, mt * 10 + mu, st * 10 + su, ft * 10 + fu),
        drop_frame: get(w, 10),
        color_frame: get(w, 11),
        field_mark: get(w, 27),
        bgf0: get(w, 43),
        bgf1: get(w, 58),
        biphase_correction: get(w, 59),
        user_bits,
    })
}

pub fn to_hex(w: u128) -> String {
    format!("0x{w:020X}")
}

pub fn from_hex(s: &str) -> Result<u128, String> {
    let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if t.is_empty() || t.len() > 20 {
        return Err(format!("bad LTC hex length: {s:?}"));
    }
    u128::from_str_radix(t, 16).map_err(|e| format!("bad LTC hex {s:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = LtcFrame {
            tc: Timecode::new(1, 2, 3, 4),
            drop_frame: true,
            color_frame: false,
            field_mark: true,
            bgf0: false,
            bgf1: true,
            biphase_correction: false,
            user_bits: 0xDEADBEEF,
        };
        let w = encode(&f);
        assert_eq!(getn(w, 64, 16) as u16, SYNC_WORD);
        let d = decode(w).unwrap();
        assert_eq!(d, f);
    }

    #[test]
    fn bad_sync_rejected() {
        let f = LtcFrame {
            tc: Timecode::new(0, 0, 0, 1),
            drop_frame: false,
            color_frame: false,
            field_mark: false,
            bgf0: false,
            bgf1: false,
            biphase_correction: false,
            user_bits: 0,
        };
        let w = encode(&f) ^ (1u128 << 79);
        assert!(decode(w).is_err());
    }

    #[test]
    fn hex_roundtrip() {
        let w = 0x3FFD_0000_0000_0000_00ABu128;
        assert_eq!(from_hex(&to_hex(w)).unwrap(), w);
        assert!(from_hex("0xZZ").is_err());
    }
}
