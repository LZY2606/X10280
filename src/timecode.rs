//! SMPTE 12M timecode codec and drop-frame numbering mathematics.
//!
//! The preserved raw bit payloads follow the documented LTC/VITC bit layout:
//!
//! LTC (80 bits, one bit per frame):
//! ```text
//!   0-3   frame units (BCD)     8-11 seconds units
//!   4-7   frame tens  (BCD)    12-15 seconds tens
//!   10    drop-frame flag (D)  11    color-frame flag (C)
//!   16-19 minutes units 20-23 minutes tens
//!   24-27 hours units   28-31 hours tens
//!   32-39 user bits 1    40-47 user bits 2 (frame/sample groups)
//!   48-55 user bits 3    56-63 user bits 4 (minute/hour groups)
//!   64-77 binary group zero/one + fixed zeros
//!   78    biphase polarity parity placeholder
//!   79    sync group bit 0 ; 64..80 carry `00111111 11111101`
//! ```
//! VITC (90 bits, one full codeword per interlaced field): identical BCD
//! placement for frame/sec/min/hour, bit 11 = color frame, bit 10 reserved
//! (drop frame lives on frame tens bit position per SMPTE; we carry it at
//! bit 10 too), field flag at bit 35, and the VITC sync word 0b10
//! at bits 78..90 (`10000001 11111110`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeKind {
    Ltc,
    Vitc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub h: u8,
    pub m: u8,
    pub s: u8,
    pub f: u8,
    pub drop_frame: bool,
    pub color_frame: bool,
    pub field_flag: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedCode {
    pub kind: CodeKind,
    pub label: Label,
    pub user_bits: String,
    pub sync_ok: bool,
}

const LTC_SYNC: [u8; 16] = [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1];
const VITC_SYNC: [u8; 12] = [1, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0];

fn bcd_lo(v: u8) -> u8 {
    v % 10
}
fn bcd_hi(v: u8) -> u8 {
    v / 10
}

fn put_bcd(bits: &mut [u8], base: usize, value: u8) {
    bits[base] = value % 10 % 2;
    bits[base + 1] = (value % 10 / 2) % 2;
    bits[base + 2] = (value % 10 / 4) % 2;
    bits[base + 3] = (value % 10 / 8) % 2;
    bits[base + 4] = (value / 10) % 2;
    bits[base + 5] = (value / 20) % 2;
    bits[base + 6] = (value / 40) % 2;
}

fn get_digit(bits: &[u8], base: usize) -> Option<u8> {
    let lo = bits[base] + bits[base + 1] * 2 + bits[base + 2] * 4 + bits[base + 3] * 8;
    let hi = bits[base + 4] + bits[base + 5] * 2 + bits[base + 6] * 4;
    if lo > 9 {
        return None;
    }
    Some(hi * 10 + lo)
}

fn put_user(bits: &mut [u8], group: &str, base: usize) {
    let bytes = group.as_bytes();
    for i in 0..8 {
        let v = bytes.get(i).copied().unwrap_or(b'0');
        bits[base + i] = if v == b'1' { 1 } else { 0 };
    }
}

fn get_user(bits: &[u8], base: usize) -> String {
    let mut s = String::with_capacity(8);
    for i in 0..8 {
        s.push(if bits[base + i] == 1 { '1' } else { '0' });
    }
    s
}

/// Encode a label + 4 octets of user bits into the documented bit string.
pub fn encode_code(
    kind: CodeKind,
    lab: &Label,
    user_octets: &[&str; 4],
    field: Option<bool>,
) -> String {
    let n = match kind {
        CodeKind::Ltc => 80,
        CodeKind::Vitc => 90,
    };
    let mut b = vec![0u8; n];
    put_bcd(&mut b, 0, lab.f);
    put_bcd(&mut b, 8, lab.s);
    put_bcd(&mut b, 16, lab.m);
    put_bcd(&mut b, 24, lab.h);
    b[10] = lab.drop_frame as u8;
    b[11] = lab.color_frame as u8;
    put_user(&mut b, user_octets[0], 32);
    put_user(&mut b, user_octets[1], 40);
    put_user(&mut b, user_octets[2], 48);
    put_user(&mut b, user_octets[3], 56);
    match kind {
        CodeKind::Ltc => {
            for (i, &v) in LTC_SYNC.iter().enumerate() {
                b[64 + i] = v;
            }
            // bit 78 parity placeholder left 0
        }
        CodeKind::Vitc => {
            for (i, &v) in VITC_SYNC.iter().enumerate() {
                b[78 + i] = v;
            }
            b[35] = field.unwrap_or(false) as u8;
        }
    }
    b.iter().map(|x| char::from_digit(*x as u32, 10).unwrap()).collect()
}

