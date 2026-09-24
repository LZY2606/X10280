//! Append-only event log. Raw inputs are immutable once submitted; every
//! revision is a new event referencing its parent. Derived values never edit
//! history. IDs are content-addressed (`ev_` + sha256 prefix) so an
//! export/delete/import round trip reproduces identical identities and order.

use serde::{Deserialize, Serialize};

use crate::domain::raw::ParsedImport;

pub const ALGO_TC: &str = "tc-arithmetic";
pub const ALGO_TC_VERSION: &str = "1.0.0";
pub const ALGO_SEG: &str = "segment-break-detect";
pub const ALGO_SEG_VERSION: &str = "1.0.0";
pub const ALGO_ALIGN: &str = "multi-source-align";
pub const ALGO_ALIGN_VERSION: &str = "1.0.0";

/// Algorithm + parameter fingerprint carried by every derived value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlgoRef {
    pub algo: String,
    pub version: String,
    pub params_fingerprint: String,
    /// Event ids that are the DIRECT sources of this derived value.
    pub direct_sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// Initial immutable raw submission.
    ImportCommitted { import: ParsedImport, label: String },
    /// A new revision of one source block; parent points at the previous
    /// source revision event. Only dependents of that source go stale.
    SourceRevised {
        source_id: String,
        parent_source_event: String,
        block_json: String,
        reason: String,
    },
    /// Parameter revision (e.g. alignment tolerance, wrap policy default).
    ParamsRevised { params_json: String, parent_event: String, note: String },
    /// Human adjudication of an ambiguous breakpoint or alignment candidate.
    Decision {
        target_id: String,
        /// kind of object being decided: BREAK | CANDIDATE_SET
        target_kind: String,
        choice: String,
        rejected: Vec<String>,
        rationale: String,
    },
    /// Freeze of an alignment version (an immutable review snapshot).
    AlignmentFrozen { snapshot_id: String, note: String },
    /// Reopen for review (does not delete the frozen snapshot).
    AlignmentReopened { snapshot_id: String },
    /// Provenance marker: project created / case-pack step.
    Marker { note: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Stable content-addressed id, e.g. `ev_3f9a..`.
    pub id: String,
    /// Monotonic logical sequence number within the project.
    pub seq: i64,
    /// Virtual/recorded timestamp (ISO 8601). Tests use a fixed virtual clock.
    pub at: String,
    /// Id of the previous event (parent in the log), empty for the first.
    pub parent: String,
    pub kind: EventKind,
}

/// Deterministic content hash for stable identities.
pub fn content_id(seq: i64, parent: &str, at: &str, canonical: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"tcwb-event|");
    h.update(seq.to_string().as_bytes());
    h.update(b"|");
    h.update(parent.as_bytes());
    h.update(b"|");
    h.update(at.as_bytes());
    h.update(b"|");
    h.update(canonical.as_bytes());
    let digest = h.finalize();
    format!("ev_{}", hex::encode(&digest[..8]))
}

/// Compute the id for an event from its canonical JSON payload.
pub fn event_id(seq: i64, parent: &str, at: &str, kind: &EventKind) -> String {
    let canonical = serde_json::to_string(kind).expect("event kind serializes");
    content_id(seq, parent, at, &canonical)
}
