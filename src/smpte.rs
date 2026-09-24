//! Core SMPTE-12M style timecode domain.
//!
//! Frame rates are kept as exact rationals (`num/den` frames per second).
//! Drop-frame numbering is a *labeling* convention: it skips timecode labels
//! at minute boundaries; it never drops real frames.  We implement 29.97 and
//! 59.94 drop-frame per SMPTE (the only rates for which the df flag is legal
//! in this workbench).

use serde::{Deserialize, Serialize};

/// Exact frame rate in frames per second: `frames_per_second = num/den`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rate {
    pub num: u64,
    pub den: u64,
}

impl Rate {
    pub const F2997: Rate = Rate { num: 30000, den: 1001 };
    pub const F5994: Rate = Rate { num: 60000, den: 1001 };
    pub const F25: Rate = Rate { num: 25, den: 1 };
    pub const F24: Rate = Rate { num: 24, den: 1 };

    pub fn new(num: u64, den: u64) -> anyhow::Result<Rate> {
        if num == 0 || den == 0 {
            anyhow::bail!("rate components must be non-zero");
        }
        Ok(Rate { num, den })
    }

    /// Integer label rate (30 for 29.97, 60 for 59.94, etc.).
    pub fn nominal_fps(&self) -> u64 {
        // All supported rates round their label rate this way.
        (self.num + self.den / 2) / self.den
    }

    /// Frames per *labeled* minute used by drop-frame formulas.
    pub fn rounded_fps(&self) -> u64 {
        self.nominal_fps()
    }

    pub fn is_df_capable(&self) -> bool {
        *self == Rate::F2997 || *self == Rate::F5994
    }
}

/// A fully decoded SMPTE timecode label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timecode {
    pub h: u8,
    pub m: u8,
    pub s: u8,
    pub f: u8,
    pub drop_frame: bool,
    pub color_frame: bool,
    pub field: Option<bool>,
}

impl Timecode {
    pub fn label(h: u8, m: u8, s: u8, f: u8, df: bool, cf: bool) -> Self {
        Timecode { h, m, s, f, drop_frame: df, color_frame: cf, field: None }
    }

    pub fn parse(text: &str) -> Option<Timecode> {
        // Accept HH:MM:SS:FF or HH;MM;SS;FF (the latter means drop-frame).
        let bytes = text.as_bytes();
        let sep = bytes.get(2).copied()?;
        if sep != b':' && sep != b';' {
            return None;
        }
        let df = sep == b';';
        let parts: Vec<&str> = if df {
            text.split(';').collect()
        } else {
            text.split(':').collect()
        };
        if parts.len() != 4 {
            return None;
        }
        let mut v = [0u8; 4];
        for (i, p) in parts.iter().enumerate() {
            v[i] = p.parse().ok()?;
        }
        Some(Timecode::label(v[0], v[1], v[2], v[3], df, false))
    }

    pub fn format(&self) -> String {
        let sep = if self.drop_frame { ';' } else { ':' };
        format!(
            "{:02}{sep}{:02}{sep}{:02}{sep}{:02}",
            self.h, self.m, self.s, self.f
        )
    }

    pub fn with_df(mut self, df: bool) -> Self {
        self.drop_frame = df;
        self
    }
}

/// Whether a frame number is an *illegal* drop-frame label
/// (frame numbers 0..drop_count at the start of a minute that is not a
/// multiple of ten). `fps30` => drop_count 2; `fps60` => drop_count 4.
pub fn is_illegal_df_label(tc: Timecode, rate: Rate) -> bool {
    if !tc.drop_frame || !rate.is_df_capable() {
        return false;
    }
    let drop_count = if rate == Rate::F5994 { 4 } else { 2 };
    tc.s == 0 && tc.m % 10 != 0 && tc.f < drop_count
}

/// Validate BCD/range of a label for a given rate. Returns an error string.
pub fn validate_label(tc: Timecode, rate: Rate) -> Result<(), String> {
    let fps = rate.nominal_fps();
    if tc.h > 23 || tc.m > 59 || tc.s > 59 || tc.f >= fps as u8 {
        return Err(format!("label {} out of range for rate {}", tc.format(), rate_str(rate)));
    }
    if tc.drop_frame && !rate.is_df_capable() {
        return Err(format!("drop-frame flag illegal at rate {}", rate_str(rate)));
    }
    if is_illegal_df_label(tc, rate) {
        return Err(format!("{} is a skipped (illegal) drop-frame label", tc.format()));
    }
    Ok(())
}

pub fn rate_str(r: Rate) -> String {
    // Stable human/exchange rendering; exact rational is kept alongside.
    match (r.num, r.den) {
        (30000, 1001) => "29.97".into(),
        (60000, 1001) => "59.94".into(),
        (25, 1) => "25".into(),
        (24, 1) => "24".into(),
        (30, 1) => "30".into(),
        _ => format!("{}/{}", r.num, r.den),
    }
}

