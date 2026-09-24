//! Core data model: immutable materials, append-only events and the derived
//! review vocabulary (segments, breaks, skips, alignment candidates).

use serde::{Deserialize, Serialize};

pub use crate::timecode::{Label, Rate};

/// Provenance of a piece of decoded timecode.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Ltc,
    Vitc,
    Video,
    Log,
}

impl SourceKind {
    pub fn parse(text: &str) -> Option<SourceKind> {
        match text.trim().to_ascii_lowercase().as_str() {
            "ltc" => Some(SourceKind::Ltc),
            "vitc" => Some(SourceKind::Vitc),
            "video" | "frames" | "frame" => Some(SourceKind::Video),
            "log" | "rec" | "record" => Some(SourceKind::Log),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Ltc => "ltc",
            SourceKind::Vitc => "vitc",
            SourceKind::Video => "video",
            SourceKind::Log => "log",
        }
    }
}

/// Field position for interlaced VITC sampling.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Frame,
    Field1,
    Field2,
}

impl FieldKind {
    pub fn parse(text: &str) -> FieldKind {
        match text.trim().to_ascii_lowercase().as_str() {
            "f1" | "field1" | "1" => FieldKind::Field1,
            "f2" | "field2" | "2" => FieldKind::Field2,
            _ => FieldKind::Frame,
        }
    }
}

/// One decoded source observation.  Every byte of source representation that
/// matters for review is kept on the row itself; nothing is normalised away.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Sample {
    /// 1-based position of the row inside the material version.
    pub ordinal: u64,
    /// Label exactly as submitted.
    pub label_text: String,
    /// 8 hex nibbles user bits, verbatim (may contain non-BCD characters).
    pub user_bits: String,
    pub field: FieldKind,
    /// 0..100 bit/decoder quality score, verbatim string preserved too.
    pub quality: Option<u8>,
    pub quality_text: Option<String>,
    /// Content fingerprint (frame hash) exactly as submitted.
    pub fingerprint: Option<String>,
    /// Device clock exactly as submitted (free text ISO-8601-ish).
    pub device_clock: Option<String>,
    /// Raw original line, before parsing.
    pub raw_line: String,
}

/// A versioned, immutable-once-submitted block of source material.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MaterialVersion {
    pub material_id: String,
    pub version: u32,
    pub parent_version: Option<u32>,
    pub kind: SourceKind,
    /// Rate rational exactly as submitted.
    pub rate_text: String,
    /// Drop-frame flag as declared by the header.
    pub drop_frame: bool,
    /// Colour-frame flag (bit 11), preserved.
    pub color_frame: bool,
    pub title: String,
    pub samples: Vec<Sample>,
    /// Free-form log/clip payload for `log` materials (raw rows kept).
    pub note: Option<String>,
}

/// Severity levels for diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Something unconvertible stays at its origin and gets a diagnostic rather
/// than being dropped for the sake of a chart.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub material_id: String,
    pub version: u32,
    pub ordinal: Option<u64>,
    pub message: String,
    pub raw: Option<String>,
}

/// Algorithm/version stamp carried by every derived value.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DerivationInfo {
    pub algorithm_version: String,
    pub parameter_fingerprint: String,
    /// Stable ids of the direct sources (material versions or other derivs).
    pub direct_sources: Vec<String>,
}

/// Classification of one adjacent-pair transition inside a material walk.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Forward,
    /// Label numbers skipped at a DF minute boundary (no frame lost).
    DropFrameSkip,
    MidnightWrap,
    /// Ambiguous between midnight wrap and a real backwards clock step.
    WrapOrBack,
    Backwards,
    FieldPair,
    UserBitsChanged,
    /// Continuous recording resumes after missing rows (recoverable).
    GapContinued,
    /// Discontinuous jump with no consistent continuation.
    Discontinuity,
    IllegalLabel,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WalkStep {
    pub from_ordinal: u64,
    pub to_ordinal: u64,
    pub kind: StepKind,
    /// Signed frame-unit delta under the chosen forward interpretation.
    pub delta_units: Option<i64>,
    /// Alternate signed delta when two readings are valid.
    pub alternate_delta_units: Option<i64>,
    pub skipped_label_numbers: Vec<u32>,
    pub detail: String,
}

/// A maximal run of samples that are mutually consistent (a reviewable
/// timecode segment).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub segment_id: String,
    pub start_ordinal: u64,
    pub end_ordinal: u64,
    pub direction: String, // "forward" | "reverse" | "stationary"
    pub start_label: String,
    pub end_label: String,
    pub length_units: u64,
}

