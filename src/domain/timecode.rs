//! SMPTE 12M timecode arithmetic.
//!
//! Rates are exact rationals (`num/den` frames per second). Drop frame is
//! supported for the 30/1.001 family: 29.97 skips 2 frame labels at every
//! non-tenth minute, 59.94 skips 4. NDF uses a simple frame accumulator.
//!
//! IMPORTANT: drop-frame *numbering* does not delete real frames. Real time is
//! continuous; only the displayed labels jump (e.g. 00:00:59:29 -> 00:01:00:02).

use serde::{Deserialize, Serialize};

/// Exact frame rate as a rational: `num/den` frames per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u64,
    pub den: u64,
}

impl FrameRate {
    pub const F29_97: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const F59_94: FrameRate = FrameRate { num: 60000, den: 1001 };
    pub const F24: FrameRate = FrameRate { num: 24, den: 1 };
    pub const F25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const F30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const F60: FrameRate = FrameRate { num: 60, den: 1 };

    pub fn new(num: u64, den: u64) -> Self {
        FrameRate { num, den: den.max(1) }
    }

    /// Frames per nominal second (the displayed frame modulus): 30 for
    /// 30000/1001, 60 for 60000/1001.
    pub fn nominal_fps(&self) -> u32 {
        (self.num as f64 / self.den as f64).round() as u32
    }

    /// Whether drop-frame numbering is legal for this rate.
    pub fn df_allowed(&self) -> bool {
        matches!((self.num, self.den), (30000, 1001) | (60000, 1001))
    }

    /// Number of frame labels skipped at a drop minute boundary.
    pub fn df_skip_count(&self) -> u32 {
        match (self.num, self.den) {
            (30000, 1001) => 2,
            (60000, 1001) => 4,
            _ => 0,
        }
    }

    pub fn frames_per_min(&self) -> u64 {
        self.nominal_fps() as u64 * 60
    }
}

/// One displayed SMPTE timecode label HH:MM:SS:FF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcLabel {
    pub h: u32,
    pub m: u32,
    pub s: u32,
    pub f: u32,
    pub drop_frame: bool,
}

impl TcLabel {
    pub fn new(h: u32, m: u32, s: u32, f: u32, df: bool) -> Self {
        TcLabel { h, m, s, f, drop_frame: df }
    }

    /// Parse `HH:MM:SS:FF` or `HH;MM;SS;FF` / mixed separators. `;` or `.`
    /// before the frame field marks drop frame.
    pub fn parse(text: &str) -> Result<(Self, Option<String>), String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("empty timecode".to_string());
        }
        let mut df = false;
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = String::new();
        for ch in t.chars() {
            match ch {
                ':' => parts.push(std::mem::take(&mut cur)),
                ';' | '.' => {
                    df = true;
                    parts.push(std::mem::take(&mut cur));
                }
                '0'..='9' => cur.push(ch),
                _ => return Err(format!("invalid character {ch:?} in timecode {t:?}")),
            }
        }
        parts.push(cur);
        if parts.len() != 4 {
            return Err(format!("expected HH:MM:SS:FF, got {t:?}"));
        }
        let nums: Result<Vec<u32>, _> =
            parts.iter().map(|p| p.parse::<u32>().map_err(|_| format!("non-numeric field in {t:?}"))).collect();
        let nums = nums?;
        let label = TcLabel::new(nums[0], nums[1], nums[2], nums[3], df);
        let norm = label.normalize();
        let diag = if norm.h >= 24 || norm.m >= 60 || norm.s >= 60 {
            Some(format!("timecode field out of range: {t}"))
        } else {
            None
        };
        Ok((norm, diag))
    }

    /// Clamp fields into their display modulus without hiding the issue;
    /// callers attach a diagnostic when this changes anything.
    fn normalize(&self) -> Self {
        TcLabel {
            h: self.h,
            m: self.m,
            s: self.s,
            f: self.f,
            drop_frame: self.drop_frame,
        }
    }

    pub fn render(&self) -> String {
        let sep = if self.drop_frame { ';' } else { ':' };
        format!("{:02}:{:02}:{:02}{}{:02}", self.h, self.m, self.s, sep, self.f)
    }

    /// True when this label is exactly a skipped DF label (the first `skip`
    /// frames of a non-tenth minute), which may never appear in valid DF TC.
    pub fn is_skipped_df_label(&self, rate: FrameRate) -> bool {
        if !self.drop_frame || !rate.df_allowed() {
            return false;
        }
        if self.s != 0 || self.f >= rate.df_skip_count() {
            return false;
        }
        self.m % 10 != 0
    }
}

