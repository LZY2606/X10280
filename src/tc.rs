//! SMPTE timecode mathematics: drop-frame numbering, midnight wrap,
//! reverse recording, field mode. All domain quantities carry their
//! unit / coordinate basis explicitly.

use serde::{Deserialize, Serialize};

/// Frame rate as an exact rational (e.g. 30000/1001 for 29.97).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub fn new(num: u32, den: u32) -> Self {
        FrameRate { num, den }
    }
    /// Nominal (integer) frame rate used for timecode labelling:
    /// 30000/1001 -> 30, 60000/1001 -> 60, 25/1 -> 25.
    pub fn nominal(&self) -> u32 {
        (self.num + self.den / 2) / self.den
    }
    /// Real frames per second as f64 (for residual maths only).
    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
    /// Duration of one frame in nanoseconds (rational, rounded).
    pub fn frame_duration_ns(&self) -> f64 {
        1_000_000_000.0 * self.den as f64 / self.num as f64
    }
}

/// A SMPTE timecode label. This is a *label*, not a frame count:
/// with drop-frame numbering some labels never exist, yet no frames
/// are dropped ("skipped numbers != dropped frames").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timecode {
    pub h: u32,
    pub m: u32,
    pub s: u32,
    pub f: u32,
}

impl Timecode {
    pub fn fmt_df(&self, drop: bool) -> String {
        let sep = if drop { ";" } else { ":" };
        format!("{:02}:{:02}:{:02}{}{:02}", self.h, self.m, self.s, sep, self.f)
    }
}

/// Number of labels skipped per dropped minute for a nominal rate.
/// 29.97 (nominal 30) drops 2, 59.94 (nominal 60) drops 4.
pub fn drop_count(nominal: u32) -> u32 {
    nominal / 30 * 2
}

/// Convert a timecode label to a monotonic frame number (within one
/// 24h cycle) under drop-frame or non-drop numbering.
pub fn tc_to_frame(tc: &Timecode, nominal: u32, drop: bool) -> i64 {
    let raw = ((tc.h as i64 * 60 + tc.m as i64) * 60 + tc.s as i64) * nominal as i64 + tc.f as i64;
    if !drop {
        return raw;
    }
    let d = drop_count(nominal) as i64;
    let total_minutes = (tc.h * 60 + tc.m) as i64;
    raw - d * (total_minutes - total_minutes / 10)
}

/// Convert a frame number back to a timecode label.
pub fn frame_to_tc(n: i64, nominal: u32, drop: bool) -> Timecode {
    let nominal = nominal as i64;
    let fr = if !drop {
        n
    } else {
        let d = drop_count(nominal as u32) as i64;
        let fp10 = nominal * 600 - 9 * d; // frames per 10-minute block
        let blocks = n.div_euclid(fp10);
        let m = n.rem_euclid(fp10);
        let mut skipped = 9 * d * blocks;
        if m >= nominal * 60 {
            // past minute 0 of the block: minutes 1..=9 are dropped
            let rem = m - nominal * 60;
            let dropped_minutes = rem / (nominal * 60 - d) + 1;
            skipped += d * dropped_minutes;
        }
        n + skipped
    };
    let fr = fr.rem_euclid(nominal * 86400);
    Timecode {
        h: (fr / (nominal * 3600)) as u32,
        m: ((fr / (nominal * 60)) % 60) as u32,
        s: ((fr / nominal) % 60) as u32,
        f: (fr % nominal) as u32,
    }
}

/// Labels skipped at a drop-frame minute boundary (e.g. 00;00 and 00;01
/// at every minute except every 10th). Returned as labels that do NOT
/// exist; the frames themselves are all present.
pub fn skipped_labels_at_minute(h: u32, m: u32, nominal: u32, drop: bool) -> Vec<Timecode> {
    if !drop || m % 10 == 0 {
        return Vec::new();
    }
    (0..drop_count(nominal))
        .map(|f| Timecode { h, m, s: 0, f })
        .collect()
}

/// Frames in one 24h timecode cycle under the given numbering.
pub fn cycle_frames(nominal: u32, drop: bool) -> i64 {
    tc_to_frame(&Timecode { h: 24, m: 0, s: 0, f: 0 }, nominal, drop)
}

/// A discontinuity between two consecutive decoded frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Discontinuity {
    /// Label went 23:59:59:xx -> 00:00:00:00. Ambiguous: may be a real
    /// midnight wrap of the timecode generator, or a genuine backward
    /// jump (re-record). Both interpretations are kept.
    MidnightWrapAmbiguous {
        interpretations: Vec<WrapInterpretation>,
    },
    /// Labels strictly decreasing but not at a midnight boundary:
    /// reverse recording (rewind/ shuttle) or time jump backwards.
    ReverseRun { delta_frames: i64 },
    /// Forward jump larger than one frame (code break / dropped decode).
    ForwardGap { missing_frames: i64 },
    /// Continuous (+1 frame).
    Continuous,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub struct WrapInterpretation {
    pub id: String,
    pub label: String,
    /// Signed frame delta under this interpretation.
    pub delta_frames: i64,
}