/// A reviewed break between two segments.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Break {
    pub break_id: String,
    pub after_ordinal: u64,
    pub kind: String,
    pub detail: String,
}

/// Everything derived for one material version.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MaterialDerivation {
    pub deriv_id: String,
    pub material_version_id: String,
    pub info: DerivationInfo,
    pub rate_valid: bool,
    pub units_per_second: Option<u64>,
    pub steps: Vec<WalkStep>,
    pub segments: Vec<Segment>,
    pub breaks: Vec<Break>,
    pub diagnostics: Vec<Diagnostic>,
}

/// One alignment correspondence pair.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Correspondence {
    pub base_ordinal: u64,
    pub other_ordinal: u64,
    pub base_fingerprint: String,
    pub other_fingerprint: String,
    pub observed_offset: i64,
}

/// A legal alignment candidate — candidates are first-class data even when
/// later rejected.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AlignmentCandidate {
    pub candidate_id: String,
    pub offset_units: i64,
    pub support: u64,
    pub residual_max: u64,
    pub residual_sum: u64,
    pub valid_start: Option<i64>,
    pub valid_end: Option<i64>,
    pub status: CandidateStatus,
    pub reject_reason: Option<String>,
    pub correspondences: Vec<Correspondence>,
    /// True when the runner could not uniquely decide (tie or periodicity).
    pub ambiguous: bool,
    /// All equally-legal offsets when ambiguity holds (including this one).
    pub legal_offsets: Vec<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Proposed,
    Accepted,
    Rejected,
    Superseded,
}

/// Full multi-source alignment derivation for a base/other material pair.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AlignmentDerivation {
    pub deriv_id: String,
    pub alignment_group_id: String,
    pub base_material_version_id: String,
    pub other_material_version_id: String,
    pub info: DerivationInfo,
    pub candidates: Vec<AlignmentCandidate>,
    pub valid_segments: Vec<ValidSegment>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ValidSegment {
    pub base_start_ordinal: u64,
    pub base_end_ordinal: u64,
    pub other_start_ordinal: u64,
    pub other_end_ordinal: u64,
    pub length_units: u64,
}

/// Tunable parameters.  Every revision is fingerprinted; derived ids embed the
/// fingerprint so a parameter change naturally supersedes old results.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Parameters {
    pub max_gap_units: u64,
    pub residual_tolerance_units: u64,
    pub ambiguity_window_units: u64,
    /// When true, wrap-vs-back ambiguity is resolved forward by policy; the
    /// alternative always remains visible as a candidate.
    pub prefer_midnight_wrap: bool,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            max_gap_units: 60,
            residual_tolerance_units: 1,
            ambiguity_window_units: 8,
            prefer_midnight_wrap: true,
        }
    }
}

/// Append-only history entry.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EventRecord {
    pub seq: u64,
    pub event_id: String,
    pub parent_event_id: Option<String>,
    pub at_ms: i64,
    pub actor: String,
    pub payload: EventPayload,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    ProjectOpened { seed: u64 },
    MaterialCommitted { version: MaterialVersion },
    /// A local source change: only this material version is new; parent points
    /// at the version it replaces.
    MaterialPatched {
        material_id: String,
        from_version: u32,
        to: MaterialVersion,
        reason: String,
    },
    ParametersRevised {
        from_fingerprint: Option<String>,
        parameters: Parameters,
        reason: String,
    },
    CandidateDecided {
        alignment_group_id: String,
        candidate_id: String,
        accept: bool,
        reason: String,
        /// Previous decision superseded by this one, if any.
        supersedes_event_id: Option<String>,
    },
    /// Conflicting manual decisions are kept verbatim and marked.
    DecisionConflict {
        alignment_group_id: String,
        kept_event_id: String,
        rejected_event_id: String,
        reason: String,
    },
    AlignmentFrozen {
        alignment_group_id: String,
        candidate_id: String,
        label: String,
        snapshot: serde_json::Value,
    },
    FreezeRevoked {
        freeze_event_id: String,
        reason: String,
    },
    /// Row in the counter-example archive, machine readable.
    CounterExampleRecorded {
        record: crate::archive::CounterExampleRecord,
    },
}

/// Frozen alignment version.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Freeze {
    pub event_id: String,
    pub group_id: String,
    pub candidate_id: String,
    pub label: String,
    pub active: bool,
    pub snapshot: serde_json::Value,
}