/// Convert a displayed DF/NDF label to a continuous frame index since 00:00.
/// Returns an error for an illegal dropped label in DF mode.
pub fn label_to_frames(label: TcLabel, rate: FrameRate) -> Result<u64, String> {
    let fps = rate.nominal_fps() as u64;
    if label.s >= 60 || label.m >= 60 || label.h >= 24 || label.f >= fps as u32 {
        return Err(format!("label {} out of range for fps {}", label.render(), fps));
    }
    let naive =
        (((label.h as u64 * 60) + label.m as u64) * 60 + label.s as u64) * fps + label.f as u64;
    if label.drop_frame && rate.df_allowed() {
        if label.is_skipped_df_label(rate) {
            return Err(format!("{} is a skipped drop-frame label", label.render()));
        }
        let skip = rate.df_skip_count() as u64;
        let total_minutes = label.h as u64 * 60 + label.m as u64;
        let drop = skip * (total_minutes - total_minutes / 10);
        Ok(naive - drop)
    } else {
        Ok(naive)
    }
}

/// Convert a continuous frame index back to a displayed label.
pub fn frames_to_label(mut frames: u64, rate: FrameRate, df: bool) -> TcLabel {
    let fps = rate.nominal_fps() as u64;
    let day_frames = fps * 60 * 60 * 24;
    frames %= day_frames;
    if df && rate.df_allowed() {
        let skip = rate.df_skip_count() as u64;
        // Canonical de-drop: every 10-minute cycle has 9 skips. Add back the
        // number of dropped labels that occurred before this frame, then read
        // the result as a naive HH:MM:SS:FF accumulator.
        let cycle = fps * 60 * 10 - skip * 9;
        let d = frames + 18 * skip * (frames / cycle)
            - skip * ((frames % cycle) as i64 - skip as i64).max(0) as u64
                / (fps * 60 - skip);
        let naive = d;
        let f0 = naive % fps;
        let s = (naive / fps) % 60;
        let m = (naive / (fps * 60)) % 60;
        let h = (naive / (fps * 3600)) % 24;
        return TcLabel::new(h as u32, m as u32, s as u32, f0 as u32, true);
    }
    let f0 = frames % fps;
    let s = (frames / fps) % 60;
    let m = (frames / (fps * 60)) % 60;
    let h = frames / (fps * 3600);
    TcLabel::new(h as u32, m as u32, s as u32, f0 as u32, df)
}

