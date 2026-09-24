//! Replay + derivation. The log is replayed to materialize:
//! current source revisions, parameters, decisions and frozen snapshots.
//! Derived observations/segments/breaks are recomputed per source revision and
//! carry an `AlgoRef`. A source revision only invalidates dependents of THAT
//! source; stale values remain viewable but are flagged, never overwritten as
//! if they were current.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::events::{
    AlgoRef, Event, EventKind, ALGO_SEG, ALGO_SEG_VERSION, ALGO_TC, ALGO_TC_VERSION,
};
use crate::domain::raw::{RawBlock, RawRecord, SourceKind};
use crate::domain::timecode::{self, FrameRate, TcLabel};

/// A normalized view of one raw record. Fields that could not be converted are
/// `None` and the original string is always retained in `RawRecord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub source_event_id: String,
    pub source_id: String,
    pub ordinal: usize,
    pub label: Option<TcLabel>,
    pub tc_unparseable: bool,
    /// Continuous frame index since source 00:00 (None if unparseable/skipped).
    pub frame_index: Option<i64>,
    pub quality: Option<f64>,
    pub user_bits: Option<String>,
    pub color_frame: Option<bool>,
    pub field: Option<u32>,
    pub fingerprint: String,
    pub device_clock: String,
    pub direction: String,
    pub bits_raw: String,
    pub diagnostics: Vec<String>,
    /// Unit/coordinate basis for every quantitative value on this row.
    pub basis: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakKind {
    /// Expected drop-frame minute skip (not an error, no frames lost).
    DropNumbering,
    /// Midnight wrap candidate vs true backwards: both kept until decided.
    WrapOrReverse,
    /// Direction marker/value indicates reverse recording.
    ReverseRecording,
    /// User bits changed mid-run.
    UserBitsChange,
    /// Field flag/mode changed.
    FieldModeChange,
    /// Color framing toggled.
    ColorFrameChange,
    /// Signal loss / discontinuity then resync continuation.
    Resync,
    /// A genuinely unparseable or skipped-label value.
    IllegalOrUnparseable,
}

impl BreakKind {
    pub fn code(&self) -> &'static str {
        match self {
            BreakKind::DropNumbering => "DROP_NUMBERING",
            BreakKind::WrapOrReverse => "WRAP_OR_REVERSE",
            BreakKind::ReverseRecording => "REVERSE_RECORDING",
            BreakKind::UserBitsChange => "USER_BITS_CHANGE",
            BreakKind::FieldModeChange => "FIELD_MODE_CHANGE",
            BreakKind::ColorFrameChange => "COLOR_FRAME_CHANGE",
            BreakKind::Resync => "RESYNC",
            BreakKind::IllegalOrUnparseable => "ILLEGAL_OR_UNPARSEABLE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Break {
    pub id: String,
    pub source_event_id: String,
    pub source_id: String,
    pub ordinal: usize,
    pub kind: String,
    pub from_label: Option<String>,
    pub to_label: Option<String>,
    pub forward_frames: Option<i64>,
    pub backward_frames: Option<i64>,
    pub direct_frames: Option<i64>,
    pub state: String, // OPEN | ACCEPTED | REJECTED
    pub decision_event_id: Option<String>,
    pub chosen: Option<String>,
    pub rejected_reasons: Vec<String>,
    pub detail: String,
    pub algo: AlgoRef,
}

/// A contiguous, reviewable timecode segment bounded by breaks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: String,
    pub source_event_id: String,
    pub source_id: String,
    pub start_ordinal: usize,
    pub end_ordinal: usize,
    pub start_label: Option<String>,
    pub end_label: Option<String>,
    pub frame_count: Option<i64>,
    pub direction: String,
    pub min_quality: Option<f64>,
    pub basis: String,
}

/// Parameters that revisions may change (parameter fingerprinted into AlgoRef).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// Residual tolerance in real frames for an alignment to be valid.
    pub align_tolerance_frames: i64,
    /// Min shared fingerprints required to emit candidates.
    pub min_shared_fingerprints: usize,
    /// Default policy when wrap vs reverse is undecided: KEEP_BOTH.
    pub ambiguity_policy: String,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            align_tolerance_frames: 1,
            min_shared_fingerprints: 2,
            ambiguity_policy: "KEEP_BOTH".into(),
        }
    }
}

