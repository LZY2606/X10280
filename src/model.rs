//! Preserved inputs, machine-readable events and derived result records.

use crate::rat::Rat;
use serde::{Deserialize, Serialize};

// ---- preserved raw input --------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Ltc,
    Vitc,
    Video,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FieldMode {
    Progressive,
    FieldOne,
    FieldTwo,
}

/// One preserved observation. Every original textual representation is kept
/// verbatim in `raw_*`; normalized values live in the derived layer only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub source_id: String,
    pub seq: i64,
    pub kind: SourceKind,
    pub raw_line: String,
    pub raw_bits: Option<String>,
    pub raw_rate: String,
    pub raw_tc: Option<String>,
    pub raw_device_clock: Option<String>,
    pub device_clock_ns: Option<i64>,
    pub fingerprint: Option<String>,
    pub rate: Rat,
    pub df: bool,
    pub field_mode: FieldMode,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub rate_raw: String,
    pub rate: Rat,
    pub df: bool,
    pub color_frame: bool,
    pub created_event: String,
}

// ---- machine-readable events ----------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub at_ns: i64,
    pub kind: String,
    pub parent_event_id: Option<String>,
    pub payload: serde_json::Value,
    /// direct-source object ids (source/observation/param/decision)
    pub sources: Vec<String>,
    pub algo_version: Option<String>,
    pub param_fingerprint: Option<String>,
    pub result_id: Option<String>,
    pub superseded_by: Option<String>,
}

// ---- derived outputs -------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterpretKind {
    Forward,
    Reverse,
    MidnightWrap,
    Broken,
    Unconvertible,
}

/// One adjacency interpretation. Ambiguous adjacencies keep *all* legal
/// interpretations (e.g. small backward jump => both true rewind and a forward
/// midnight wrap) until a human decision resolves them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub from_seq: i64,
    pub to_seq: i64,
    pub kind: InterpretKind,
    pub frame_delta: i64,
    pub device_clock_delta_ns: Option<i64>,
    pub skipped_numbers: i64,
    pub expected_ns: Option<i64>,
    pub note: String,
    pub rejected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimecodeSegment {
    pub source_id: String,
    pub start_seq: i64,
    pub end_seq: i64,
    pub start_label: String,
    pub end_label: String,
    pub direction: String,
    pub frame_count: i64,
    pub field_mode: FieldMode,
    pub user_bits_seen: Vec<String>,
    pub continuation_of: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakPoint {
    pub source_id: String,
    pub seq: i64,
    pub before_label: Option<String>,
    pub after_label: Option<String>,
    pub missing_frames: i64,
    pub kind: String,
    pub candidate_continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkipPosition {
    pub source_id: String,
    pub seq: i64,
    pub label: String,
    pub minute: i64,
    pub skipped: i64,
    pub is_numbering_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintMatch {
    pub fingerprint: String,
    pub occurrences: Vec<FpOccurrence>,
    pub relation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FpOccurrence {
    pub source_id: String,
    pub seq: i64,
    pub label: Option<String>,
    pub field_mode: FieldMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignmentCandidate {
    pub id: String,
    pub pair: Vec<String>,
    pub offset_ns_num: i64,
    pub offset_ns_den: i64,
    pub inlier_count: i64,
    pub max_abs_residual_ns: i64,
    pub residuals: Vec<Residual>,
    pub valid_from_seq: i64,
    pub valid_to_seq: i64,
    pub status: String,
    pub reject_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Residual {
    pub fingerprint: String,
    pub at_seq: i64,
    pub observed_offset_ns: i64,
    pub residual_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Analysis {
    pub segments: Vec<TimecodeSegment>,
    pub breaks: Vec<BreakPoint>,
    pub skips: Vec<SkipPosition>,
    pub transitions: Vec<Transition>,
    pub fingerprint_matches: Vec<FingerprintMatch>,
    pub unconvertible: Vec<Unconvertible>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unconvertible {
    pub source_id: String,
    pub seq: i64,
    pub raw: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignmentResult {
    pub id: String,
    pub param_version: String,
    pub candidates: Vec<AlignmentCandidate>,
    pub accepted_id: Option<String>,
    pub note: String,
}
