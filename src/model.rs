//! Core domain model: every raw fact and every derived value is an explicit,
//! unit-labelled, provenance-bearing value. Nothing is silently dropped.

use serde::{Deserialize, Serialize};

/// Frame rate as an exact rational number, e.g. 30000/1001 for 29.97.
/// Never converted to a float anywhere in the domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rate {
    pub num: i64,
    pub den: i64,
}

impl Rate {
    pub fn new(num: i64, den: i64) -> Self {
        Rate { num, den }
    }

    /// Nominal label frames per second (round to nearest): 30 for 30000/1001.
    pub fn nominal_fps(&self) -> i64 {
        (self.num + self.den / 2) / self.den
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.num, self.den)
    }
}

/// Device clock sample. `original` is the verbatim imported representation.
/// `nanos` is the parsed value relative to `basis`; it is `None` when the
/// value cannot be converted, and the unconverted value stays in place with
/// a diagnostic instead of being removed for charting convenience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceClock {
    pub original: String,
    pub basis: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nanos: Option<i64>,
}

/// A diagnostic attached at the exact position where the problem occurred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: String, // "error" | "warning" | "info"
    pub code: String,
    pub message: String,
}

/// One submitted source batch: LTC/VITC decoder export, frame-summary feed or
/// recording log. Submitted content is immutable; corrections arrive as new
/// versions with a `parent_source_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInput {
    pub kind: String,            // "ltc" | "vitc" | "video_frames" | "recording_log"
    pub rate: Rate,
    pub drop_frame: bool,
    pub color_frame: bool,
    pub name: String,
    pub parent_source_id: Option<String>,
    pub records: Vec<RawRecord>,
}

/// One raw record. All fields are optional from the parser's point of view;
/// which are required depends on the source kind and is validated on import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRecord {
    /// 1-based ordinal within the source, used for stable ordering.
    pub ord: i64,
    /// Verbatim raw bits ("0"/"1"), kept exactly as submitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bits: Option<String>,
    /// Declared timecode label in the original representation, e.g. "00:00:59;28".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// User bits exactly as submitted, e.g. "12 AB 34 CD".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_bits: Option<String>,
    /// Field marker from the original representation: "1" | "2" | "."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_mark: Option<String>,
    /// Frame fingerprint verbatim (hash/digest of picture content).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Recording direction in the original representation: "fwd" | "rev".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    pub device_clock: Option<DeviceClock>,
    /// Free-form original key/value pairs that had no typed slot; retained.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<(String, String)>,
}

/// Coordinate basis for a scalar position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coord {
    pub value: i128,
    pub unit: String,
    pub basis: String,
}

/// Quality assessment of a raw bit word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitQuality {
    pub score: i64,
    pub sync_ok: bool,
    pub bcd_ok: bool,
    pub length: i64,
    pub issues: Vec<String>,
}

/// Derived view of a single record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordD {
    pub record_id: String,
    pub source_id: String,
    pub source_version: i64,
    pub ord: i64,
    pub label: Option<String>,
    pub label_index: Option<Coord>,
    pub decoded_label: Option<String>,
    pub user_bits: Option<String>,
    pub field_mark: Option<String>,
    pub field_index: Option<Coord>,
    pub frame_index: Option<Coord>,
    pub fingerprint: Option<String>,
    pub direction: String,
    pub device_clock: Option<DeviceClock>,
    pub bit_quality: Option<BitQuality>,
    pub diagnostics: Vec<Diagnostic>,
}

/// One possible explanation of a transition between adjacent records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JumpSolution {
    pub kind: String, // "continuous" | "midnight_wrap" | "true_rollback" | "reverse" | "gap"
    pub delta: Coord,
    pub evidence: Vec<String>,
    pub score: i64,
}

/// Derived transition between two adjacent records of a source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionD {
    pub node_id: String,
    pub source_id: String,
    pub from_ord: i64,
    pub to_ord: i64,
    pub kind: String, // adopted classification
    pub ambiguous: bool,
    pub solutions: Vec<JumpSolution>,
    pub missing_numbers: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// A maximal run of consistent records inside one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentD {
    pub node_id: String,
    pub source_id: String,
    pub start_ord: i64,
    pub end_ord: i64,
    pub direction: String,
    pub break_reason: Option<String>,
}

/// A fingerprint correspondence between two sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintLink {
    pub record_id_a: String,
    pub record_id_b: String,
    pub source_a: String,
    pub source_b: String,
    pub ord_a: i64,
    pub ord_b: i64,
    pub fingerprint: String,
    pub clock_offset: Option<Coord>,
    pub label_offset: Option<Coord>,
}

/// One legal multi-source alignment offset with residual evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignCandidate {
    pub candidate_id: String,
    pub source_a: String,
    pub source_b: String,
    pub offset: Coord,
    pub residual: Coord,
    pub max_residual: Coord,
    pub support: i64,
    pub valid_segment: (i64, i64, i64, i64),
    pub rejected_reason: Option<String>,
    pub tied: bool,
}
