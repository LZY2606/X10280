//! 帧率与时间偏移所用的精确有理数（分子/分母均为 i64，内部约分，分母恒正）。
//! 所有跨帧率比较都走有理数，避免 29.97/59.94 的浮点误差。

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::ops::{Add, Neg, Sub};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64, // 恒为正数
}

impl Rational {
    pub const fn new_raw(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "rational denominator must not be zero");
        let g = gcd(num.unsigned_abs(), den.unsigned_abs());
        let n = num / g as i64;
        let d = den / g as i64;
        if d < 0 {
            Self { num: -n, den: -d }
        } else {
            Self { num: n, den: d }
        }
    }

    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub const ONE: Self = Self { num: 1, den: 1 };

    pub fn from_int(n: i64) -> Self {
        Self::new(n, 1)
    }

    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    pub fn is_negative(&self) -> bool {
        self.num < 0
    }

    pub fn mul_i(&self, n: i64) -> Self {
        Self::new(self.num * n, self.den)
    }

    pub fn mul(&self, other: Self) -> Self {
        Self::new(self.num * other.num, self.den * other.den)
    }

    pub fn div(&self, other: Self) -> Self {
        assert!(other.num != 0);
        Self::new(self.num * other.den, self.den * other.num)
    }

    pub fn abs(&self) -> Self {
        Self { num: self.num.abs(), den: self.den }
    }

    pub fn to_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// 半帧（等分候选常用的 1/2）
    pub fn half(&self) -> Self {
        Self::new(self.num, self.den * 2)
    }

    /// 平方，保留有理精度
    pub fn square(&self) -> Self {
        Self::new(self.num * self.num, self.den * self.den)
    }

    pub fn as_json_string(&self) -> String {
        format!("{}/{}", self.num, self.den)
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if let Some((a, b)) = s.split_once('/') {
            let n: i64 = a.trim().parse().map_err(|_| format!("非法分子: {a}"))?;
            let d: i64 = b.trim().parse().map_err(|_| format!("非法分母: {b}"))?;
            if d == 0 {
                return Err("分母为零".into());
            }
            Ok(Self::new(n, d))
        } else {
            let n: i64 = s.parse().map_err(|_| format!("非法整数: {s}"))?;
            Ok(Self::from_int(n))
        }
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

impl Add for Rational {
    type Output = Rational;
    fn add(self, rhs: Rational) -> Rational {
        Rational::new(self.num * rhs.den + rhs.num * self.den, self.den * rhs.den)
    }
}

impl Sub for Rational {
    type Output = Rational;
    fn sub(self, rhs: Rational) -> Rational {
        Rational::new(self.num * rhs.den - rhs.num * self.den, self.den * rhs.den)
    }
}

impl Neg for Rational {
    type Output = Rational;
    fn neg(self) -> Rational {
        Rational { num: -self.num, den: self.den }
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.num * other.den).cmp(&(other.num * self.den))
    }
}

impl std::fmt::Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl std::fmt::Debug for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rational({}/{})", self.num, self.den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_arith_and_order() {
        let a = Rational::new(30000, 1001);
        let b = Rational::new(60000, 1001);
        assert_eq!(a + a, b);
        assert!(a < Rational::from_int(30));
        assert_eq!((b - a), a);
        assert_eq!(
            Rational::new(2, 3) - Rational::new(1, 6),
            Rational::new(1, 2)
        );
        assert_eq!(Rational::parse("30000/1001").unwrap(), a);
        assert_eq!(Rational::parse("-3").unwrap(), Rational::from_int(-3));
        assert!(Rational::parse("x/2").is_err());
        assert!(Rational::new(1, 2).half(), Rational::new(1, 4));
    }

    #[test]
    fn rational_denom_always_positive() {
        assert!(Rational::new(1, -2).den > 0);
        assert!(Rational::new(1, -2).num < 0);
    }
}
