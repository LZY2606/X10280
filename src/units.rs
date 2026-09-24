//! 领域量：全部携带单位或坐标基准；导入时保留原始表示，派生层才规范化。
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// 帧率有理数 rate_num/rate_den 帧/秒（坐标基准：墙钟秒）。
/// 例如 30000/1001 = 29.97，60000/1001 = 59.94，25/1 = 25。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: i64,
    pub den: i64,
}

impl FrameRate {
    pub const F2997: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const F5994: FrameRate = FrameRate { num: 60000, den: 1001 };
    pub const F30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const F25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const F24: FrameRate = FrameRate { num: 24, den: 1 };

    pub fn new(num: i64, den: i64) -> Self {
        let (n, d) = reduce(num, den);
        FrameRate { num: n, den: d }
    }
    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
    pub fn frames_per_10m(&self) -> i64 {
        // 10 分钟墙钟内的整数帧数（四舍五入到最近）
        ((self.num as f64 * 600.0) / self.den as f64).round() as i64
    }
    pub fn nominal_integer(&self) -> Option<i64> {
        // SMPTE 标签使用的“名义整数帧率”
        match (self.num, self.den) {
            (30000, 1001) => Some(30),
            (60000, 1001) => Some(60),
            (30, 1) => Some(30),
            (25, 1) => Some(25),
            (24, 1) => Some(24),
            _ => None,
        }
    }
}

impl std::fmt::Display for FrameRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

pub fn reduce(a: i64, b: i64) -> (i64, i64) {
    assert!(b != 0, "帧率分母不能为 0");
    let g = gcd(a.unsigned_abs(), b.unsigned_abs()) as i64;
    (a / g, b / g)
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// 设备时钟：相对于项目原点的纳秒数（单位 ns，坐标基准：设备单调时钟）。
/// 原始字符串同时保留在 raw 表示里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceClockNs(pub i128);

/// 墙钟时间戳：UNIX 纳秒，虚拟时间下由可注入时钟产生。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstantNs(pub i128);

/// 帧序号（捕获序号，单位=帧，坐标基准：该源流的首帧为 0 或 source_start）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seq(pub i64);

/// 一天之内的帧索引（帧，坐标基准：SMPTE 午夜）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayIndex(pub i64);

/// 两个信号之间的偏移（帧，坐标基准：参考源时间轴）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffsetFrames(pub i64);

/// 残差（帧，有符号：观测偏移减模型偏移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidualFrames(pub i64);

/// 置信度/品质 [0,1]，以千分位整数存储避免浮点不确定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct QualityMilli(pub u32);

impl QualityMilli {
    pub fn of(v: f64) -> Self {
        QualityMilli((v.clamp(0.0, 1.0) * 1000.0).round() as u32)
    }
    pub fn as_f64(&self) -> f64 {
        self.0 as f64 / 1000.0
    }
}

impl PartialOrd for FrameRate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for FrameRate {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.num * other.den).cmp(&(other.num * self.den))
    }
}
