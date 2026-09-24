//! SMPTE timecode <-> linear frame index conversion.
//!
//! A "frame index" is counted on the *timecode generator's* timeline and is
//! independent of wall clock. Drop-frame is purely a *numbering* convention:
//! frame numbers 0 and 1 are omitted from the labels after a non-tenth minute,
//! no video frame is discarded. See `alignment` and the counter-example
//! archive for the distinction made explicit.

use crate::domain::framerate::Rate;

/// Hours, minutes, seconds, frames exactly as labelled on tape/in the stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tc {
    pub h: i64,
    pub m: i64,
    pub s: i64,
    pub f: i64,
}

impl Tc {
    pub fn new(h: i64, m: i64, s: i64, f: i64) -> Self {
        Tc { h, m, s, f }
    }

    pub fn parse(text: &str) -> Result<Tc, String> {
        let parts: Vec<&str> = text.trim().split(':').collect();
        if parts.len() != 4 {
            return Err(format!("timecode must be HH:MM:SS:FF, got {text:?}"));
        }
        let mut v = [0i64; 4];
        for (i, p) in parts.iter().enumerate() {
            if p.len() != 2 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("timecode component must be two digits: {p:?}"));
            }
            v[i] = p.parse().map_err(|_| format!("bad component {p:?}"))?;
        }
        Ok(Tc::new(v[0], v[1], v[2], v[3]))
    }

    pub fn format(&self) -> String {
        format!("{:02}:{:02}:{:02}:{:02}", self.h, self.m, self.s, self.f)
    }

    /// Number of frame labels in one full 24h cycle at the nominal rate.
    fn day_frames(nominal: i64) -> i64 {
        24 * 60 * 60 * nominal
    }

    /// Is this label one of the numbers that must never appear under drop-frame?
    /// (frame 0/1 within a non-tenth minute, frame 0..3 for 59.94)
    pub fn is_dropped_label(&self, rate: Rate) -> bool {
        if !rate.supports_drop_frame() || self.s != 0 {
            return false;
        }
        let scale = if (rate.num, rate.den) == (60000, 1001) { 2 } else { 1 };
        let nskip = 2 * scale;
        self.m % 10 != 0 && self.f < nskip
    }
}

/// Convert a labelled timecode to a linear frame index within a 24h cycle.
/// Returns an error string if the label is illegal (e.g. a dropped number used
/// with the drop-frame flag set).
pub fn tc_to_index(tc: Tc, rate: Rate, drop_frame: bool) -> Result<i64, String> {
    let nominal = rate.nominal();
    if !(0..24).contains(&tc.h)
        || !(0..60).contains(&tc.m)
        || !(0..60).contains(&tc.s)
        || tc.f < 0
        || tc.f >= nominal
    {
        return Err(format!("timecode out of range for rate: {}", tc.format()));
    }
    if drop_frame && !rate.supports_drop_frame() {
        return Err("drop-frame flag set for a rate without DF numbering".to_string());
    }
    if drop_frame && tc.is_dropped_label(rate) {
        return Err(format!(
            "{} is a skipped (never displayed) drop-frame label",
            tc.format()
        ));
    }
    let total_minutes = 60 * tc.h + tc.m;
    let mut idx = total_minutes * 60 * nominal
        + tc.s * nominal
        + tc.f
        - rate.dropped_before_minute(total_minutes, drop_frame);
    if idx < 0 {
        idx += Tc::day_frames(nominal);
    }
    Ok(idx)
}

/// Inverse: linear frame index (mod the 24h cycle) -> labelled timecode.
pub fn index_to_tc(mut idx: i64, rate: Rate, drop_frame: bool) -> Tc {
    let nominal = rate.nominal();
    let day = Tc::day_frames(nominal);
    idx = idx.rem_euclid(day);
    if !drop_frame || !rate.supports_drop_frame() {
        let f = idx % nominal;
        let s = (idx / nominal) % 60;
        let m = (idx / (nominal * 60)) % 60;
        let h = idx / (nominal * 3600);
        return Tc::new(h, m, s, f);
    }
    // Frames elapsed up to the start of each ten-minute block = 10*60*N - 18*scale.
    let scale = if (rate.num, rate.den) == (60000, 1001) { 2 } else { 1 };
    let block_len = 10 * 60 * nominal - 18 * scale;
    let block = idx / block_len;
    let within = idx % block_len;
    // Walk the ten one-minute segments inside the block.
    let mut m_in_block = 0i64;
    let mut remaining = within;
    while m_in_block < 9 {
        let seg = 60 * nominal - if m_in_block == 0 { 0 } else { 2 * scale };
        if remaining < seg {
            break;
        }
        remaining -= seg;
        m_in_block += 1;
    }
    let minute = block * 10 + m_in_block;
    let f = remaining % nominal;
    let s = (remaining / nominal) % 60;
    Tc::new(minute / 60, minute % 60, s, f)
}

