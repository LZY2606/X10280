//! Derived layer: single-source sequence analysis and multi-source alignment.
//!
//! Every derived value carries an algorithm version, parameter fingerprint and
//! direct sources. Local input changes only invalidate dependent results; the
//! storage layer keeps older derivations readable (as-of replay) while marking
//! them stale.

use crate::hashing::Hasher;
use crate::model::*;
use crate::rat::{self, Rat};
use crate::storage::ALGO_VERSION;
use crate::timecode::{self, decode_code, CodeKind, DfSpec, Label};
use serde_json::json;

pub fn default_params() -> serde_json::Value {
    json!({
        "align_tolerance_ns": 250_000,
        "min_inliers": 2,
        "wrap_frames_threshold": 60,
        "note": "default derivation parameters"
    })
}

pub fn param_fingerprint(p: &serde_json::Value) -> String {
    let canon = canonical_json(p);
    Hasher::id(&["params", &canon])
}

/// Deterministic canonical JSON encoding (keys sorted, no spaces).
pub fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => serde_json::Value::String(s.clone()).to_string(),
        serde_json::Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        serde_json::Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), canonical_json(&o[*k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

struct Slot {
    obs: Observation,
    label: Option<Label>,
    frame_index: Option<i64>,
    user_bits: Option<String>,
    field_flag: Option<bool>,
    unconvertible: Option<String>,
}

fn prepare_slot(o: Observation) -> Slot {
    let mut label = None;
    let mut frame_index = None;
    let mut user_bits = None;
    let mut field_flag = None;
    let mut unconvertible = None;

    let nominal = o.rate.nominal_fps();

    // Prefer the preserved bit payload (true decoder path); raw tc is fallback.
    if let Some(bits) = &o.raw_bits {
        let kind = match o.kind {
            SourceKind::Vitc => CodeKind::Vitc,
            _ => CodeKind::Ltc,
        };
        match decode_code(kind, bits) {
            Ok(dec) => {
                label = Some(dec.label.clone());
                user_bits = Some(dec.user_bits);
                field_flag = dec.label.field_flag;
                match timecode::label_to_frames(&dec.label, nominal) {
                    Ok(fi) => frame_index = Some(fi),
                    Err(e) => unconvertible = Some(e),
                }
            }
            Err(e) => unconvertible = Some(e),
        }
    } else if let Some(tc) = &o.raw_tc {
        match Label::parse(tc) {
            Ok((mut lab, marker)) => {
                lab.drop_frame = marker || o.df;
                label = Some(lab.clone());
                match timecode::label_to_frames(&lab, nominal) {
                    Ok(fi) => frame_index = Some(fi),
                    Err(e) => unconvertible = Some(e),
                }
            }
            Err(e) => unconvertible = Some(e),
        }
    }

    // VITC field observations may also carry an explicit field attribute.
    if o.kind == SourceKind::Vitc {
        if let Some(fm) = match o.field_mode {
            FieldMode::FieldOne => Some(true),
            FieldMode::FieldTwo => Some(false),
            FieldMode::Progressive => None,
        } {
            field_flag = field_flag.or(Some(fm));
        }
    }

    Slot { obs: o, label, frame_index, user_bits, field_flag, unconvertible }
}

fn day_frames(o: &Observation) -> i64 {
    timecode::frames_per_day(o.rate.nominal_fps(), o.df)
}

fn analyze_pair(a: &Slot, b: &Slot, params: &serde_json::Value) -> Vec<Transition> {
    let wrap_thr: i64 = params["wrap_frames_threshold"].as_i64().unwrap_or(60);
    let day = day_frames(&a.obs);
    let rate_nom = a.obs.rate.nominal_fps();

    let clock_delta = match (a.obs.device_clock_ns, b.obs.device_clock_ns) {
        (Some(x), Some(y)) => Some(y - x),
        _ => None,
    };

    let mut out = Vec::new();

    match (a.frame_index, b.frame_index) {
        (Some(fa), Some(fb)) => {
            let delta = fb - fa;
            if delta == 1 {
                out.push(Transition {
                    from_seq: a.obs.seq,
                    to_seq: b.obs.seq,
                    kind: InterpretKind::Forward,
                    frame_delta: 1,
                    device_clock_delta_ns: clock_delta,
                    skipped_numbers: skipped_at_boundary(&b.label, rate_nom),
                    expected_ns: Some(rat::frames_to_ns(1, b.obs.rate)),
                    note: skip_note(&b.label, rate_nom),
                    rejected: vec![],
                });
            } else if delta > 1 {
                let missing = delta - 1;
                let expected = rat::frames_to_ns(delta, b.obs.rate);
                let mut rejected = Vec::new();
                // A large forward gap is a recording break, never a numbering skip.
                if missing > rate_nom {
                    rejected.push("gap exceeds one second: not drop-frame numbering".into());
                }
                out.push(Transition {
                    from_seq: a.obs.seq,
                    to_seq: b.obs.seq,
                    kind: InterpretKind::Broken,
                    frame_delta: delta,
                    device_clock_delta_ns: clock_delta,
                    skipped_numbers: 0,
                    expected_ns: Some(expected),
                    note: format!("{missing} real picture frame(s) absent between the two captured observations"),
                    rejected,
                });
            } else if delta == 0 {
                out.push(Transition {
                    from_seq: a.obs.seq, to_seq: b.obs.seq, kind: InterpretKind::Broken,
                    frame_delta: 0, device_clock_delta_ns: clock_delta, skipped_numbers: 0,
                    expected_ns: Some(0),
                    note: "identical timecode on adjacent observations".into(), rejected: vec![],
                });
            } else {
                // backward delta: ambiguous between true rewind and midnight wrap.
                let wrap_delta = delta + day;
                let forward_wrap_plausible = (0..=wrap_thr.max(rate_nom * 2)).contains(&wrap_delta);
                let clock_advances = clock_delta.map(|d| d > 0).unwrap_or(false);

                if forward_wrap_plausible {
                    // Solution 1: forward capture crossing midnight.
                    out.push(Transition {
                        from_seq: a.obs.seq, to_seq: b.obs.seq, kind: InterpretKind::MidnightWrap,
                        frame_delta: wrap_delta, device_clock_delta_ns: clock_delta,
                        skipped_numbers: skipped_at_boundary(&b.label, rate_nom),
                        expected_ns: Some(rat::frames_to_ns(wrap_delta, b.obs.rate)),
                        note: "solution A: forward recording across 24:00:00 wrap; label reset to 00:00:00".into(),
                        rejected: if clock_advances {
                            vec![]
                        } else {
                            vec!["device clock does not advance across the span".into()]
                        },
                    });
                    // Solution 2: genuine reverse (shuttling back / reverse record).
                    out.push(Transition {
                        from_seq: a.obs.seq, to_seq: b.obs.seq, kind: InterpretKind::Reverse,
                        frame_delta: delta, device_clock_delta_ns: clock_delta,
                        skipped_numbers: 0,
                        expected_ns: Some(rat::frames_to_ns(delta, b.obs.rate)),
                        note: format!("solution B: true rewind/reverse record of {} frame(s)", -delta),
                        rejected: if clock_advances {
                            vec!["device clock advances, consistent with forward capture".into()]
                        } else {
                            vec![]
                        },
                    });
                } else {
                    out.push(Transition {
                        from_seq: a.obs.seq, to_seq: b.obs.seq, kind: InterpretKind::Reverse,
                        frame_delta: delta, device_clock_delta_ns: clock_delta,
                        skipped_numbers: 0,
                        expected_ns: Some(rat::frames_to_ns(delta, b.obs.rate)),
                        note: format!("backward span of {} frames is too large for a midnight wrap", -delta),
                        rejected: vec!["midnight wrap exceeds threshold".into()],
                    });
                }
            }
        }
        _ => {
            out.push(Transition {
                from_seq: a.obs.seq, to_seq: b.obs.seq, kind: InterpretKind::Unconvertible,
                frame_delta: 0, device_clock_delta_ns: clock_delta, skipped_numbers: 0,
                expected_ns: None,
                note: "one side could not be converted to a frame index; value kept at origin".into(),
                rejected: vec![],
            });
        }
    }
    out
}

fn skipped_at_boundary(label: &Option<Label>, nominal: i64) -> i64 {
    if let Some(l) = label {
        if l.drop_frame && l.s == 0 && l.m % 10 != 0 {
            if let Some(spec) = DfSpec::for_rate(nominal) {
                return spec.skip();
            }
        }
    }
    0
}

fn skip_note(label: &Option<Label>, nominal: i64) -> String {
    if let Some(l) = label {
        if l.drop_frame && l.s == 0 && l.f == DfSpec::for_rate(nominal).map(|s| s.skip()).unwrap_or(0) as u8
            && l.m % 10 != 0
        {
            return format!(
                "minute boundary: frame numbers 0..{} omitted (numbering convention; no picture frame dropped)",
                DfSpec::for_rate(nominal).unwrap().skip()
            );
        }
    }
    String::new()
}