/// Decode a preserved bit string. BCD errors surface as a diagnostic string.
pub fn decode_code(kind: CodeKind, bits: &str) -> Result<DecodedCode, String> {
    let want = match kind {
        CodeKind::Ltc => 80,
        CodeKind::Vitc => 90,
    };
    if bits.len() != want {
        return Err(format!("{} bit payload must be exactly {} bits, got {}", kind_name(kind), want, bits.len()));
    }
    let b: Result<Vec<u8>, ()> = bits
        .chars()
        .map(|c| match c {
            '0' => Ok(0),
            '1' => Ok(1),
            _ => Err(()),
        })
        .collect();
    let b = b.map_err(|_| format!("{} payload may only contain '0'/'1'", kind_name(kind)))?;

    let f = get_digit(&b, 0).ok_or_else(|| "frame BCD out of range (00-59)".to_string())?;
    let s = get_digit(&b, 8).ok_or_else(|| "seconds BCD out of range (00-59)".to_string())?;
    let m = get_digit(&b, 16).ok_or_else(|| "minutes BCD out of range (00-59)".to_string())?;
    let h = get_digit(&b, 24).ok_or_else(|| "hours BCD out of range (00-23)".to_string())?;

    let sync_ok = match kind {
        CodeKind::Ltc => b[64..80].iter().copied().eq(LTC_SYNC.iter().copied()),
        CodeKind::Vitc => b[78..90].iter().copied().eq(VITC_SYNC.iter().copied()),
    };

    let user = format!("{}{}{}{}", get_user(&b, 32), get_user(&b, 40), get_user(&b, 48), get_user(&b, 56));
    let field_flag = match kind {
        CodeKind::Vitc => Some(b[35] == 1),
        CodeKind::Ltc => None,
    };

    Ok(DecodedCode {
        kind,
        label: Label { h, m, s, f, drop_frame: b[10] == 1, color_frame: b[11] == 1, field_flag },
        user_bits: user,
        sync_ok,
    })
}

fn kind_name(k: CodeKind) -> &'static str {
    match k {
        CodeKind::Ltc => "LTC",
        CodeKind::Vitc => "VITC",
    }
}

impl std::fmt::Display for CodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(kind_name(*self))
    }
}

impl Label {
    /// Parse `HH:MM:SS;FF` (drop-frame marker `;`/`.`/`,`) or `HH:MM:SS:FF`.
    pub fn parse(text: &str) -> Result<(Label, bool), String> {
        let t = text.trim();
        let mut chars = t.chars();
        let drop_marker = match chars.find(|c| *c == ';' || *c == ',' || *c == '.') {
            Some(_) => true,
            None => false,
        };
        let norm: String = t.replace([';', ',', '.'], ":");
        let p: Vec<&str> = norm.split(':').collect();
        if p.len() != 4 {
            return Err("timecode must look like HH:MM:SS:FF".into());
        }
        let nums: Vec<u8> = p
            .iter()
            .map(|x| x.parse::<u8>().map_err(|_| format!("non-numeric component '{x}'")))
            .collect::<Result<_, _>>()?;
        let lab = Label {
            h: nums[0],
            m: nums[1],
            s: nums[2],
            f: nums[3],
            drop_frame: drop_marker,
            color_frame: false,
            field_flag: None,
        };
        lab.check_ranges()?;
        Ok((lab, drop_marker))
    }

    pub fn check_ranges(&self) -> Result<(), String> {
        if self.h > 23 {
            return Err("hours out of range (00-23)".into());
        }
        if self.m > 59 || self.s > 59 {
            return Err("minutes/seconds out of range (00-59)".into());
        }
        Ok(())
    }

    pub fn format(&self) -> String {
        let sep = if self.drop_frame { ';' } else { ':' };
        format!("{:02}:{:02}:{:02}{}{:02}", self.h, self.m, self.s, sep, self.f)
    }
}

// ---- drop-frame numbering -------------------------------------------------
//
// SMPTE "drop frame" is a *numbering* convention for 30000/1001 (NTSC 29.97)
// and 60000/1001 (59.94) rates. Two (or four) frame *numbers* are omitted at
// the start of each minute except every tenth minute — no picture frame is
// actually dropped. These functions are exact over i64 frame counts.

