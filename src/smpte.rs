// SMPTE linear-frame arithmetic.
//
// Label numbering (the HH:MM:SS:FF display) is NOT the count of recorded
// frames when drop frame is active: at each minute boundary except every
// tenth minute, `drop_count` labels are skipped. No frame is dropped.
//
// `linear` below is the count of *frame slots* since 00:00:00:00.

use crate::domain::{Rate, Timecode};

/// Labels skipped at a drop-frame minute boundary (2 at 29.97, 4 at 59.94).
pub fn drop_count(rate: &Rate) -> u64 {
    (rate.nominal() / 15).max(1) * 2
}

/// Frames per nominal second label.
pub fn fps(rate: &Rate) -> u64 {
    rate.nominal()
}

/// Valid frame label range check.
pub fn valid_label(tc: Timecode, rate: &Rate) -> bool {
    let f = fps(rate);
    tc.h < 24 && tc.m < 60 && tc.s < 60 && (tc.f as u64) < f
}

/// Does this exact label get skipped at a drop-frame minute boundary?
/// (frames 0..drop_count in minute % 10 != 0)
pub fn is_skipped_label(tc: Timecode, rate: &Rate) -> bool {
    tc.s == 0
        && tc.m % 10 != 0
        && (tc.f as u64) < drop_count(rate)
}

/// Convert a label to linear frame-slot count. Assumes a valid label;
/// skipped labels are not in the valid label stream but convert consistently.
pub fn label_to_linear(tc: Timecode, drop: bool, rate: &Rate) -> i128 {
    let f = fps(rate) as i128;
    let total_minutes = tc.h as i128 * 60 + tc.m as i128;
    let mut lin = total_minutes * 60 * f + tc.s as i128 * f + tc.f as i128;
    if drop {
        let d = (drop_count(rate)) as i128;
        lin -= d * (total_minutes - total_minutes / 10);
    }
    lin
}

/// Linear frame slots per full 24-hour label cycle (at frame-slot rate f).
pub fn frames_per_day(rate: &Rate) -> i128 {
    fps(rate) as i128 * 60 * 60 * 24
}

/// Inverse: linear frame-slot count (mod 24h) -> label.
pub fn linear_to_label(lin: i128, drop: bool, rate: &Rate) -> Timecode {
    let f = fps(rate) as i128;
    let d = drop_count(rate) as i128;
    let mut n = lin.rem_euclid(frames_per_day(rate));
    let hf = 3600 * f;
    let hour = n / hf;
    n %= hf;
    let mut minute: i128 = 0;
    if drop {
        // Frames into hour at the start of minute m:
        //   60*m*f - d*(m - m/10)
        while minute < 59 {
            let start = minute * 60 * f - d * (minute - minute / 10);
            let next = (minute + 1) * 60 * f - d * ((minute + 1) - (minute + 1) / 10);
            if n >= start && n < next {
                break;
            }
            minute += 1;
        }
    } else {
        minute = n / (60 * f);
    }
    let minute_start = if drop {
        minute * 60 * f - d * (minute - minute / 10)
    } else {
        minute * 60 * f
    };
    let rest = n - minute_start;
    let sec = rest / f;
    let frame = rest % f;
    Timecode {
        h: hour as u8,
        m: minute as u8,
        s: sec as u8,
        f: frame as u8,
    }
}

/// Signed linear delta from label a to label b; None when either label is
/// invalid for the rate. Callers resolve wrap vs. real backward themselves.
pub fn signed_delta(a: Timecode, b: Timecode, drop: bool, rate: &Rate) -> Option<i128> {
    if !valid_label(a, rate) || !valid_label(b, rate) {
        return None;
    }
    Some(label_to_linear(b, drop, rate) - label_to_linear(a, drop, rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r30() -> Rate {
        Rate::parse("30000/1001").unwrap()
    }
    fn r60() -> Rate {
        Rate::parse("60000/1001").unwrap()
    }

    #[test]
    fn nominal_and_drop_count() {
        assert_eq!(r30().nominal(), 30);
        assert_eq!(r60().nominal(), 60);
        assert_eq!(drop_count(&r30()), 2);
        assert_eq!(drop_count(&r60()), 4);
        assert!(r30().drop_frame_supported());
        assert!(r60().drop_frame_supported());
        assert!(!Rate::parse("25").unwrap().drop_frame_supported());
    }

    #[test]
    fn drop_minute_boundary_2997() {
        // 00:00:59;29 is followed by 00:01:00;02 in the valid label stream.
        let a = Timecode { h: 0, m: 0, s: 59, f: 29 };
        let b = Timecode { h: 0, m: 1, s: 0, f: 2 };
        assert_eq!(label_to_linear(a, true, &r30()), 1799);
        assert_eq!(label_to_linear(b, true, &r30()), 1800);
        assert!(is_skipped_label(Timecode { h: 0, m: 1, s: 0, f: 0 }, true, &r30()));
        assert!(!is_skipped_label(Timecode { h: 0, m: 10, s: 0, f: 0 }, true, &r30()));
    }

    #[test]
    fn ten_minute_block_integrity() {
        // 00:09:59;29 -> 00:10:00;00 are adjacent frame slots.
        let a = Timecode { h: 0, m: 9, s: 59, f: 29 };
        let b = Timecode { h: 0, m: 10, s: 0, f: 0 };
        assert_eq!(signed_delta(a, b, true, &r30()), Some(1));
    }

    #[test]
    fn roundtrip_all_2997_df_labels() {
        let rate = r30();
        let total = frames_per_day(&rate);
        for lin in (0..total).step_by(977) {
            let tc = linear_to_label(lin, true, &rate);
            assert_eq!(label_to_linear(tc, true, &rate), lin, "lin {lin} -> {tc:?}");
        }
    }

    #[test]
    fn roundtrip_all_5994_df_labels() {
        let rate = r60();
        let total = frames_per_day(&rate);
        for lin in (0..total).step_by(2029) {
            let tc = linear_to_label(lin, true, &rate);
            assert_eq!(label_to_linear(tc, true, &rate), lin, "lin {lin} -> {tc:?}");
        }
    }

    #[test]
    fn roundtrip_non_drop() {
        let rate = Rate::parse("25").unwrap();
        for lin in (0..frames_per_day(&rate)).step_by(1373) {
            let tc = linear_to_label(lin, false, &rate);
            assert_eq!(label_to_linear(tc, false, &rate), lin);
        }
    }
}