/// Number of *skipped labels* between two consecutive observed frames.
/// Returns 0 for normal step, 2/4 for a drop-frame minute boundary, etc.
/// Assumes labels are individually legal.
pub fn skipped_labels_between(prev: Timecode, next: Timecode, rate: Rate) -> i64 {
    let fps = rate.rounded_fps() as i64;
    let a = label_index(prev);
    let b = label_index(next);
    let naive = b - a; // may be negative (reverse) or wrapped
    if naive < 0 {
        return 0; // wrap / reverse handled elsewhere
    }
    // Real-frame distance: count legal labels strictly between a and b.
    let mut legal = 0i64;
    for x in a + 1..b {
        let t = index_label(x, prev.drop_frame);
        if !is_illegal_df_label(t, rate) {
            legal += 1;
        }
    }
    // expected real-frame delta for a normal step = 1.
    // skipped = naive label delta - 1 - legal_between
    // (legal_between frames are genuine frames that would mean a gap).
    naive - 1 - legal
}

/// Monotonic index of a label in the abstract 24h clock space (without
/// applying drop-frame skipping). Used purely for between-label math.
pub fn label_index(tc: Timecode) -> i64 {
    (((tc.h as i64) * 60 + tc.m as i64) * 60 + tc.s as i64) * 60 + tc.f as i64
}

pub fn index_label(mut x: i64, df: bool) -> Timecode {
    let f = (x % 60) as u8;
    x /= 60;
    let s = (x % 60) as u8;
    x /= 60;
    let m = (x % 60) as u8;
    x /= 60;
    let h = (x % 24) as u8;
    Timecode::label(h, m, s, f, df, false)
}

/// Convert a legal timecode label to an absolute frame count since 00:00,
/// applying drop-frame compensation.
pub fn tc_to_frames(tc: Timecode, rate: Rate) -> i64 {
    let fps = rate.rounded_fps() as i64;
    let drop_count = if rate == Rate::F5994 { 4 } else { 2 };
    let h = tc.h as i64;
    let m = tc.m as i64;
    let s = tc.s as i64;
    let f = tc.f as i64;
    let total_minutes = 60 * h + m;
    let skip = drop_count
        * (total_minutes - total_minutes / 10)
        * if tc.drop_frame { 1 } else { 0 };
    let frame = ((total_minutes * 60 + s) * fps + f) - skip;
    frame
}

/// Inverse of [`tc_to_frames`].
pub fn frames_to_tc(mut frames: i64, rate: Rate, df: bool, cf: bool) -> Timecode {
    let fps = rate.rounded_fps() as i64;
    let drop_count = if rate == Rate::F5994 { 4 } else { 2 };
    let frames_per_10min = 60 * fps * 10 - drop_count * 9;
    let frames_per_min = 60 * fps - drop_count;

    let mut fr = frames.rem_euclid(fps * 60 * 60 * 24);
    if df {
        let n10 = fr / frames_per_10min;
        fr -= n10 * frames_per_10min;
        let mut minute = 0i64;
        if fr > drop_count {
            minute = (fr - drop_count) / frames_per_min;
        }
        fr += drop_count * 9 * n10 + drop_count * minute;
    }
    let f = (fr % fps) as u8;
    let s = ((fr / fps) % 60) as u8;
    let m = ((fr / (fps * 60)) % 60) as u8;
    let h = (fr / (fps * 3600)) as u8;
    let _ = frames;
    Timecode::label(h, m, s, f, df, cf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_roundtrip_2997() {
        for n in [0i64, 1, 1798, 1799, 1800, 10789, 108000, 2589407] {
            let tc = frames_to_tc(n, Rate::F2997, true, false);
            assert_eq!(tc_to_frames(tc, Rate::F2997), n, "n={n} tc={}", tc.format());
        }
    }

    #[test]
    fn df_boundary_skips_two_labels_2997() {
        // 00:00:59:29 -> 00:01:00:02 skips labels :00 and :01.
        let a = Timecode::label(0, 0, 59, 29, true, false);
        let b = Timecode::label(0, 1, 0, 2, true, false);
        assert_eq!(skipped_labels_between(a, b, Rate::F2997), 2);
        // Every tenth minute skips nothing.
        let c = Timecode::label(0, 9, 59, 29, true, false);
        let d = Timecode::label(0, 10, 0, 0, true, false);
        assert_eq!(skipped_labels_between(c, d, Rate::F2997), 0);
    }

    #[test]
    fn df_boundary_skips_four_labels_5994() {
        let a = Timecode::label(0, 0, 59, 59, true, false);
        let b = Timecode::label(0, 1, 0, 4, true, false);
        assert_eq!(skipped_labels_between(a, b, Rate::F5994), 4);
    }

    #[test]
    fn illegal_labels() {
        assert!(is_illegal_df_label(Timecode::parse("00:01:00:00").unwrap().with_df(true), Rate::F2997));
        assert!(!is_illegal_df_label(Timecode::label(0, 10, 0, 0, true, false), Rate::F2997));
    }
}