/// nominal frames per second supported for drop-frame (30 and 60).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DfSpec {
    pub nominal: i64,
}

impl DfSpec {
    pub fn for_rate(nominal: i64) -> Option<DfSpec> {
        match nominal {
            30 | 60 => Some(DfSpec { nominal }),
            _ => None,
        }
    }
    /// skipped label numbers per drop minute boundary (2 for 29.97, 4 for 59.94)
    pub fn skip(&self) -> i64 {
        if self.nominal == 60 {
            4
        } else {
            2
        }
    }
}

/// Validate a label's frame digit against the rate's frames-per-second and,
/// when drop-frame, against the skipped-number rule. Returns
/// `Ok(())` / `Err(diagnostic)`. This explicitly distinguishes an *illegal
/// label* from a legitimately skipped label number.
pub fn validate_label(lab: &Label, fps_nominal: i64) -> Result<(), String> {
    lab.check_ranges()?;
    if lab.f >= fps_nominal as u8 {
        return Err(format!("frame digit {} exceeds fps {}", lab.f, fps_nominal));
    }
    if lab.drop_frame {
        let spec = DfSpec::for_rate(fps_nominal)
            .ok_or_else(|| format!("drop-frame flag illegal at {fps_nominal} fps"))?;
        let skipped = lab.s == 0 && lab.f < spec.skip() as u8;
        if skipped && lab.m % 10 != 0 {
            return Err(format!(
                "illegal drop-frame label {}: frame numbers 0..{} are omitted at minute {} (numbering only, no picture frame lost)",
                lab.format(),
                spec.skip(),
                lab.m
            ));
        }
    }
    Ok(())
}

/// Count label-omitted frame numbers strictly before a given time-of-day index.
pub fn dropped_before(total_minutes: i64, spec: DfSpec) -> i64 {
    spec.skip() * (total_minutes - total_minutes / 10)
}

/// Convert a validated label to an absolute frame index since 00:00:00:00.
/// For drop-frame labels the index counts *real* picture frames; skipped label
/// numbers simply do not exist.
pub fn label_to_frames(lab: &Label, fps_nominal: i64) -> Result<i64, String> {
    validate_label(lab, fps_nominal)?;
    let base = ((lab.h as i64 * 60 + lab.m as i64) * 60 + lab.s as i64) * fps_nominal
        + lab.f as i64;
    if lab.drop_frame {
        let spec = DfSpec::for_rate(fps_nominal).unwrap();
        let mins = lab.h as i64 * 60 + lab.m as i64;
        Ok(base - dropped_before(mins, spec))
    } else {
        Ok(base)
    }
}

/// Inverse: real frame index since midnight -> display label.
pub fn frames_to_label(idx: i64, fps_nominal: i64, drop: bool) -> Label {
    let total_frames_per_hour = fps_nominal * 3600;
    let h = (idx / total_frames_per_hour).clamp(0, 23);
    let mut rem = idx - h * total_frames_per_hour;
    let spec = DfSpec::for_rate(fps_nominal);
    let mut m = 0i64;
    let mut s = 0i64;
    if drop {
        let spec = spec.unwrap();
        // walk minutes within the hour (at most 60)
        while m < 60 {
            let minute_frames = fps_nominal * 60 - if m % 10 != 0 { spec.skip() } else { 0 };
            if rem < minute_frames {
                break;
            }
            rem -= minute_frames;
            m += 1;
        }
        s = rem / fps_nominal;
    } else {
        m = rem / (fps_nominal * 60);
        rem -= m * fps_nominal * 60;
        s = rem / fps_nominal;
    }
    let f = rem % fps_nominal;
    Label {
        h: h as u8,
        m: m as u8,
        s: s as u8,
        f: f as u8,
        drop_frame: drop,
        color_frame: false,
        field_flag: None,
    }
}

/// Frames in a full 24h day for the rate/mode (59.94 DF and 29.97 DF map to
/// a whole day: 17982/1798 frames compensate the clock drift).
pub fn frames_per_day(fps_nominal: i64, drop: bool) -> i64 {
    if drop {
        let spec = DfSpec::for_rate(fps_nominal).unwrap();
        fps_nominal * 86400 - dropped_before(1440, spec)
    } else {
        fps_nominal * 86400
    }
}
