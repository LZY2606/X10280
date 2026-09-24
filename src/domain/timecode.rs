//! SMPTE time-of-day timecode arithmetic.
//!
//! Everything here operates on label positions within a 24 hour label day.
//! Skipping labels (drop-frame numbering) is renumbering only: it never drops
//! any actual frame. [`FrameRate`] carries both the label rate and the media
//! (sample) rate as exact rationals so field modes (59.94 VITC vs 29.94
//! frames) remain distinguishable.

use serde::{Deserialize, Serialize};

/// Exact non-negative rational p/q. Preserved verbatim from input; the
/// normalized decimal is derived separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const fn new(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    pub fn reduce(self) -> Self {
        let g = gcd(self.num.unsigned_abs(), self.den.unsigned_abs()).max(1) as i64;
        let sign = if self.den < 0 { -1 } else { 1 };
        Rational::new(sign * self.num / g, sign * self.den / g)
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub fn mul_int(self, n: i64) -> i64 {
        self.num * n / self.den
    }
}

impl std::fmt::Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

pub(crate) fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Label rate (timecode labels per nominal second) and media sample rate
/// (actual frames or fields captured per real second), both exact rationals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    /// Label grid, e.g. 30/1, 24/1, 25/1, 60/1.
    pub labels_per_second: Rational,
    /// Media samples per real second, e.g. 30000/1001, 60000/1001.
    pub media_per_second: Rational,
    /// SMPTE drop-frame numbering flag (bit 10 of the frame units word).
    pub drop_frame: bool,
    /// `true` when the media rate counts fields (59.94) whose labels step on
    /// the half-frame grid of a 29.97/30 label rate.
    pub field_mode: bool,
}

impl FrameRate {
    pub const fn ntsc2997() -> Self {
        Self {
            labels_per_second: Rational::new(30, 1),
            media_per_second: Rational::new(30000, 1001),
            drop_frame: true,
            field_mode: false,
        }
    }

    pub const fn ntsc5994_field() -> Self {
        Self {
            labels_per_second: Rational::new(30, 1),
            media_per_second: Rational::new(60000, 1001),
            drop_frame: true,
            field_mode: true,
        }
    }

    pub const fn ntsc30nd() -> Self {
        Self {
            labels_per_second: Rational::new(30, 1),
            media_per_second: Rational::new(30, 1),
            drop_frame: false,
            field_mode: false,
        }
    }

    pub const fn pal25() -> Self {
        Self {
            labels_per_second: Rational::new(25, 1),
            media_per_second: Rational::new(25, 1),
            drop_frame: false,
            field_mode: false,
        }
    }

    /// Labels in one full 24 hour label day.
    pub fn labels_per_day(&self) -> i64 {
        self.labels_per_second.num * 60 * 60 * 24 / self.labels_per_second.den
    }

    /// Label steps per nominal minute.
    pub fn labels_per_minute(&self) -> i64 {
        self.labels_per_second.num * 60 / self.labels_per_second.den
    }

    /// Number of labels skipped at one drop-frame minute boundary, i.e. 2 for
    /// a 30/59.94 label rate, 4 for a 60-label rate, 0 when non-drop.
    pub fn skip_size(&self) -> i64 {
        if !self.drop_frame {
            0
        } else {
            // SMPTE 12M: two labels for the 30-label grid (29.97), four for
            // the 60-label grid (59.94 VITC field labels).
            self.labels_per_minute() / 15
        }
    }
}
