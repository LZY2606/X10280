//! Quantities always carry their unit / coordinate basis.
//!
//! Importers keep the raw representation untouched; only the derived layer
//! normalises. Values that cannot be converted stay at their original
//! position and receive a diagnostic instead of being dropped.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const fn new(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Canonical display: reduced fraction as `num/den` (den == 1 -> integer).
    pub fn canonical(self) -> String {
        let (num, den) = self.normalized_pair();
        if den == 1 {
            num.to_string()
        } else {
            format!("{num}/{den}")
        }
    }

    pub fn normalized_pair(mut self) -> (i64, i64) {
        if self.den < 0 {
            self.num = -self.num;
            self.den = -self.den;
        }
        let g = gcd(self.num.unsigned_abs(), self.den.unsigned_abs()).max(1);
        (self.num / g as i64, self.den / g as i64)
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// A measured value kept together with its unit basis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub value: f64,
    pub unit: String,
    /// Original textual representation as submitted (never overwritten).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl Measure {
    pub fn new(value: f64, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
            raw: None,
        }
    }

    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }
}

/// Reference frame for a coordinate: which source timeline and which ordinal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameCoordinate {
    pub source_id: String,
    /// 0-based ordinal of the captured field/frame on that source.
    pub ordinal: i64,
    pub basis: String,
}
