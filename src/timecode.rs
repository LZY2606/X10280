//! SMPTE 时码核心计算：帧率有理数、drop-frame 跳号、24h 午夜回绕、反向录制。
//! 所有换算只在派生层使用；原始层永远保留导入时的原表示。

use serde::{Deserialize, Serialize};

/// 帧率有理数，例如 30000/1001 (29.97) 或 60000/1001 (59.94)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: i64,
    pub den: i64,
}

impl FrameRate {
    pub fn new(num: i64, den: i64) -> Result<Self, String> {
        if num <= 0 || den <= 0 {
            return Err(format!("帧率有理数必须为正: {num}/{den}"));
        }
        Ok(FrameRate { num, den })
    }
    /// 标称帧率（四舍五入到整数），用于时码编号。
    pub fn nominal(&self) -> i64 {
        (2 * self.num + self.den) / (2 * self.den)
    }
    /// 每分钟跳号个数：标称 30 -> 2，标称 60 -> 4。仅编号跳号，不丢任何真实帧。
    pub fn drop_count(&self) -> i64 {
        match self.nominal() {
            30 => 2,
            60 => 4,
            _ => 0,
        }
    }
    pub fn drop_capable(&self) -> bool {
        matches!(self.nominal(), 30 | 60)
    }
    /// 精确秒数（有理数基准），用于把帧差换算成带单位的量。
    pub fn seconds_per_frame_milli(&self) -> (i64, i64) {
        // 返回 (分子, 分母) 表示 秒/帧 * 1_000_000
        (self.den * 1_000_000, self.num)
    }
}

/// 一段时码 (h, m, s, f)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Tc {
    pub h: i64,
    pub m: i64,
    pub s: i64,
    pub f: i64,
}

impl Tc {
    pub fn to_string(&self, drop: bool) -> String {
        let sep = if drop { ";" } else { ":" };
        format!("{:02}:{:02}:{:02}{}{:02}", self.h, self.m, self.s, sep, self.f)
    }
}

/// 24 小时内的总帧数（drop-frame 时扣除跳号）。
pub fn frames_per_24h(rate: FrameRate, drop: bool) -> i64 {
    let nominal = rate.nominal();
    let d = if drop { rate.drop_count() } else { 0 };
    nominal * 86_400 - d * (1_440 - 144)
}

/// 时码 -> 24h 内帧号。drop-frame 下跳过不存在的编号（如 00:01:00;00/01）。
pub fn tc_to_frame(tc: Tc, rate: FrameRate, drop: bool) -> Result<i64, String> {
    let nominal = rate.nominal();
    if tc.h < 0 || tc.h > 23 || tc.m < 0 || tc.m > 59 || tc.s < 0 || tc.s > 59 || tc.f < 0 || tc.f >= nominal {
        return Err(format!("时码越界: {}", tc.to_string(drop)));
    }
    let total_min = 60 * tc.h + tc.m;
    let dropped = if drop {
        let d = rate.drop_count();
        if d == 0 {
            return Err("该帧率不支持 drop-frame".to_string());
        }
        // 00 分以外的每分钟前 d 个编号不存在
        if tc.s == 0 && tc.f < d && tc.m % 10 != 0 {
            return Err(format!("drop-frame 跳号编号不存在: {}", tc.to_string(true)));
        }
        d * (total_min - total_min / 10)
    } else {
        0
    };
    Ok((tc.h * 3600 + tc.m * 60 + tc.s) * nominal + tc.f - dropped)
}

/// 24h 内帧号 -> 时码。
pub fn frame_to_tc(frame: i64, rate: FrameRate, drop: bool) -> Tc {
    let nominal = rate.nominal();
    let fp24 = frames_per_24h(rate, drop);
    let n = frame.rem_euclid(fp24);
    if !drop {
        let h = n / (nominal * 3600);
        let m = (n / (nominal * 60)) % 60;
        let s = (n / nominal) % 60;
        let f = n % nominal;
        return Tc { h, m, s, f };
    }
    let d = rate.drop_count();
    let fp10 = nominal * 600 - 9 * d; // 每 10 分钟的帧数
    let fpm = nominal * 60 - d; // 非整 10 分钟里每分钟的帧数
    let ten = n / fp10;
    let rem = n % fp10;
    let (min_in_block, sec, frac) = if rem < nominal * 60 {
        (0, rem / nominal, rem % nominal)
    } else {
        let r = rem - nominal * 60;
        let min_in_block = 1 + r / fpm;
        let off = r % fpm;
        (min_in_block, off / nominal, off % nominal)
    };
    let f = if min_in_block == 0 { frac } else { frac + d };
    Tc { h: ten / 6, m: (ten % 6) * 10 + min_in_block, s: sec, f }
}

