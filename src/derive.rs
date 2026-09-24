//! Derivation engine: builds reviewable timecode segments, skip
//! positions, bit-quality tables and fingerprint correspondence from
//! raw sources. Every output records algorithm version, parameter
//! fingerprint and direct inputs. Unconvertible values stay in place
//! with diagnostics.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::model::{content_hash, Diagnostic, Derivation, RawFrame, SourceVersion};
use crate::tc::{
    classify_transition, frame_to_tc, parse_tc, skipped_labels_at_minute, tc_to_frame,
    Discontinuity, FieldMode,
};

pub const SEGMENT_ALGORITHM: &str = "segment-builder";
pub const SEGMENT_ALGORITHM_VERSION: &str = "1.3.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentParams {
    /// How to resolve midnight-wrap ambiguity when a single timeline
    /// is required: "midnight-wrap" | "true-rollback" | "keep-both".
    pub midnight_wrap_resolution: String,
    /// Minimum run length (frames) to treat a reverse run as recording
    /// rather than decode jitter.
    pub reverse_run_min_frames: i64,
}

impl Default for SegmentParams {
    fn default() -> Self {
        SegmentParams {
            midnight_wrap_resolution: "keep-both".into(),
            reverse_run_min_frames: 2,
        }
    }
}

pub fn params_fingerprint<T: Serialize>(params: &T) -> String {
    let canonical = serde_json::to_string(params).unwrap_or_default();
    format!("pf_{}", content_hash(&[&canonical]))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub segment_id: String,
    pub start_label: String,
    pub end_label: String,
    pub start_frame: i64,
    pub end_frame: i64,
    /// +1 forward, -1 reverse recording.
    pub direction: i32,
    pub frame_count: i64,
    pub drop_frame: bool,
    pub nominal_fps: u32,
    /// Labels skipped by drop-frame numbering inside this segment.
    pub skipped_labels: Vec<String>,
    /// Positions (frame indices into the source) where user bits change.
    pub user_bit_changes: Vec<UserBitChange>,
    pub field_mode: String,
    pub mean_bit_quality: f64,
    /// True when the segment boundary involved an unresolved ambiguity
    /// (midnight wrap kept with two interpretations).
    pub ambiguous_boundary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBitChange {
    pub frame_index: usize,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentOutput {
    pub segments: Vec<Segment>,
    /// Per-frame bit quality table (frame index, label, quality).
    pub bit_quality: Vec<BitQualityRow>,
    /// All drop-frame skip positions with their labels.
    pub skip_positions: Vec<SkipPosition>,
    /// Diagnostics for values that could not be converted.
    pub diagnostics: Vec<Diagnostic>,
    /// Frame fingerprint -> timecode label correspondence.
    pub fingerprint_map: Vec<FingerprintRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitQualityRow {
    pub frame_index: usize,
    pub tc_label: String,
    pub bit_quality: u8,
    pub raw_bits: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkipPosition {
    pub after_frame_index: usize,
    pub skipped_labels: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintRow {
    pub frame_index: usize,
    pub frame_fingerprint: String,
    pub tc_label: String,
    pub frame_number: i64,
}

fn infer_field_mode(frames: &[RawFrame]) -> FieldMode {
    let mut flips = 0usize;
    let mut const1 = 0usize;
    for w in frames.windows(2) {
        if w[0].field_flag != w[1].field_flag {
            flips += 1;
        }
        if w[0].field_flag == 1 {
            const1 += 1;
        }
    }
    if frames.len() >= 4 && flips * 2 >= frames.len() {
        FieldMode::Field1First
    } else if const1 * 2 > frames.len() {
        FieldMode::Field2First
    } else if flips == 0 {
        FieldMode::Progressive
    } else {
        FieldMode::Unknown
    }
}

/// Build segments for one source version under the given params.
pub fn build_segments(sv: &SourceVersion, params: &SegmentParams) -> Derivation {
    let mut diagnostics: Vec<Diagnostic> = sv.diagnostics.clone();
    let mut bit_quality = Vec::new();
    let mut fingerprint_map = Vec::new();
    let mut skip_positions = Vec::new();

    // Parse labels; unconvertible values stay in place with diagnostics.
    let mut parsed: Vec<Option<(crate::tc::Timecode, bool)>> = Vec::new();
    for (i, f) in sv.frames.iter().enumerate() {
        let p = parse_tc(&f.tc_label);
        if p.is_none() {
            diagnostics.push(Diagnostic {
                frame_index: i,
                field: "tc_label".into(),
                raw_value: f.tc_label.clone(),
                message: "unparseable timecode label; kept in raw position".into(),
            });
        }
        parsed.push(p);
        bit_quality.push(BitQualityRow {
            frame_index: i,
            tc_label: f.tc_label.clone(),
            bit_quality: f.bit_quality,
            raw_bits: f.raw_bits.clone(),
        });
    }

    let nominal = sv.frames.first().map(|f| f.rate().nominal()).unwrap_or(30);
    let drop = sv.frames.first().map(|f| f.drop_frame).unwrap_or(false);

    // Split into segments at discontinuities.
    let mut segments: Vec<Segment> = Vec::new();
    let mut seg_start = 0usize;
    let mut direction: i32 = 1;
    let mut user_bit_changes: Vec<UserBitChange> = Vec::new();
    let mut ambiguous_boundary = false;

    let close_segment = |sv: &SourceVersion,
                         parsed: &Vec<Option<(crate::tc::Timecode, bool)>>,
                         start: usize,
                         end: usize,
                         direction: i32,
                         ubc: &[UserBitChange],
                         ambiguous: bool,
                         segments: &mut Vec<Segment>,
                         skip_positions: &mut Vec<SkipPosition>| {
        if end <= start {
            return;
        }
        let frames = &sv.frames[start..end];
        let first = parsed[start];
        let last = parsed[end - 1];
        let (Some((first_tc, _)), Some((last_tc, last_drop))) = (first, last) else {
            return;
        };
        let nominal = frames[0].rate().nominal();
        let drop = frames[0].drop_frame;
        let start_frame = tc_to_frame(&first_tc, nominal, drop);
        let end_frame = tc_to_frame(&last_tc, nominal, drop);
        let mut skipped = Vec::new();
        // enumerate drop-frame skipped labels inside the segment span
        let (lo, hi) = if start_frame <= end_frame {
            (start_frame, end_frame)
        } else {
            (end_frame, start_frame)
        };
        let mut n = lo;
        let mut last_min_seen: Option<(u32, u32)> = None;
        while n <= hi {
            let tc = frame_to_tc(n, nominal, drop);
            let key = (tc.h, tc.m);
            if last_min_seen != Some(key) {
                last_min_seen = Some(key);
                for lab in skipped_labels_at_minute(tc.h, tc.m, nominal, drop) {
                    skipped.push(lab.fmt_df(drop));
                }
            }
            n += 1;
        }
        if !skipped.is_empty() {
            skip_positions.push(SkipPosition {
                after_frame_index: start,
                skipped_labels: skipped.clone(),
                note: "drop-frame numbering: labels skipped, no frames dropped".into(),
            });
        }
        let mean_q = frames.iter().map(|f| f.bit_quality as f64).sum::<f64>()
            / frames.len().max(1) as f64;
        let seg = Segment {
            segment_id: format!(
                "seg_{}",
                content_hash(&[&sv.id, &start.to_string(), &end.to_string()])
            ),
            start_label: first_tc.fmt_df(drop),
            end_label: last_tc.fmt_df(last_drop),
            start_frame,
            end_frame,
            direction,
            frame_count: (end_frame - start_frame).abs() + 1,
            drop_frame: drop,
            nominal_fps: nominal,
            skipped_labels: skipped,
            user_bit_changes: ubc.to_vec(),
            field_mode: crate::model::field_mode_label(&infer_field_mode(frames)).into(),
            mean_bit_quality: (mean_q * 100.0).round() / 100.0,
            ambiguous_boundary: ambiguous,
        };
        segments.push(seg);
    };

    for i in 1..=sv.frames.len() {
        let boundary = if i == sv.frames.len() {
            Some(Discontinuity::Continuous) // force close
        } else {
            match (&parsed[i - 1], &parsed[i]) {
                (Some((a, _)), Some((b, _))) => {
                    let d = classify_transition(a, b, nominal, drop);
                    match d {
                        Discontinuity::Continuous => None,
                        other => Some(other),
                    }
                }
                _ => Some(Discontinuity::ForwardGap { missing_frames: 0 }),
            }
        };
        // user bits change tracking (within current segment)
        if i < sv.frames.len() && sv.frames[i].user_bits != sv.frames[i - 1].user_bits {
            user_bit_changes.push(UserBitChange {
                frame_index: i,
                from: sv.frames[i - 1].user_bits.clone(),
                to: sv.frames[i].user_bits.clone(),
            });
        }
        if let Some(disc) = boundary {
            let is_end = i == sv.frames.len();
            let split_here = match &disc {
                Discontinuity::Continuous => is_end,
                Discontinuity::MidnightWrapAmbiguous { .. } => {
                    ambiguous_boundary = true;
                    match params.midnight_wrap_resolution.as_str() {
                        // keep-both: do not split, flag the segment
                        "keep-both" => is_end,
                        _ => is_end,
                    }
                }
                Discontinuity::ReverseRun { delta_frames } => {
                    if delta_frames.abs() >= params.reverse_run_min_frames {
                        true
                    } else {
                        is_end
                    }
                }
                Discontinuity::ForwardGap { .. } => true,
            };
            if split_here {
                let end = if is_end { i } else { i };
                close_segment(
                    sv, parsed, seg_start, end, direction, &user_bit_changes,
                    ambiguous_boundary, &mut segments, &mut skip_positions,
                );
                seg_start = i;
                user_bit_changes = Vec::new();
                ambiguous_boundary = false;
                if let Discontinuity::ReverseRun { .. } = disc {
                    direction = -direction;
                } else {
                    direction = 1;
                }
            }
        }
    }

    // fingerprint correspondence (frame number under segment-local parse)
    for (i, f) in sv.frames.iter().enumerate() {
        let frame_number = match &parsed[i] {
            Some((tc, d)) => tc_to_frame(tc, nominal, *d),
            None => -1,
        };
        fingerprint_map.push(FingerprintRow {
            frame_index: i,
            frame_fingerprint: f.frame_fingerprint.clone(),
            tc_label: f.tc_label.clone(),
            frame_number,
        });
    }

    let output = SegmentOutput {
        segments,
        bit_quality,
        skip_positions,
        diagnostics,
        fingerprint_map,
    };
    let pfp = params_fingerprint(params);
    Derivation {
        id: format!(
            "der_{}",
            content_hash(&[
                SEGMENT_ALGORITHM,
                SEGMENT_ALGORITHM_VERSION,
                &pfp,
                &sv.id,
            ])
        ),
        algorithm: SEGMENT_ALGORITHM.into(),
        algorithm_version: SEGMENT_ALGORITHM_VERSION.into(),
        params_fingerprint: pfp,
        input_version_ids: vec![sv.id.clone()],
        output: serde_json::to_value(&output).unwrap_or(json!({})),
        status: "current".into(),
        created_tick: 0,
    }
}