/// Classify the transition between two consecutive labels.
pub fn classify_transition(
    prev: &Timecode,
    next: &Timecode,
    nominal: u32,
    drop: bool,
) -> Discontinuity {
    let a = tc_to_frame(prev, nominal, drop);
    let b = tc_to_frame(next, nominal, drop);
    let delta = b - a;
    if delta == 1 {
        return Discontinuity::Continuous;
    }
    let cycle = cycle_frames(nominal, drop);
    // midnight wrap: forward delta equals -(cycle-1), i.e. wrapping 24h
    if delta == -(cycle - 1) {
        return Discontinuity::MidnightWrapAmbiguous {
            interpretations: vec![
                WrapInterpretation {
                    id: "midnight-wrap".into(),
                    label: "midnight wrap: generator crossed 24h, continuous +1 frame".into(),
                    delta_frames: 1,
                },
                WrapInterpretation {
                    id: "true-rollback".into(),
                    label: format!("true rollback: jumped back {} frames", cycle - 1),
                    delta_frames: -(cycle - 1),
                },
            ],
        };
    }
    if delta < 0 {
        return Discontinuity::ReverseRun { delta_frames: delta };
    }
    Discontinuity::ForwardGap { missing_frames: delta - 1 }
}

/// Field dominance for interlaced sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldMode {
    Progressive,
    Field1First,
    Field2First,
    Unknown,
}

/// Parse "HH:MM:SS:FF" or "HH:MM:SS;FF" (drop).
pub fn parse_tc(s: &str) -> Option<(Timecode, bool)> {
    let parts: Vec<&str> = s.split(|c| c == ':' || c == ';').collect();
    if parts.len() != 4 {
        return None;
    }
    let drop = s.contains(';');
    let v: Option<Vec<u32>> = parts.iter().map(|p| p.parse().ok()).collect();
    let v = v?;
    Some((Timecode { h: v[0], m: v[1], s: v[2], f: v[3] }, drop))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_roundtrip_2997() {
        let rate = FrameRate::new(30000, 1001);
        let n = rate.nominal();
        for f in [0i64, 1, 2, 1799, 1800, 1801, 17981, 17982, 17983, 107892, 2_073_599] {
            let tc = frame_to_tc(f, n, true);
            assert_eq!(tc_to_frame(&tc, n, true), f, "roundtrip failed at {f}");
        }
    }

    #[test]
    fn df_skips_labels_not_frames() {
        // 00:01:00:00 and 00:01:00:01 do not exist at 29.97 DF
        let skips = skipped_labels_at_minute(0, 1, 30, true);
        assert_eq!(skips.len(), 2);
        assert_eq!(skips[0], Timecode { h: 0, m: 1, s: 0, f: 0 });
        // but frame numbering is continuous across the boundary
        let a = tc_to_frame(&Timecode { h: 0, m: 0, s: 59, f: 29 }, 30, true);
        let b = tc_to_frame(&Timecode { h: 0, m: 1, s: 0, f: 2 }, 30, true);
        assert_eq!(b - a, 1);
        // every 10th minute keeps all labels
        assert!(skipped_labels_at_minute(0, 10, 30, true).is_empty());
    }

    #[test]
    fn df_5994_skips_four() {
        let skips = skipped_labels_at_minute(0, 1, 60, true);
        assert_eq!(skips.len(), 4);
        let a = tc_to_frame(&Timecode { h: 0, m: 0, s: 59, f: 59 }, 60, true);
        let b = tc_to_frame(&Timecode { h: 0, m: 1, s: 0, f: 4 }, 60, true);
        assert_eq!(b - a, 1);
    }

    #[test]
    fn midnight_wrap_keeps_two_interpretations() {
        let prev = Timecode { h: 23, m: 59, s: 59, f: 29 };
        let next = Timecode { h: 0, m: 0, s: 0, f: 0 };
        match classify_transition(&prev, &next, 30, true) {
            Discontinuity::MidnightWrapAmbiguous { interpretations } => {
                assert_eq!(interpretations.len(), 2);
                assert_eq!(interpretations[0].delta_frames, 1);
                assert!(interpretations[1].delta_frames < 0);
            }
            other => panic!("expected midnight wrap ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn reverse_run_detected() {
        let prev = Timecode { h: 1, m: 0, s: 10, f: 5 };
        let next = Timecode { h: 1, m: 0, s: 9, f: 20 };
        match classify_transition(&prev, &next, 30, false) {
            Discontinuity::ReverseRun { delta_frames } => assert!(delta_frames < 0),
            other => panic!("expected reverse run, got {other:?}"),
        }
    }
}
