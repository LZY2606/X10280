//! Exact rational arithmetic for frame rates and frame durations.
//! Frame rates are stored as num/den frames-per-second (e.g. 30000/1001 for 29.97).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rat {
    pub num: i64,
    pub den: i64,
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.abs()
}

impl Rat {
    pub fn new(num: i64, den: i64) -> Self {
        assert!(den > 0, "rational denominator must be positive");
        let g = gcd(num, den);
        Rat { num: num / g, den: den / g }
    }
    pub fn parse(text: &str) -> Option<Self> {
        let (n, d) = text.split_once('/')?;
        Some(Rat::new(n.trim().parse().ok()?, d.trim().parse().ok()?))
    }
    pub fn to_string_raw(&self) -> String {
        format!("{}/{}", self.num, self.den)
    }
    pub fn approximate(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
    /// nominal integer frames per second (rounding), e.g. 30000/1001 -> 30
    pub fn nominal_fps(&self) -> i64 {
        (self.num + self.den / 2) / self.den
    }
    pub fn mul_int(&self, n: i64) -> Rat {
        Rat::new(self.num * n, self.den)
    }
}

impl PartialOrd for Rat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some((self.num * other.den).cmp(&(other.num * self.den)))
    }
}

/// Frame duration in nanoseconds as a rational: 1e9 * rate.den / rate.num.
pub fn frame_duration_ns_rat(rate: Rat) -> Rat {
    Rat::new(1_000_000_000i64 * rate.den, rate.num)
}

/// Convert `frames` frame-counts to nanoseconds, rounded to nearest integer.
pub fn frames_to_ns(frames: i128, rate: Rat) -> i64 {
    let num = frames * 1_000_000_000i128 * rate.den as i128;
    let den = rate.num as i128;
    let q = num / den;
    let r = num % den;
    if r * 2 >= den {
        (q + 1) as i64
    } else {
        q as i64
    }
}

/// Exact residual as rational nanoseconds for `(a - b)` given per-rate frame counts.
pub fn ns_diff_rat(frames_a: i64, rate_a: Rat, frames_b: i64, rate_b: Rat) -> Rat {
    let da = frame_duration_ns_rat(rate_a).mul_int(frames_a);
    let db = frame_duration_ns_rat(rate_b).mul_int(frames_b);
    Rat::new(da.num * db.den - db.num * da.den, da.den * db.den)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_basics() {
        let r = Rat::new(30000, 1001);
        assert_eq!(r.nominal_fps(), 30);
        assert!((r.approximate() - 29.97).abs() < 0.001);
        // one 29.97 frame ≈ 33366.67 us? frame is 1001/30000 s = 33366.67 ns? actually µs
        let ns = frames_to_ns(1, r);
        assert_eq!(ns, 33367);
    }
}