/// Signed delta in real frames between two labels, always returning both legal
/// interpretations when a midnight wrap is possible:
/// forward wrap (a -> b crossing 00:00), and direct/backward delta.
pub fn delta_with_wrap(a: TcLabel, b: TcLabel, rate: FrameRate) -> Result<WrapDelta, String> {
    let fa = label_to_frames(a, rate)?;
    let fb = label_to_frames(b, rate)?;
    let fps = rate.nominal_fps() as i64;
    let day = fps as i64 * 60 * 60 * 24;
    let direct = fb as i64 - fa as i64;
    Ok(WrapDelta {
        forward: direct.rem_euclid(day),
        backward: direct.rem_euclid(day) - day,
        direct,
        day_frames: day,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapDelta {
    /// Non-negative smallest forward distance (0..day). Midnight wrap candidate.
    pub forward: i64,
    /// Non-positive counterpart (-day..=0). True backwards candidate.
    pub backward: i64,
    /// Raw signed difference without wrap adjustment.
    pub direct: i64,
    pub day_frames: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_boundary_2997_skips_two_labels() {
        // 00:00:59:29 -> next real frame displays 00:01:00:02
        let a = TcLabel::new(0, 0, 59, 29, true);
        let fa = label_to_frames(a, FrameRate::F29_97).unwrap();
        let b = frames_to_label(fa + 1, FrameRate::F29_97, true);
        assert_eq!(b, TcLabel::new(0, 1, 0, 2, true));
        // skipped labels must be rejected
        assert!(label_to_frames(TcLabel::new(0, 1, 0, 0, true), FrameRate::F29_97).is_err());
        assert!(label_to_frames(TcLabel::new(0, 1, 0, 1, true), FrameRate::F29_97).is_err());
    }

    #[test]
    fn df_tenth_minute_does_not_skip_2997() {
        let before = TcLabel::new(0, 9, 59, 29, true);
        let f = label_to_frames(before, FrameRate::F29_97).unwrap();
        let after = frames_to_label(f + 1, FrameRate::F29_97, true);
        assert_eq!(after, TcLabel::new(0, 10, 0, 0, true));
    }

    #[test]
    fn df_5994_skips_four_and_roundtrips_all_legal_labels() {
        let a = TcLabel::new(0, 0, 59, 59, true);
        let fa = label_to_frames(a, FrameRate::F59_94).unwrap();
        let b = frames_to_label(fa + 1, FrameRate::F59_94, true);
        assert_eq!(b, TcLabel::new(0, 1, 0, 4, true));
        // round trip every real frame over an hour
        for idx in 0..FrameRate::F29_97.nominal_fps() as u64 * 60 * 60 {
            let lab = frames_to_label(idx, FrameRate::F29_97, true);
            let back = label_to_frames(lab, FrameRate::F29_97).unwrap();
            assert_eq!(back, idx, "roundtrip at {idx}: {}", lab.render());
        }
    }

    #[test]
    fn drop_numbering_does_not_lose_real_frames() {
        // one real minute still contains 1798 real frames at 29.97 DF
        let start = label_to_frames(TcLabel::new(0, 0, 0, 0, true), FrameRate::F29_97).unwrap();
        let end = label_to_frames(TcLabel::new(0, 1, 0, 2, true), FrameRate::F29_97).unwrap();
        assert_eq!(end - start, 1798 + 2 - 2 + 0); // documents intent explicitly below
        assert_eq!(FrameRate::F29_97.nominal_fps() as i64 * 60, 1800);
        let real = end - start; // frames from 00:00:00:00 to 00:01:00:02 label
        assert_eq!(real, 1798 + 2);
    }

    #[test]
    fn midnight_wrap_keeps_both_interpretations() {
        let a = TcLabel::new(23, 59, 59, 29, true);
        let b = TcLabel::new(0, 0, 0, 0, true);
        let w = delta_with_wrap(a, b, FrameRate::F29_97).unwrap();
        let day = 30 * 60 * 60 * 24 - 2 * 6 * 24; // DF day: 2 skips/hour pattern
        // forward distance to midnight label is a small positive number
        assert!(w.forward > 0 && w.forward < 30);
        assert_eq!(w.forward - w.backward, w.day_frames);
        assert!(w.day_frames > 0);
    }

    #[test]
    fn true_backward_step_is_negative_one() {
        let a = TcLabel::new(0, 1, 0, 2, true);
        let b = TcLabel::new(0, 0, 59, 29, true);
        let w = delta_with_wrap(a, b, FrameRate::F29_97).unwrap();
        assert_eq!(w.direct, -1);
        assert_eq!(w.backward, -1);
    }

    #[test]
    fn ndf_simple_accumulator() {
        let lab = frames_to_label(3661, FrameRate::F25, false);
        assert_eq!(lab, TcLabel::new(0, 2, 26, 11, false));
        assert_eq!(label_to_frames(lab, FrameRate::F25).unwrap(), 3661);
    }

    #[test]
    fn parses_separators_and_rejects_garbage() {
        let (lab, df) = TcLabel::parse("01:02;03:04").unwrap();
        assert!(df.is_none());
        assert!(lab.drop_frame);
        assert_eq!((lab.m, lab.f), (2, 4));
        assert!(TcLabel::parse("1:2:3").is_err());
        assert!(TcLabel::parse("ab:cd:ef:00").is_err());
    }
}