impl Params {
    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let canon = serde_json::to_string(self).unwrap();
        let mut h = Sha256::new();
        h.update(canon.as_bytes());
        format!("pf_{}", hex::encode(&h.finalize()[..6]))
    }
}

/// One current revision of a source plus its derived material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRevision {
    pub source_id: String,
    /// Event that introduced this revision (ImportCommitted or SourceRevised).
    pub event_id: String,
    pub seq: i64,
    pub block: RawBlock,
    pub observations: Vec<Observation>,
    pub segments: Vec<Segment>,
    pub breaks: Vec<Break>,
    pub tc_algo: AlgoRef,
    pub seg_algo: AlgoRef,
}

/// A frozen alignment snapshot id + the event that froze it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenSnapshot {
    pub snapshot_id: String,
    pub event_id: String,
    pub seq: i64,
    pub note: String,
    pub reopened: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectState {
    pub events: Vec<Event>,
    /// source_id -> current revision
    pub revisions: BTreeMap<String, SourceRevision>,
    /// Event ids in order of introduction.
    pub source_order: Vec<String>,
    pub params: Params,
    pub params_event_id: String,
    pub decisions: BTreeMap<String, Event>,
    pub frozen: Vec<FrozenSnapshot>,
}

impl ProjectState {
    pub fn new() -> Self {
        ProjectState {
            events: Vec::new(),
            revisions: BTreeMap::new(),
            source_order: Vec::new(),
            params: Params::default(),
            params_event_id: String::new(),
            decisions: BTreeMap::new(),
            frozen: Vec::new(),
        }
    }

    pub fn head_event_id(&self) -> String {
        self.events.last().map(|e| e.id.clone()).unwrap_or_default()
    }
}

fn oid(source_event_id: &str, ordinal: usize) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"obs|");
    h.update(source_event_id.as_bytes());
    h.update(b"|");
    h.update(ordinal.to_string().as_bytes());
    format!("obs_{}", hex::encode(&h.finalize()[..8]))
}

fn bid(source_event_id: &str, ordinal: usize, kind: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"brk|");
    h.update(source_event_id.as_bytes());
    h.update(b"|");
    h.update(ordinal.to_string().as_bytes());
    h.update(b"|");
    h.update(kind.as_bytes());
    format!("brk_{}", hex::encode(&h.finalize()[..8]))
}

