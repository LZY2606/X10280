//! SMPTE 12M style timecode arithmetic.
//!
//! Important domain distinction maintained throughout this module:
//!
//! * A drop-frame *label* (`HH:MM:SS;FF`) only skips label numbers (frame
//!   numbers 0 and 1 of most minutes are never displayed).  No real frame is
//!   dropped.  Converting a legal DF label to a frame count is strictly
//!   monotonic, including across minute boundaries.
//! * At midnight labels wrap from `23:59:59;29` back to `00:00:00;00`, and when
//!   reading backwards the same transition can equally mean a genuine clock
//!   step backwards.  Both interpretations are retained as candidates.

use serde::{Deserialize, Serialize};

/// Frame rate as an exact rational number of *frame slots per second*.
///
/// For field-rate samples (59.94 VITC) the rate is still 60000/1001; field
/// pairs belonging to one frame are distinguished by [`FieldKind`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Rate {
    pub numerator: u64,
    pub denominator: u64,
}

impl Rate {
    pub fn parse(text: &str) -> Option<Rate> {
        let text = text.trim();
        let (n, d) = if let Some((a, b)) = text.split_once('/') {
            (a.trim().parse().ok()?, b.trim().parse().ok()?)
        } else {
            let whole: u64 = text.parse().ok()?;
            (whole, 1)
        };
        if n == 0 || d == 0 {
            return None;
        }
        Some(Rate { numerator: n, denominator: d })
    }

    pub fn render(&self) -> String {
        format!("{}/{}", self.numerator, self.denominator)
    }

    /// Number of label units inside one second (30 for 30000/1001, 60 for
    /// 60000/1001, 25 for 25/1 ...).
    pub fn units_per_second(&self) -> Option<u64> {
        let candidates = [
            (24000, 1001, 24),
            (24, 1, 24),
            (25, 1, 25),
            (30000, 1001, 30),
            (30, 1, 30),
            (60000, 1001, 60),
            (60, 1, 60),
            (50, 1, 50),
        ];
        candidates
            .into_iter()
            .find(|(n, d, _)| *n == self.numerator && *d == self.denominator)
            .map(|(_, _, u)| u)
    }

    /// Whether SMPTE drop-frame numbering is legal at this rate.
    pub fn dropframe_legal(&self) -> bool {
        matches!(
            (self.numerator, self.denominator),
            (30000, 1001) | (60000, 1001)
        )
    }

    /// Nominal integer rate used by the label grammar (30 or 60 for DF).
    pub fn nominal_units(&self) -> Option<u64> {
        self.units_per_second()
    }
}

impl std::fmt::Display for Rate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

/// A parsed `HH:MM:SS:FF` / `HH:MM:SS;FF` label exactly as received.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Label {
    pub h: u32,
    pub m: u32,
    pub s: u32,
    pub f: u32,
    /// Separator actually present in the source text (`;` `.` or `:`).
    pub separator: char,
}

impl Label {
    pub fn parse(text: &str) -> Result<Label, String> {
        let raw = text.trim();
        let sep = raw
            .chars()
            .find(|c| *c == ';' || *c == '.' || *c == ':')
            .ok_or_else(|| format!("timecode label '{raw}' must look like HH:MM:SS:FF"))?;
        let norm: String = raw
            .chars()
            .map(|c| if c == ';' || c == '.' { ':' } else { c })
            .collect();
        let parts: Vec<&str> = norm.split(':').collect();
        if parts.len() != 4 {
            return Err(format!("timecode label '{raw}' needs four fields"));
        }
        let mut nums = [0u32; 4];
        for (i, p) in parts.iter().enumerate() {
            nums[i] = p
                .parse()
                .map_err(|_| format!("field #{i} of '{raw}' is not an integer"))?;
        }
        let [h, m, s, f] = nums;
        if h > 23 {
            return Err(format!("hours out of range in '{raw}'"));
        }
        if m > 59 || s > 59 {
            return Err(format!("minutes/seconds out of range in '{raw}'"));
        }
        Ok(Label { h, m, s, f, separator: sep })
    }

    pub fn render(&self) -> String {
        format!(
            "{:02}:{:02}:{:02}{}{:02}",
            self.h, self.m, self.s, self.separator, self.f
        )
    }

    pub fn declared_df(&self) -> Option<bool> {
        match self.separator {
            ';' | '.' => Some(true),
            ':' => None, // non-committal; the material header decides
            _ => None,
        }
    }
}

/// Drop-frame label validity at a given nominal unit rate (30 or 60).
///
/// Returns `Ok(())` for legal labels and `Err(dropped)` describing the unit
/// numbers that the grammar skips at that minute boundary.
pub fn df_label_legal(label: Label, units: u64) -> Result<(), Vec<u32>> {
    let factor = units / 30; // 1 for 29.97, 2 for 59.94
    let drop_count = 2 * factor as u32;
    if label.s == 0 && label.m % 10 != 0 && label.f < drop_count {
        Err((0..drop_count).collect())
    } else {
        Ok(())
    }
}

/// True for minutes where labels 0(,1) are skipped (every minute except every
/// tenth minute).
pub fn df_minute_skips(total_minutes: u64, units: u64) -> bool {
    total_minutes % 10 != 0 && (units == 30 || units == 60)
}