/// 解析 LTC 80-bit（20 个十六进制字符）为时码分量。
/// 返回 (Tc, user_bits 8hex, drop_frame, color_frame, field_flag)。
/// 这是派生层解析；原始比特在导入层已原样保留。
/// 位布局（每字节低 4 位为分量、高 4 位为 user bits 组）：
/// byte0: frame units | user1 ; byte1: frame tens(2) df(1) color(1) | user2
/// byte2: sec units | user3   ; byte3: sec tens(3) field(1) | user4
/// byte4: min units | user5   ; byte5: min tens(3) biphase(1) | user6
/// byte6: hour units | user7  ; byte7: hour tens(2) reserved(2) | user8
/// 末尾 16 bit 为同步字 0x3FFD。
pub fn parse_ltc_bits(bits: &str) -> Result<ParsedLtc, String> {
    let b = bits.trim();
    if b.len() != 20 || !b.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("LTC 原比特须为 20 个十六进制字符, 实际 {} 字符", b.len()));
    }
    let v = u64::from_str_radix(&b[..16], 16).map_err(|e| e.to_string())?;
    let sync = u16::from_str_radix(&b[16..], 16).map_err(|e| e.to_string())?;
    if sync != 0x3FFD {
        return Err(format!("LTC 同步字错误: 0x{sync:04X} (期望 0x3FFD)"));
    }
    let byte = |i: u32| ((v >> (i * 8)) & 0xFF) as i64;
    let f = (byte(0) & 0xF) + ((byte(1) & 0x3)) * 10;
    let df = (byte(1) >> 2) & 1 == 1;
    let cf = (byte(1) >> 3) & 1 == 1;
    let s = (byte(2) & 0xF) + ((byte(3) & 0x7)) * 10;
    let field = (byte(3) >> 3) & 1;
    let m = (byte(4) & 0xF) + ((byte(5) & 0x7)) * 10;
    let h = (byte(6) & 0xF) + ((byte(7) & 0x3)) * 10;
    let user = (0..8)
        .map(|i| format!("{:X}", (v >> (4 + i * 8)) & 0xF))
        .collect::<Vec<_>>()
        .join("");
    Ok(ParsedLtc {
        tc: Tc { h, m, s, f },
        user_bits: user,
        drop_frame: df,
        color_frame: cf,
        field_flag: field,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedLtc {
    pub tc: Tc,
    pub user_bits: String,
    pub drop_frame: bool,
    pub color_frame: bool,
    pub field_flag: i64,
}

/// 由分量合成 LTC 80-bit（用于案例包构造确定性样例）。
pub fn build_ltc_bits(tc: Tc, user_bits: &str, drop: bool, color: bool, field: i64) -> String {
    let u: Vec<i64> = user_bits
        .chars()
        .map(|c| i64::from_str_radix(&c.to_string(), 16).unwrap_or(0))
        .collect();
    let u = |i: usize| *u.get(i).unwrap_or(&0);
    let mut v: u64 = 0;
    let mut byte = |i: u32, val: i64| v |= ((val & 0xFF) as u64) << (i * 8);
    byte(0, tc.f % 10);
    byte(1, (tc.f / 10) & 0x3 | if drop { 0x4 } else { 0 } | if color { 0x8 } else { 0 });
    byte(2, tc.s % 10);
    byte(3, (tc.s / 10) & 0x7 | (field & 1) << 3);
    byte(4, tc.m % 10);
    byte(5, (tc.m / 10) & 0x7);
    byte(6, tc.h % 10);
    byte(7, (tc.h / 10) & 0x3);
    for i in 0..8 {
        v |= ((u(i) & 0xF) as u64) << (4 + i * 8);
    }
    format!("{:016X}3FFD", v)
}

/// 相邻两帧的不连续性判定。所有假设都保留，由上层决定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Discontinuity {
    /// 正常连续（含 drop-frame 分钟边界的纯编号跳号）。
    Continuous { numbering_skip: bool },
    /// 帧号回退：可能是午夜回绕（前进跨过 24h），也可能是真实回退/反向录制。两解都保留。
    Decrease { hypotheses: Vec<String> },
    /// 帧号前跳：断码后续接，缺失区间未知。
    Gap { missing_frames: i64 },
}

pub fn classify_step(prev_frame: i64, next_frame: i64, rate: FrameRate, drop: bool) -> Discontinuity {
    let fp24 = frames_per_24h(rate, drop);
    let delta = next_frame - prev_frame;
    if delta == 1 {
        // 检查是否跨越 drop-frame 分钟边界（编号跳号 ≠ 丢帧）
        let pt = frame_to_tc(prev_frame, rate, drop);
        let nt = frame_to_tc(next_frame, rate, drop);
        let numbering_skip = drop && pt.m != nt.m && nt.m % 10 != 0;
        return Discontinuity::Continuous { numbering_skip };
    }
    if delta == -(fp24 - 1) {
        return Discontinuity::Decrease {
            hypotheses: vec!["midnight_wrap_forward".to_string(), "true_rollback".to_string()],
        };
    }
    if delta < 0 {
        return Discontinuity::Decrease {
            hypotheses: vec!["reverse_recording".to_string(), "timecode_rollback".to_string()],
        };
    }
    Discontinuity::Gap { missing_frames: delta - 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r2997() -> FrameRate { FrameRate { num: 30000, den: 1001 } }
    fn r5994() -> FrameRate { FrameRate { num: 60000, den: 1001 } }

    #[test]
    fn drop_frame_minute_boundary_skips_numbers_not_frames() {
        let r = r2997();
        // 00:00:59;29 之后是 00:01:00;02 —— 编号跳过 00/01，但帧号连续。
        let a = tc_to_frame(Tc { h: 0, m: 0, s: 59, f: 29 }, r, true).unwrap();
        let b = tc_to_frame(Tc { h: 0, m: 1, s: 0, f: 2 }, r, true).unwrap();
        assert_eq!(b - a, 1, "跳号只是编号规则，帧计数必须连续");
        // 被跳过的编号不存在
        assert!(tc_to_frame(Tc { h: 0, m: 1, s: 0, f: 0 }, r, true).is_err());
        assert!(tc_to_frame(Tc { h: 0, m: 1, s: 0, f: 1 }, r, true).is_err());
        // 每 10 分钟不跳号
        assert_eq!(tc_to_frame(Tc { h: 0, m: 10, s: 0, f: 0 }, r, true).unwrap(), 17982);
    }

    #[test]
    fn drop_frame_5994_skips_four() {
        let r = r5994();
        assert_eq!(r.drop_count(), 4);
        assert!(tc_to_frame(Tc { h: 0, m: 1, s: 0, f: 3 }, r, true).is_err());
        let a = tc_to_frame(Tc { h: 0, m: 0, s: 59, f: 59 }, r, true).unwrap();
        let b = tc_to_frame(Tc { h: 0, m: 1, s: 0, f: 4 }, r, true).unwrap();
        assert_eq!(b - a, 1);
    }

    #[test]
    fn roundtrip_all_day() {
        let r = r2997();
        for base in [0, 1, 1797, 1798, 1799, 1800, 17981, 17982, 2_589_407, 2_589_408 - 1] {
            let tc = frame_to_tc(base, r, true);
            assert_eq!(tc_to_frame(tc, r, true).unwrap(), base, "tc={tc:?}");
        }
        assert_eq!(frames_per_24h(r, true), 2_589_408);
    }

    #[test]
    fn midnight_wrap_vs_rollback_keeps_two_hypotheses() {
        let r = r2997();
        let fp24 = frames_per_24h(r, true);
        match classify_step(fp24 - 1, 0, r, true) {
            Discontinuity::Decrease { hypotheses } => {
                assert!(hypotheses.contains(&"midnight_wrap_forward".to_string()));
                assert!(hypotheses.contains(&"true_rollback".to_string()));
            }
            other => panic!("期望两解, 得到 {other:?}"),
        }
    }

    #[test]
    fn reverse_recording_detected() {
        let r = r2997();
        match classify_step(5000, 4999, r, true) {
            Discontinuity::Decrease { hypotheses } => {
                assert!(hypotheses.contains(&"reverse_recording".to_string()));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ltc_bits_roundtrip() {
        let bits = build_ltc_bits(Tc { h: 1, m: 2, s: 3, f: 4 }, "0A1B2C3D", true, true, 1);
        let p = parse_ltc_bits(&bits).unwrap();
        assert_eq!(p.tc, Tc { h: 1, m: 2, s: 3, f: 4 });
        assert_eq!(p.user_bits, "0A1B2C3D");
        assert!(p.drop_frame && p.color_frame && p.field_flag == 1);
        assert!(parse_ltc_bits("ZZZZ").is_err());
        assert!(parse_ltc_bits(&bits[..18]).is_err());
    }
}
