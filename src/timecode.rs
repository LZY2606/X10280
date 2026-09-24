//! SMPTE timecode mathematics: rational frame rates, drop-frame label
//! skipping (which skips *labels*, never media frames), and day wrap.
use serde::{Deserialize, Serialize};

/// Frame rate as an exact rational, e.g. 30000/1001 for 29.97.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub fn new(num: u32, den: u32) -> Option<Self> {
        if num == 0 || den == 0 {
            return None;
        }
        Some(Self { num, den })
    }
    /// Nominal (rounded) frame rate used for label arithmetic: 30000/1001 -> 30.
    pub fn nominal(&self) -> u32 {
        (self.num + self.den / 2) / self.den
    }
    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some((a, b)) = s.split_once('/') {
            Self::new(a.trim().parse().ok()?, b.trim().parse().ok()?)
        } else {
            Self::new(s.parse().ok()?, 1)
        }
    }
    pub fn to_string(&self) -> String {
        format!("{}/{}", self.num, self.den)
    }
}

/// A SMPTE timecode label. Labels are addresses, not frame counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Timecode {
    pub h: u8,
    pub m: u8,
    pub s: u8,
    pub f: u8,
}

impl Timecode {
    pub fn new(h: u8, m: u8, s: u8, f: u8) -> Self {
        Self { h, m, s, f }
    }
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.trim().split([':', ';', '.']).collect();
        if parts.len() != 4 {
            return None;
        }
        let v: Vec<u8> = parts.iter().map(|p| p.parse().ok()).collect::<Option<_>>()?;
        Some(Self::new(v[0], v[1], v[2], v[3]))
    }
    pub fn to_string(&self, drop: bool) -> String {
        if drop {
            format!("{:02}:{:02}:{:02};{:02}", self.h, self.m, self.s, self.f)
        } else {
            format!("{:02}:{:02}:{:02}:{:02}", self.h, self.m, self.s, self.f)
        }
    }
}

/// Labels skipped per minute (except every 10th minute) under drop-frame.
/// 29.97 (nominal 30) skips 2 labels; 59.94 (nominal 60) skips 4.
/// This is a *numbering* rule: no media frame is ever dropped.
pub fn dropped_labels_per_minute(rate: FrameRate, drop: bool) -> u32 {
    if !drop {
        return 0;
    }
    match rate.nominal() {
        30 => 2,
        60 => 4,
        _ => 0,
    }
}

/// Total addressable labels per 24h day under the given numbering.
pub fn frames_per_day(rate: FrameRate, drop: bool) -> u64 {
    let fps = rate.nominal() as u64;
    let d = dropped_labels_per_minute(rate, drop) as u64;
    let total_minutes = 24 * 60u64;
    fps * 86_400 - d * (total_minutes - total_minutes / 10)
}

/// Convert a label to a frame number within the day.
/// Returns `None` for labels that do not exist (e.g. 00:01:00;00 in 29.97 DF).
pub fn tc_to_frame(tc: Timecode, rate: FrameRate, drop: bool) -> Option<u64> {
    let fps = rate.nominal();
    if tc.m >= 60 || tc.s >= 60 || tc.h >= 24 || (tc.f as u32) >= fps {
        return None;
    }
    let d = dropped_labels_per_minute(rate, drop) as u64;
    if d > 0 && tc.m % 10 != 0 && tc.s == 0 && (tc.f as u64) < d {
        return None; // skipped label: does not exist under drop numbering
    }
    let total_minutes = tc.h as u64 * 60 + tc.m as u64;
    Some(
        (tc.h as u64 * 3600 + tc.m as u64 * 60 + tc.s as u64) * fps as u64
            + tc.f as u64
            - d * (total_minutes - total_minutes / 10),
    )
}

