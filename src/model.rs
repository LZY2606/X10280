//! Data model. Raw inputs are immutable once submitted; revisions are
//! new versions linked to a parent. Every derived value carries the
//! algorithm version, a parameter fingerprint and its direct sources.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::tc::{FieldMode, FrameRate};

/// One decoded frame of raw input. Everything the decoder saw is kept:
/// raw bits, rational rate, drop/color flags, user bits, field flag,
/// frame fingerprint and device clock. Nothing is normalised here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawFrame {
    /// 80-bit LTC/VITC word, hex encoded, exactly as decoded.
    pub raw_bits: String,
    pub fps_num: u32,
    pub fps_den: u32,
    pub drop_frame: bool,
    pub color_frame: bool,
    /// 32 user bits, hex encoded (8 chars).
    pub user_bits: String,
    /// Field flag as decoded (0/1); meaning depends on field mode.
    pub field_flag: u8,
    /// Content fingerprint of the associated video frame (phash hex).
    pub frame_fingerprint: String,
    /// Device clock reading in nanoseconds (device timebase).
    pub device_clock_ns: i64,
    /// Decoded timecode label as printed, e.g. "01:00:00;00".
    pub tc_label: String,
    /// Decoder bit-quality 0..=100 (parity/sync confidence).
    pub bit_quality: u8,
}

impl RawFrame {
    pub fn rate(&self) -> FrameRate {
        FrameRate::new(self.fps_num, self.fps_den)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    LtcDecode,
    VitcDecode,
    VideoFrameSummary,
    RecordingLog,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::LtcDecode => "ltc_decode",
            SourceKind::VitcDecode => "vitc_decode",
            SourceKind::VideoFrameSummary => "video_frame_summary",
            SourceKind::RecordingLog => "recording_log",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ltc_decode" => SourceKind::LtcDecode,
            "vitc_decode" => SourceKind::VitcDecode,
            "video_frame_summary" => SourceKind::VideoFrameSummary,
            "recording_log" => SourceKind::RecordingLog,
            _ => return None,
        })
    }
}

/// An immutable version of a raw source submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceVersion {
    /// Stable identity: content-addressed, survives export/import.
    pub id: String,
    /// Identity of the logical source (shared by all its versions).
    pub source_id: String,
    pub version: u32,
    pub parent_version_id: Option<String>,
    pub kind: SourceKind,
    pub name: String,
    pub frames: Vec<RawFrame>,
    /// Logical (virtual) clock at submission.
    pub submitted_at_tick: u64,
    /// Values that could not be converted stay in place with a
    /// diagnostic; they are never dropped for the sake of plotting.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub frame_index: usize,
    pub field: String,
    pub raw_value: String,
    pub message: String,
}

/// Deterministic content hash used for stable identities.
pub fn content_hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    let out = h.finalize();
    out.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

pub fn source_version_id(source_id: &str, version: u32, frames: &[RawFrame]) -> String {
    let frame_blob = frames
        .iter()
        .map(|f| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                f.raw_bits, f.fps_num, f.fps_den, f.drop_frame, f.color_frame,
                f.user_bits, f.field_flag, f.frame_fingerprint, f.device_clock_ns,
                f.tc_label, f.bit_quality
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("sv_{}", content_hash(&[source_id, &version.to_string(), &frame_blob]))
}

/// Append-only event. Every judgement, adjudication, conflict, undo,
/// freeze, export and import is a machine-readable event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub id: String,
    pub tick: u64,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// A derived result with full provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Derivation {
    pub id: String,
    /// e.g. "segment-builder", "alignment-solver"
    pub algorithm: String,
    pub algorithm_version: String,
    /// Fingerprint of the parameter set used.
    pub params_fingerprint: String,
    /// Direct source version ids this result depends on.
    pub input_version_ids: Vec<String>,
    pub output: serde_json::Value,
    /// current | stale | superseded. Stale results stay viewable but
    /// must not masquerade as current.
    pub status: String,
    pub created_tick: u64,
}

/// Alignment candidate status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Pending,
    Accepted,
    Rejected,
    /// Legal candidate retained because no unique decision exists.
    RetainedAlternative,
}

/// A frozen alignment version: a reviewable snapshot whose every
/// displayed value carries a stable identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Freeze {
    pub id: String,
    pub name: String,
    pub created_tick: u64,
    /// Stable-id -> value snapshot of the review page.
    pub snapshot: serde_json::Value,
    pub input_version_ids: Vec<String>,
}

pub fn field_mode_label(m: &FieldMode) -> &'static str {
    match m {
        FieldMode::Progressive => "progressive",
        FieldMode::Field1First => "field1_first",
        FieldMode::Field2First => "field2_first",
        FieldMode::Unknown => "unknown",
    }
}
