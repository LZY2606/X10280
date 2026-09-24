//! Frame rates as exact rationals (unit: frames per second, base: wall-clock second).
//!
//! 29.97 is never represented as the decimal 29.97 internally; it is the exact
//! rational 30000/1001. Keeping the rational all the way through lets the drop
//! frame arithmetic be exact instead of drifting.

use serde::{Deserialize, Serialize};

/// Exact frame rate `num/den` frames per second. Always stored reduced.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Rate {
    pub num: i64,
    pub den: i64,
}

impl Rate {
    pub fn new(mut num: i64, mut den: i64) -> Result<Rate, String> {
        if den == 0 {
            return Err("frame-rate denominator must not be zero".to_string());
        }
        if num <= 0 {
            return Err("frame-rate numerator must be positive".to_string());
        }
        if den < 0 {
            num = -num;
            den = -den;
        }
        let g = gcd(num.unsigned_abs(), den.unsigned_abs()) as i64;
        Ok(Rate { num: num / g, den: den / g })
    }

    /// Parse the supported textual frame-rate representations, preserving the
    /// exact rational. Accepts "30000/1001", "29.97", "59.94", "30", "25", ...
    pub fn parse(text: &str) -> Result<Rate, String> {
        let t = text.trim();
        if let Some((a, b)) = t.split_once('/') {
            let n: i64 = a
                .trim()
                .parse()
                .map_err(|_| format!("invalid rational numerator: {a:?}"))?;
            let d: i64 = b
                .trim()
                .parse()
                .map_err(|_| format!("invalid rational denominator: {b:?}"))?;
            return Rate::new(n, d);
        }
        // Canonical decimal aliases for the fractional rates.
        let canonical = match t {
            "29.97" => Some((30000, 1001)),
            "59.94" => Some((60000, 1001)),
            "23.98" | "23.976" => Some((24000, 1001)),
            _ => None,
        };
        if let Some((n, d)) = canonical {
            return Rate::new(n, d);
        }
        if let Ok(n) = t.parse::<i64>() {
            return Rate::new(n, 1);
        }
        Err(format!("unrecognised frame-rate representation: {t:?}"))
    }

    /// Nominal/rounded integer rate used by SMPTE numbering (30, 60, 24, 25...).
    pub fn nominal(&self) -> i64 {
        (self.num as f64 / self.den as f64).round() as i64
    }

    /// Only 30000/1001 and 60000/1001 have a SMPTE drop-frame numbering.
    pub fn supports_drop_frame(&self) -> bool {
        matches!(
            (self.num, self.den),
            (30000, 1001) | (60000, 1001)
        )
    }

    /// Frames skipped at the boundary of minute `minute` (0..=9).
    /// `02:00` etc. are not skipped; every tenth minute keeps them.
    pub fn drop_count(&self, minute: i64, drop_frame: bool) -> i64 {
        if !drop_frame || !self.supports_drop_frame() {
            return 0;
        }
        let scale = if (self.num, self.den) == (60000, 1001) { 2 } else { 1 };
        if minute % 10 == 0 { 0 } else { 2 * scale }
    }

    /// Number of frame numbers dropped across the ten-minute blocks that begin
    /// the given minute (standard SMPTE formula generalised to 59.94).
    pub fn dropped_before_minute(&self, minute: i64, drop_frame: bool) -> i64 {
        if !drop_frame || !self.supports_drop_frame() {
            return 0;
        }
        let scale = if (self.num, self.den) == (60000, 1001) { 2 } else { 1 };
        // sum over minutes 0..minute of 2*scale*(minute%10!=0)
        let tens = minute / 10;
        let rem = minute % 10;
        (tens * 18 + rem * 2) * scale
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_exact_rationals() {
        assert_eq!(Rate::parse("29.97").unwrap(), Rate::new(30000, 1001).unwrap());
        assert_eq!(Rate::parse("59.94").unwrap(), Rate::new(60000, 1001).unwrap());
        assert_eq!(Rate::parse("30000/1001").unwrap(), Rate::parse("29.97").unwrap());
        assert_eq!(Rate::parse("30").unwrap().num, 30);
        assert!(Rate::parse("banana").is_err());
        assert!(Rate::new(1, 0).is_err());
    }

    #[test]
    fn drop_schedule() {
        let r = Rate::new(30000, 1001).unwrap();
        assert_eq!(r.drop_count(0, true), 0);
        assert_eq!(r.drop_count(1, true), 2);
        assert_eq!(r.drop_count(10, true), 0);
        // 18 dropped frame numbers per ten minutes.
        assert_eq!(r.dropped_before_minute(10, true), 18);
        let r2 = Rate::new(60000, 1001).unwrap();
        assert_eq!(r2.drop_count(1, true), 4);
        assert_eq!(r2.dropped_before_minute(10, true), 36);
        assert_eq!(r.drop_count(1, false), 0);
    }
}