/// Inverse of `tc_to_frame`; wraps modulo one day.
pub fn frame_to_tc(n: u64, rate: FrameRate, drop: bool) -> Timecode {
    let fps = rate.nominal() as u64;
    let d = dropped_labels_per_minute(rate, drop) as u64;
    let n = n % frames_per_day(rate, drop);
    if d == 0 {
        let f = (n % fps) as u8;
        let mut rest = n / fps;
        let s = (rest % 60) as u8;
        rest /= 60;
        let m = (rest % 60) as u8;
        let h = (rest / 60) as u8;
        return Timecode::new(h, m, s, f);
    }
    let frames_first_min = fps * 60;
    let frames_per_min = fps * 60 - d;
    let frames_per_10min = frames_first_min + 9 * frames_per_min;
    let ten = n / frames_per_10min;
    let rem = n % frames_per_10min;
    let (minute_in_block, frame_in_minute) = if rem < frames_first_min {
        (0u64, rem)
    } else {
        let r = rem - frames_first_min;
        (1 + r / frames_per_min, d + r % frames_per_min)
    };
    let total_minutes = ten * 10 + minute_in_block;
    let s = (frame_in_minute / fps) as u8;
    let f = (frame_in_minute % fps) as u8;
    Timecode::new(
        (total_minutes / 60) as u8,
        (total_minutes % 60) as u8,
        s,
        f,
    )
}

/// Signed smallest difference b-a on the daily ring, in frames.
/// Positive means b is ahead of a (forward direction).
pub fn ring_delta(a: u64, b: u64, rate: FrameRate, drop: bool) -> i64 {
    let day = frames_per_day(rate, drop) as i64;
    let mut d = (b as i64 - a as i64) % day;
    if d > day / 2 {
        d -= day;
    }
    if d < -(day / 2) {
        d += day;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r2997() -> FrameRate {
        FrameRate::new(30000, 1001).unwrap()
    }
    fn r5994() -> FrameRate {
        FrameRate::new(60000, 1001).unwrap()
    }

    #[test]
    fn drop_minute_boundary_skips_labels_not_frames() {
        let r = r2997();
        // 00:00:59;29 -> next label is 00:01:00;02 (labels ;00 ;01 skipped)
        let a = tc_to_frame(Timecode::new(0, 0, 59, 29), r, true).unwrap();
        let b = tc_to_frame(Timecode::new(0, 1, 0, 2), r, true).unwrap();
        assert_eq!(b - a, 1, "media is continuous; only labels skipped");
        assert!(tc_to_frame(Timecode::new(0, 1, 0, 0), r, true).is_none());
        assert!(tc_to_frame(Timecode::new(0, 1, 0, 1), r, true).is_none());
        // every 10th minute does not skip
        assert!(tc_to_frame(Timecode::new(0, 10, 0, 0), r, true).is_some());
        assert!(tc_to_frame(Timecode::new(0, 10, 0, 1), r, true).is_some());
    }

    #[test]
    fn drop_5994_skips_four_labels() {
        let r = r5994();
        assert!(tc_to_frame(Timecode::new(0, 1, 0, 3), r, true).is_none());
        assert!(tc_to_frame(Timecode::new(0, 1, 0, 4), r, true).is_some());
        let a = tc_to_frame(Timecode::new(0, 0, 59, 59), r, true).unwrap();
        let b = tc_to_frame(Timecode::new(0, 1, 0, 4), r, true).unwrap();
        assert_eq!(b - a, 1);
    }

    #[test]
    fn roundtrip_all_day_drop() {
        let r = r2997();
        let day = frames_per_day(r, true);
        assert_eq!(day, 2_589_408); // well-known 29.97 DF day length
        for n in (0..day).step_by(997) {
            let tc = frame_to_tc(n, r, true);
            assert_eq!(tc_to_frame(tc, r, true), Some(n), "roundtrip at {}", n);
        }
    }

    #[test]
    fn midnight_wrap_ring() {
        let r = r2997();
        let day = frames_per_day(r, true);
        let last = frame_to_tc(day - 1, r, true);
        assert_eq!(last.to_string(true), "23:59:59;29");
        let d = ring_delta(day - 1, 0, r, true);
        assert_eq!(d, 1, "23:59:59;29 -> 00:00:00;00 is +1 across midnight");
    }

    #[test]
    fn non_drop_25_roundtrip() {
        let r = FrameRate::new(25, 1).unwrap();
        let day = frames_per_day(r, false);
        assert_eq!(day, 25 * 86_400);
        for n in (0..day).step_by(1234) {
            assert_eq!(tc_to_frame(frame_to_tc(n, r, false), r, false), Some(n));
        }
    }
}
