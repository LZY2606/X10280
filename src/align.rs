//! Multi-source alignment solver. Produces offset candidates with
//! residuals and valid segments; every candidate, rejection reason,
//! error and unknown state is first-class data. When evidence does
//! not uniquely determine an offset, all legal candidates are kept.

use serde::{Deserialize, Serialize};

use crate::derive::{params_fingerprint, SegmentOutput};
use crate::model::{content_hash, Derivation, SourceVersion};
use crate::tc::{parse_tc, tc_to_frame};

pub const ALIGN_ALGORITHM: &str = "alignment-solver";
pub const ALIGN_ALGORITHM_VERSION: &str = "1.2.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignParams {
    /// Residuals within this epsilon (ms) are considered equally good:
    /// all such candidates are retained (equal-split).
    pub equal_split_epsilon_ms: f64,
    /// Minimum number of anchor pairs for a candidate to be viable.
    pub min_anchor_support: usize,
}

impl Default for AlignParams {
    fn default() -> Self {
        AlignParams {
            equal_split_epsilon_ms: 0.5,
            min_anchor_support: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignmentCandidate {
    pub candidate_id: String,
    /// Offset in frames: reference_frame_number = query_frame_index + offset.
    pub offset_frames: i64,
    /// Mean absolute residual of the anchors, in milliseconds.
    pub residual_ms: f64,
    /// Per-anchor residuals (frames) for audit.
    pub anchor_residuals: Vec<f64>,
    /// Valid segment on the query timeline (frame-index range).
    pub valid_range: [usize; 2],
    pub anchor_count: usize,
    /// pending | rejected | retained_alternative
    pub state: String,
    /// Why a candidate was rejected, if it was.
    pub rejection_reason: Option<String>,
    /// True when this candidate ties with others within epsilon.
    pub equal_split: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignmentOutput {
    pub reference_source: String,
    pub query_source: String,
    pub candidates: Vec<AlignmentCandidate>,
    /// Anchors that matched no candidate cluster (unknown state).
    pub unmatched_anchors: Vec<UnmatchedAnchor>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnmatchedAnchor {
    pub query_frame_index: usize,
    pub frame_fingerprint: String,
    pub reason: String,
}

struct Anchor {
    query_index: usize,
    ref_frame_number: i64,
    query_clock_ns: i64,
    ref_clock_ns: i64,
}

/// Solve alignment of `query` against `reference` using frame
/// fingerprint correspondence and device clocks.
pub fn solve_alignment(
    reference: &SourceVersion,
    query: &SourceVersion,
    ref_segments: &SegmentOutput,
    params: &AlignParams,
) -> Derivation {
    // Build fingerprint -> frame number map from the reference.
    let mut ref_map: std::collections::HashMap<String, Vec<(i64, i64)>> =
        std::collections::HashMap::new();
    for row in &ref_segments.fingerprint_map {
        if row.frame_number >= 0 {
            let clock = reference.frames[row.frame_index].device_clock_ns;
            ref_map
                .entry(row.frame_fingerprint.clone())
                .or_default()
                .push((row.frame_number, clock));
        }
    }

    let nominal = query.frames.first().map(|f| f.rate().nominal()).unwrap_or(30);
    let drop = query.frames.first().map(|f| f.drop_frame).unwrap_or(false);
    let frame_ns = query
        .frames
        .first()
        .map(|f| f.rate().frame_duration_ns())
        .unwrap_or(33_366_700.0);

    let mut anchors: Vec<Anchor> = Vec::new();
    let mut unmatched: Vec<UnmatchedAnchor> = Vec::new();
    for (qi, qf) in query.frames.iter().enumerate() {
        let Some(entries) = ref_map.get(&qf.frame_fingerprint) else {
            unmatched.push(UnmatchedAnchor {
                query_frame_index: qi,
                frame_fingerprint: qf.frame_fingerprint.clone(),
                reason: "no reference frame with this fingerprint".into(),
            });
            continue;
        };
        // A fingerprint may repeat in the reference (e.g. slate frames):
        // each occurrence is a legal anchor, producing equal-split
        // candidates rather than a forced pick.
        for &(ref_frame, ref_clock) in entries {
            let q_frame = match parse_tc(&qf.tc_label) {
                Some((tc, d)) => tc_to_frame(&tc, nominal, d),
                None => qi as i64,
            };
            let _ = q_frame;
            anchors.push(Anchor {
                query_index: qi,
                ref_frame_number: ref_frame,
                query_clock_ns: qf.device_clock_ns,
                ref_clock_ns: ref_clock,
            });
        }
    }

    // Cluster anchors by offset.
    let mut clusters: std::collections::BTreeMap<i64, Vec<&Anchor>> =
        std::collections::BTreeMap::new();
    for a in &anchors {
        let offset = a.ref_frame_number - a.query_index as i64;
        clusters.entry(offset).or_default().push(a);
    }

    let mut candidates: Vec<AlignmentCandidate> = Vec::new();
    for (offset, group) in &clusters {
        let n = group.len();
        // Residual from device clocks: predicted ref clock given offset.
        let residuals: Vec<f64> = group
            .iter()
            .map(|a| {
                let predicted = a.query_clock_ns as f64 + *offset as f64 * frame_ns;
                (a.ref_clock_ns as f64 - predicted) / 1e6 // ms
            })
            .collect();
        let residual_ms = if residuals.is_empty() {
            0.0
        } else {
            residuals.iter().map(|r| r.abs()).sum::<f64>() / residuals.len() as f64
        };
        let min_q = group.iter().map(|a| a.query_index).min().unwrap_or(0);
        let max_q = group.iter().map(|a| a.query_index).max().unwrap_or(0);
        let (state, rejection_reason) = if n < params.min_anchor_support {
            (
                "rejected".to_string(),
                Some(format!(
                    "anchor support {n} below minimum {}",
                    params.min_anchor_support
                )),
            )
        } else {
            ("pending".to_string(), None)
        };
        candidates.push(AlignmentCandidate {
            candidate_id: format!(
                "cand_{}",
                content_hash(&[
                    &reference.id,
                    &query.id,
                    &offset.to_string(),
                ])
            ),
            offset_frames: *offset,
            residual_ms: (residual_ms * 1000.0).round() / 1000.0,
            anchor_residuals: residuals.iter().map(|r| (r * 1000.0).round() / 1000.0).collect(),
            valid_range: [min_q, max_q],
            anchor_count: n,
            state,
            rejection_reason,
            equal_split: false,
        });
    }

    // Equal-split: viable candidates whose residuals tie within epsilon
    // are all retained as legal alternatives.
    let viable: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.state == "pending")
        .map(|(i, _)| i)
        .collect();
    if viable.len() > 1 {
        let best = viable
            .iter()
            .map(|&i| candidates[i].residual_ms)
            .fold(f64::INFINITY, f64::min);
        let tied: Vec<usize> = viable
            .iter()
            .copied()
            .filter(|&i| (candidates[i].residual_ms - best).abs() <= params.equal_split_epsilon_ms)
            .collect();
        if tied.len() > 1 {
            for &i in &tied {
                candidates[i].equal_split = true;
                candidates[i].state = "retained_alternative".into();
            }
        }
    }

    let output = AlignmentOutput {
        reference_source: reference.id.clone(),
        query_source: query.id.clone(),
        candidates,
        unmatched_anchors: unmatched,
        note: "candidates carry residuals and valid segments; tied legal candidates are all retained".into(),
    };
    let pfp = params_fingerprint(params);
    Derivation {
        id: format!(
            "der_{}",
            content_hash(&[
                ALIGN_ALGORITHM,
                ALIGN_ALGORITHM_VERSION,
                &pfp,
                &reference.id,
                &query.id,
            ])
        ),
        algorithm: ALIGN_ALGORITHM.into(),
        algorithm_version: ALIGN_ALGORITHM_VERSION.into(),
        params_fingerprint: pfp,
        input_version_ids: vec![reference.id.clone(), query.id.clone()],
        output: serde_json::to_value(&output).unwrap_or(serde_json::json!({})),
        status: "current".into(),
        created_tick: 0,
    }
}