/// Signed, wraparound-aware shortest step from frame index `prev` to `next`,
/// returning the two legitimate hypotheses when the wrap is ambiguous.
///
/// The labels alone cannot distinguish a midnight wraparound (generator kept
/// running, index jumped 23:59:59:29 -> 00:00:00:00) from a true backwards
/// scrub/rewind. Both are therefore retained as first-class interpretations:
///   - wrap hypothesis: large *forward* jump (+day step)
///   - back hypothesis: raw negative step
/// The tuple order is `(wrap_step, back_step)`; callers keep both until a human
/// or device-clock anchor resolves them.
pub fn step_hypotheses(prev: i64, next: i64, nominal: i64) -> (i64, i64) {
    let raw = next - prev;
    let day = Tc::day_frames(nominal);
    let wrap = if raw < 0 { raw + day } else { raw - day };
    // `wrap` is the alternative (opposite-direction) interpretation.
    if raw < 0 {
        // back = raw ; forward-wrap = raw + day
        (raw + day, raw)
    } else {
        (raw, raw - day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r30() -> Rate { Rate::new(30000, 1001).unwrap() }
    fn r60() -> Rate { Rate::new(60000, 1001).unwrap() }

    #[test]
    fn classic_df_boundary_2997() {
        // 00:00:59:29 is followed by 00:01:00:02 (numbers 0,1 skipped).
        let a = tc_to_index(Tc::new(0, 0, 59, 29), r30(), true).unwrap();
        let b = tc_to_index(Tc::new(0, 1, 0, 2), r30(), true).unwrap();
        assert_eq!(b - a, 1, "labels skip but only one real frame elapses");
        // Skipped label must be rejected.
        assert!(tc_to_index(Tc::new(0, 1, 0, 0), r30(), true).is_err());
        // Tenth minute is not skipped: 00:09:59:29 -> 00:10:00:00 step 1.
        let a = tc_to_index(Tc::new(0, 9, 59, 29), r30(), true).unwrap();
        let b = tc_to_index(Tc::new(0, 10, 0, 0), r30(), true).unwrap();
        assert_eq!(b - a, 1);
    }

    #[test]
    fn df_boundary_5994_skips_four() {
        let a = tc_to_index(Tc::new(0, 0, 59, 59), r60(), true).unwrap();
        let b = tc_to_index(Tc::new(0, 1, 0, 4), r60(), true).unwrap();
        assert_eq!(b - a, 1);
        assert!(tc_to_index(Tc::new(0, 1, 0, 3), r60(), true).is_err());
    }

    #[test]
    fn roundtrip_inverse() {
        for idx in [0, 1798, 1799, 1800, 107_890, 1_000_000] {
            let tc = index_to_tc(idx, r30(), true);
            assert_eq!(tc_to_index(tc, r30(), true).unwrap(), idx.rem_euclid(24 * 3600 * 30));
        }
    }

    #[test]
    fn midnight_wrap_vs_back_two_solutions() {
        // Last label to first label: raw step -2589399 (back) or +1 (wrap).
        let last = tc_to_index(Tc::new(23, 59, 59, 29), r30(), true).unwrap();
        let first = tc_to_index(Tc::new(0, 0, 0, 0), r30(), true).unwrap();
        let (wrap, back) = step_hypotheses(last, first, 30);
        assert_eq!(wrap, 1);
        assert_eq!(back, -(24 * 3600 * 30 - 1));
    }

    #[test]
    fn non_df_plain() {
        let r = Rate::new(25, 1).unwrap();
        let a = tc_to_index(Tc::new(0, 0, 0, 0), r, false).unwrap();
        let b = tc_to_index(Tc::new(0, 0, 1, 0), r, false).unwrap();
        assert_eq!(b - a, 25);
    }
}