fn parse_quality(raw: &str) -> Option<f64> {
    raw.parse::<f64>().ok().filter(|q| (0.0..=1.0).contains(q))
}
fn parse_bool01(raw: &str) -> Option<bool> {
    match raw.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// Derive observations + segments + breaks for one source revision.
pub fn derive_revision(
    source_id: &str,
    event_id: &str,
    seq: i64,
    block: RawBlock,
    params: &Params,
    decisions: &BTreeMap<String, Event>,
) -> SourceRevision {
    let rate = block.rate();
    let pf = params.fingerprint();
    let tc_algo = AlgoRef {
        algo: ALGO_TC.into(),
        version: ALGO_TC_VERSION.into(),
        params_fingerprint: pf.clone(),
        direct_sources: vec![event_id.to_string()],
    };
    let seg_algo = AlgoRef {
        algo: ALGO_SEG.into(),
        version: ALGO_SEG_VERSION.into(),
        params_fingerprint: pf,
        direct_sources: vec![event_id.to_string()],
    };

    let mut observations = Vec::new();
    for (i, rec) in block.records.iter().enumerate() {
        observations.push(derive_observation(source_id, event_id, i, rec, &block, rate));
    }

    let mut breaks = Vec::new();
    for i in 1..observations.len() {
        detect_breaks(event_id, source_id, i, &observations[i - 1], &observations[i], &block, rate, &mut breaks);
    }
    // mark decisions
    for b in breaks.iter_mut() {
        if let Some(ev) = decisions.get(&b.id) {
            if let EventKind::Decision { choice, rejected, .. } = &ev.kind {
                b.state = "ACCEPTED".into();
                b.chosen = Some(choice.clone());
                b.decision_event_id = Some(ev.id.clone());
                b.rejected_reasons = rejected.clone();
            }
        }
    }

    let segments = build_segments(event_id, source_id, &observations, &breaks);

    SourceRevision {
        source_id: source_id.to_string(),
        event_id: event_id.to_string(),
        seq,
        block,
        observations,
        segments,
        breaks,
        tc_algo,
        seg_algo,
    }
}

#[allow(clippy::too_many_arguments)]
fn derive_observation(
    source_id: &str,
    event_id: &str,
    ordinal: usize,
    rec: &RawRecord,
    block: &RawBlock,
    rate: FrameRate,
) -> Observation {
    let mut diagnostics = Vec::new();
    let mut label = None;
    let mut frame_index = None;
    let mut tc_unparseable = false;

    if !rec.tc_raw.is_empty() {
        match TcLabel::parse(&rec.tc_raw) {
            Ok((mut lab, diag)) => {
                if let Some(d) = diag {
                    diagnostics.push(d);
                }
                // block DF flag governs numbering; keep label flag but compute
                // with the block's drop-frame setting.
                lab.drop_frame = block.drop_frame;
                match timecode::label_to_frames(lab, rate) {
                    Ok(fi) => frame_index = Some(fi as i64),
                    Err(e) => {
                        // skipped DF label or out-of-range: keep raw + diagnostic.
                        diagnostics.push(e);
                    }
                }
                label = Some(lab);
            }
            Err(e) => {
                tc_unparseable = true;
                diagnostics.push(e);
            }
        }
    }

    let quality = parse_quality(&rec.quality_raw);
    if quality.is_none() && !rec.quality_raw.is_empty() {
        diagnostics.push(format!("quality kept raw: {}", rec.quality_raw));
    }
    let color_frame = parse_bool01(&rec.color_frame_raw);
    if color_frame.is_none() && !rec.color_frame_raw.is_empty() {
        diagnostics.push(format!("cf kept raw: {}", rec.color_frame_raw));
    }
    let mut field = rec.field_raw.parse::<u32>().ok();
    if field.is_none() && !block.field_mode_raw.is_empty() {
        field = block.field_mode_raw.parse().ok();
    }
    let user_bits = if rec.user_bits_raw.is_empty() { None } else { Some(rec.user_bits_raw.clone()) };

    let basis = format!(
        "tc_frames@{}Hz ({}) ; quality=unitless[0..1] ; dev_clock=ISO8601 text as-submitted",
        // rational rate rendered exactly
        format!("{}/{}", block.rate_num, block.rate_den),
        if block.drop_frame { "drop-frame numbering" } else { "non-drop numbering" }
    );

    Observation {
        id: oid(event_id, ordinal),
        source_event_id: event_id.to_string(),
        source_id: source_id.to_string(),
        ordinal,
        label,
        tc_unparseable,
        frame_index,
        quality,
        user_bits,
        color_frame,
        field,
        fingerprint: rec.fingerprint.clone(),
        device_clock: rec.device_clock_raw.clone(),
        direction: rec.direction_raw.clone(),
        bits_raw: rec.bits_raw.clone(),
        diagnostics,
        basis,
    }
}

fn detect_breaks(
    event_id: &str,
    source_id: &str,
    i: usize,
    prev: &Observation,
    cur: &Observation,
    block: &RawBlock,
    rate: FrameRate,
    out: &mut Vec<Break>,
) {
    let render = |o: &Observation| o.label.map(|l| l.render());
    let push = |kind: BreakKind, detail: String, out: &mut Vec<Break>| {
        out.push(Break {
            id: bid(event_id, i, kind.code()),
            source_event_id: event_id.to_string(),
            source_id: source_id.to_string(),
            ordinal: i,
            kind: kind.code().into(),
            from_label: render(prev),
            to_label: render(cur),
            forward_frames: None,
            backward_frames: None,
            direct_frames: None,
            state: "OPEN".into(),
            decision_event_id: None,
            chosen: None,
            rejected_reasons: vec![],
            detail,
            algo: AlgoRef {
                algo: ALGO_SEG.into(),
                version: ALGO_SEG_VERSION.into(),
                params_fingerprint: String::new(),
                direct_sources: vec![prev.id.clone(), cur.id.clone()],
            },
        });
    };

    if let (Some(a), Some(b)) = (prev.label, cur.label) {
        if let Ok(w) = timecode::delta_with_wrap(a, b, rate) {
            // Expected DF skip at a minute boundary: forward == 1 and labels
            // jump exactly by the skip count. Documents "skipping != dropping".
            let expected_skip = block.drop_frame
                && rate.df_allowed()
                && a.s == (rate.nominal_fps() - 1)
                && b.s == 0
                && b.f == rate.df_skip_count()
                && w.forward == 1;
            if expected_skip {
                let mut br = Break::placeholder();
                br.id = bid(event_id, i, BreakKind::DropNumbering.code());
                br.source_event_id = event_id.into();
                br.source_id = source_id.into();
                br.ordinal = i;
                br.kind = BreakKind::DropNumbering.code().into();
                br.from_label = render(prev);
                br.to_label = render(cur);
                br.forward_frames = Some(1);
                br.backward_frames = Some(1 - w.day_frames);
                br.direct_frames = Some(w.direct);
                br.detail = format!(
                    "drop-frame minute boundary: labels skip {} but exactly 1 real frame elapsed (no frame lost)",
                    rate.df_skip_count()
                );
                br.state = "ACCEPTED".into();
                br.chosen = Some("DROP_NUMBERING_EXPECTED".into());
                out.push(br);
            } else if w.direct <= 0 && !(w.direct == 0) {
                // Backward movement: could be true reverse or midnight wrap.
                let mut br = Break::placeholder();
                br.id = bid(event_id, i, BreakKind::WrapOrReverse.code());
                br.source_event_id = event_id.into();
                br.source_id = source_id.into();
                br.ordinal = i;
                br.kind = BreakKind::WrapOrReverse.code().into();
                br.from_label = render(prev);
                br.to_label = render(cur);
                br.forward_frames = Some(w.forward);
                br.backward_frames = Some(w.backward);
                br.direct_frames = Some(w.direct);
                br.detail = "label moved backwards: both MIDNIGHT_WRAP and TRUE_REVERSE are legal until adjudicated".into();
                out.push(br);
            } else if w.forward != 1 && w.direct > 0 {
                // Positive non-unit jump = signal gap then resync continuation.
                let mut br = Break::placeholder();
                br.id = bid(event_id, i, BreakKind::Resync.code());
                br.source_event_id = event_id.into();
                br.source_id = source_id.into();
                br.ordinal = i;
                br.kind = BreakKind::Resync.code().into();
                br.from_label = render(prev);
                br.to_label = render(cur);
                br.forward_frames = Some(w.forward);
                br.direct_frames = Some(w.direct);
                br.detail = format!("gap of {} real frames; treated as break-and-continue (resync)", w.forward - 1);
                out.push(br);
            }
        }
    } else if cur.tc_unparseable || prev.tc_unparseable {
        push(
            BreakKind::IllegalOrUnparseable,
            "unparseable or illegal timecode value retained raw".into(),
            out,
        );
    }

    if cur.direction_raw.eq_ignore_ascii_case("REV") && !prev.direction_raw.eq_ignore_ascii_case("REV") {
        push(BreakKind::ReverseRecording, "direction marker switched to REV".into(), out);
    }
    if cur.user_bits.is_some() && prev.user_bits.is_some() && cur.user_bits != prev.user_bits {
        push(
            BreakKind::UserBitsChange,
            format!("user bits changed {} -> {}", prev.user_bits_dbg(), cur.user_bits_dbg()),
            out,
        );
    }
    if cur.field.is_some() && prev.field.is_some() && cur.field != prev.field {
        push(BreakKind::FieldModeChange, "field flag changed between records".into(), out);
    }
    if cur.color_frame.is_some() && prev.color_frame.is_some() && cur.color_frame != prev.color_frame {
        push(BreakKind::ColorFrameChange, "color-frame flag toggled".into(), out);
    }
}