/// Convert a guaranteed-legal label to its zero-based frame-slot index.
///
/// Inputs MUST already be validated (frame range + drop-frame legality); the
/// function intentionally has no illegal-label branch so it cannot silently
/// reinterpret bad input.
pub fn label_to_frame_units(label: Label, df: bool, units: u64) -> u64 {
    let factor = units / 30;
    let total_minutes = label.h as u64 * 60 + label.m as u64;
    let dropped = if df && units % 30 == 0 {
        2 * factor * (total_minutes - total_minutes / 10)
    } else {
        0
    };
    (total_minutes * 60 + label.s as u64) * units + label.f as u64 - dropped
}

/// Render a frame-slot index back into a (always legal) DF or NDF label.
pub fn frame_units_to_label(mut units_idx: u64, df: bool, units: u64) -> Label {
    let factor = units / 30;
    let sep = if df { ';' } else { ':' };
    if !df {
        let f = (units_idx % units) as u32;
        units_idx /= units;
        let s = (units_idx % 60) as u32;
        units_idx /= 60;
        let m = (units_idx % 60) as u32;
        let h = ((units_idx / 60) % 24) as u32;
        return Label { h, m, s, f, separator: sep };
    }
    // Drop-frame: count minutes, re-adding the skipped labels.
    let mut total_minutes = 0u64;
    loop {
        let minute_skips = if df_minute_skips(total_minutes, units) {
            2 * factor
        } else {
            0
        };
        // frame units belonging to minute `total_minutes`:
        let start = total_minutes * 60 * units
            - 2 * factor * (total_minutes - total_minutes / 10);
        let len = 60 * units - minute_skips;
        if units_idx < start + len {
            let within = units_idx - start;
            let s = (within / units) as u32;
            let f = (within % units) as u32;
            let h = ((total_minutes / 60) % 24) as u32;
            let m = (total_minutes % 60) as u32;
            return Label { h, m, s, f, separator: sep };
        }
        total_minutes += 1;
        if total_minutes > 24 * 60 {
            // Wrap arithmetic for callers that pass >= one day.
            return frame_units_to_label(units_idx % day_units(units), df, units);
        }
    }
}

pub fn day_units(units: u64) -> u64 {
    24 * 60 * 60 * units
}

/// Forward delta from frame-slot index `a` to `b`, retaining the midnight-wrap
/// ambiguity: returns the forward wrap solution and the unwrapped difference.
/// Callers pick which is valid from continuity context; when context cannot
/// decide, both must be kept as candidates.
pub fn forward_deltas(a: u64, b: u64, units: u64) -> ForwardDelta {
    let day = day_units(units);
    let raw = b as i64 - a as i64;
    let wrapped = ((b as i64 - a as i64) % day as i64 + day as i64) % day as i64;
    ForwardDelta {
        forward_wrapped: wrapped as u64,
        unwrapped: raw,
        day_span: day,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ForwardDelta {
    /// Delta assuming at most one forward midnight wrap.
    pub forward_wrapped: u64,
    /// Signed delta without wrap correction (negative == real backwards step).
    pub unwrapped: i64,
    pub day_span: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_roundtrip_2997_skips_minute_boundary() {
        let rate = Rate { numerator: 30000, denominator: 1001 };
        let units = rate.units_per_second().unwrap();
        // 00:00:59;29 -> next label is 00:01:00;02 (00 and 01 skipped).
        let a = Label::parse("00:00:59;29").unwrap();
        let b = Label::parse("00:01:00;02").unwrap();
        df_label_legal(b, units).unwrap();
        assert!(df_label_legal(Label::parse("00:01:00;00").unwrap(), units).is_err());
        let fa = label_to_frame_units(a, true, units);
        let fb = label_to_frame_units(b, true, units);
        assert_eq!(fb - fa, 1, "no real frame is dropped across the minute");
        // Every tenth minute keeps ;00 and ;01.
        let c = Label::parse("00:10:00;00").unwrap();
        df_label_legal(c, units).unwrap();
        // Round trip across the boundary.
        assert_eq!(frame_units_to_label(fb, true, units), b);
    }

    #[test]
    fn df_5994_skips_four_numbers() {
        let rate = Rate { numerator: 60000, denominator: 1001 };
        let units = rate.units_per_second().unwrap();
        assert_eq!(units, 60);
        let illegal = df_label_legal(Label::parse("00:01:00;02").unwrap(), units);
        assert_eq!(illegal, Err(vec![0, 1, 2, 3]));
        let first = Label::parse("00:01:00;04").unwrap();
        df_label_legal(first, units).unwrap();
        assert_eq!(
            label_to_frame_units(first, true, units),
            60 * 60 - 4
        );
    }

    #[test]
    fn midnight_wrap_keeps_two_readings() {
        let d = forward_deltas(
            label_to_frame_units(Label::parse("23:59:59:29").unwrap(), false, 30),
            label_to_frame_units(Label::parse("00:00:00:00").unwrap(), false, 30),
            30,
        );
        assert_eq!(d.forward_wrapped, 1);
        assert_eq!(d.unwrapped, -2_591_999);
    }
}
