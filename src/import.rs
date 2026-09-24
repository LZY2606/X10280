//! Copy-paste sample importer with explicit, line/column-accurate format errors.
//!
//! Accepted text is a line-oriented, human-copyable format:
//!
//! ```text
//! # comments and blank lines are ignored
//! META key=value
//! @camA LTC rate=30000/1001 df=1 color=1
//! F seq=1 tc=00:00:59;28 fp=f111 bits=<80 chars> clock=...
//! V ... (video frame summary)
//! L ... (recording log entry)
//! ```
//!
//! Structural problems return [`crate::error::DomainError::Format`] with the
//! line and column a reviewer can click back to. Values that parse structurally
//! but cannot be normalized (bad BCD, unparsable clock) are preserved in place
//! with diagnostics instead of aborting the import.

use crate::clock::Clock;
use crate::error::{DomainError, DomainResult};
use crate::hashing::Hasher;
use crate::model::*;
use crate::rat::Rat;
use crate::timecode::{decode_code, CodeKind, Label};

#[derive(Debug, Clone)]
pub struct ParsedSample {
    pub project_title: Option<String>,
    pub sources: Vec<Source>,
    pub observations: Vec<Observation>,
    pub soft_diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct SourceSpec {
    id: String,
    name: String,
    kind: SourceKind,
    rate_raw: String,
    df: bool,
    color_frame: bool,
}

fn err(line: usize, col: usize, msg: impl Into<String>) -> DomainError {
    DomainError::Format { line, col, message: msg.into() }
}

fn tokenize(line_no: usize, line: &str) -> DomainResult<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_tok = false;
    for (i, c) in line.char_indices() {
        if c.is_whitespace() {
            if in_tok {
                out.push(std::mem::take(&mut cur));
                in_tok = false;
            }
        } else {
            cur.push(c);
            in_tok = true;
        }
        let _ = i;
    }
    if in_tok {
        out.push(cur);
    }
    if out.iter().any(|t| t.contains('=')) == false && out.is_empty() {
        return Err(err(line_no, 1, "empty record"));
    }
    Ok(out)
}

/// Find column (1-based, byte offset) of a token within the original line.
fn token_col(line: &str, want: &str) -> usize {
    let mut idx = 0usize;
    for chunk in line.split_whitespace() {
        if let Some(p) = line[idx..].find(chunk) {
            let start = idx + p;
            if chunk == want {
                return start + 1;
            }
            idx = start + chunk.len();
        } else {
            break;
        }
    }
    1
}

fn kv<'a>(tok: &'a str, line_no: usize, line: &str) -> DomainResult<(&'a str, &'a str)> {
    let pos = tok.find('=').map(|p| p + 1).unwrap_or(1);
    let (k, v) = tok.split_once('=').ok_or_else(|| {
        err(line_no, token_col(line, tok), format!("expected key=value, got '{tok}'"))
    })?;
    if k.is_empty() {
        return Err(err(line_no, pos, "empty key before '='"));
    }
    if v.is_empty() {
        return Err(err(line_no, pos + 1, format!("empty value for key '{k}'")));
    }
    Ok((k, v))
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn validate_bits_payload(kind: CodeKind, bits: &str) -> Result<(), String> {
    decode_code(kind, bits).map(|_| ())
}

fn parse_field_mode(v: &str) -> Option<FieldMode> {
    Some(match v {
        "p" | "prog" | "progressive" => FieldMode::Progressive,
        "f1" | "field1" | "top" => FieldMode::FieldOne,
        "f2" | "field2" | "bottom" => FieldMode::FieldTwo,
        _ => return None,
    })
}

/// Parse an ISO-ish device clock into nanoseconds since Unix epoch.
/// Preserves the original string regardless; parse failures are diagnostics.
pub fn parse_clock_ns(raw: &str) -> Result<i64, String> {
    // accept integer ns or ns-suffixed
    if let Ok(n) = raw.trim_end_matches("ns").parse::<i64>() {
        return Ok(n);
    }
    // accept seconds with fractional part since epoch
    let (a, b) = raw.split_once('.').ok_or_else(|| "clock must be integer ns or sec.frac".to_string())?;
    let secs: i64 = a.parse().map_err(|_| "clock seconds not numeric".to_string())?;
    if b.len() > 9 {
        return Err("clock fraction has more than 9 digits".into());
    }
    let mut frac = String::from(b);
    while frac.len() < 9 {
        frac.push('0');
    }
    let f: i64 = frac.parse().map_err(|_| "clock fraction not numeric".to_string())?;
    Ok(secs * 1_000_000_000 + f)
}

pub fn parse_sample(text: &str) -> DomainResult<ParsedSample> {
    let mut sources: Vec<SourceSpec> = Vec::new();
    let mut obs: Vec<Observation> = Vec::new();
    let mut title = None;
    let mut soft = Vec::new();
    let mut seq_counter = 0i64;

    for (idx0, raw_line) in text.lines().enumerate() {
        let line_no = idx0 + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let toks = tokenize(line_no, line)?;
        let head = toks[0].as_str();

        match head {
            "META" => {
                for t in &toks[1..] {
                    let (k, v) = kv(t, line_no, line)?;
                    if k == "title" {
                        title = Some(v.to_string());
                    }
                }
            }
            "@" => {
                return Err(err(line_no, 1, "source header must be '@<id>', '@' alone is invalid"));
            }
            _ if head.starts_with('@') => {
                let id = head[1..].to_string();
                if sources.iter().any(|s| s.id == id) {
                    return Err(err(line_no, 1, format!("duplicate source '@{id}'")));
                }
                let mut spec = SourceSpec {
                    id,
                    name: String::new(),
                    kind: SourceKind::Ltc,
                    rate_raw: String::new(),
                    df: false,
                    color_frame: false,
                };
                let mut kind_set = false;
                for t in &toks[1..] {
                    let (k, v) = kv(t, line_no, line)?;
                    match k {
                        "kind" => {
                            spec.kind = match v.to_ascii_uppercase().as_str() {
                                "LTC" => SourceKind::Ltc,
                                "VITC" => SourceKind::Vitc,
                                "VIDEO" => SourceKind::Video,
                                "LOG" => SourceKind::Log,
                                other => {
                                    return Err(err(
                                        line_no,
                                        token_col(line, t),
                                        format!("unknown source kind '{other}' (LTC|VITC|VIDEO|LOG)"),
                                    ))
                                }
                            };
                            kind_set = true;
                        }
                        "rate" => spec.rate_raw = v.to_string(),
                        "df" => {
                            spec.df = parse_bool(v).ok_or_else(|| {
                                err(line_no, token_col(line, t), "df must be 1/0")
                            })?
                        }
                        "color" => {
                            spec.color_frame = parse_bool(v).ok_or_else(|| {
                                err(line_no, token_col(line, t), "color must be 1/0")
                            })?
                        }
                        "name" => spec.name = v.to_string(),
                        other => {
                            return Err(err(
                                line_no,
                                token_col(line, t),
                                format!("unknown source field '{other}'"),
                            ))
                        }
                    }
                }
                if !kind_set {
                    return Err(err(line_no, 1, "source header requires kind=LTC|VITC|VIDEO|LOG"));
                }
                if spec.rate_raw.is_empty() {
                    return Err(err(line_no, 1, "source header requires rate=num/den"));
                }
                if Rat::parse(&spec.rate_raw).is_none() {
                    return Err(err(
                        line_no,
                        1,
                        format!("rate '{}' is not an exact rational num/den", spec.rate_raw),
                    ));
                }
                sources.push(spec);
            }
            rec @ ("F" | "V" | "L") => {
                seq_counter += 1;
                let built = build_observation(rec, &toks[1..], line_no, line, &sources, &mut soft)?;
                let mut o = built;
                o.seq = seq_counter;
                obs.push(o);
            }
            other => {
                return Err(err(
                    line_no,
                    token_col(line, other),
                    format!("unknown record '{other}' (expected META, @source, F, V or L)"),
                ));
            }
        }
    }

    if sources.is_empty() {
        return Err(DomainError::Semantic("sample declares no @source records".into()));
    }

    let created = Hasher::id(&["srchead", title.as_deref().unwrap_or(""), &obs.len().to_string()]);
    let sources = sources
        .into_iter()
        .map(|s| Source {
            id: s.id.clone(),
            name: if s.name.is_empty() { s.id.clone() } else { s.name },
            kind: s.kind,
            rate: Rat::parse(&s.rate_raw).unwrap(),
            rate_raw: s.rate_raw,
            df: s.df,
            color_frame: s.color_frame,
            created_event: created.clone(),
        })
        .collect();

    Ok(ParsedSample { project_title: title, sources, observations: obs, soft_diagnostics: soft })
}

#[allow(clippy::too_many_arguments)]
fn build_observation(
    rec: &str,
    toks: &[String],
    line_no: usize,
    line: &str,
    sources: &[SourceSpec],
    soft: &mut Vec<String>,
) -> DomainResult<Observation> {
    let mut source_id: Option<String> = None;
    let mut raw_bits: Option<String> = None;
    let mut raw_tc: Option<String> = None;
    let mut raw_clock: Option<String> = None;
    let mut fingerprint: Option<String> = None;
    let mut field_mode = FieldMode::Progressive;
    let mut ub_override: Option<String> = None;

    for t in toks {
        let (k, v) = kv(t, line_no, line)?;
        match k {
            "src" => {
                if !sources.iter().any(|s| s.id == v) {
                    return Err(err(
                        line_no,
                        token_col(line, t),
                        format!("observation refers to undeclared source '{v}'"),
                    ));
                }
                source_id = Some(v.to_string());
            }
            "bits" => raw_bits = Some(v.to_string()),
            "tc" => raw_tc = Some(v.to_string()),
            "clock" => raw_clock = Some(v.to_string()),
            "fp" => fingerprint = Some(v.to_string()),
            "field" => {
                field_mode = parse_field_mode(v).ok_or_else(|| {
                    err(line_no, token_col(line, t), "field must be p|f1|f2")
                })?;
            }
            "ub" => ub_override = Some(v.to_string()),
            other => {
                return Err(err(
                    line_no,
                    token_col(line, t),
                    format!("unknown observation field '{other}'"),
                ))
            }
        }
    }

    let source_id = source_id
        .ok_or_else(|| err(line_no, 1, format!("{rec} observation requires src=@id")))?;
    let spec = sources.iter().find(|s| s.id == source_id).unwrap().clone();

    let mut diagnostics = Vec::new();

    // ---- preserve bits, decode softly; bad bits stay in place --------------
    if let Some(bits) = &raw_bits {
        let code_kind = match spec.kind {
            SourceKind::Ltc => CodeKind::Ltc,
            SourceKind::Vitc => CodeKind::Vitc,
            _ => CodeKind::Ltc,
        };
        let want = match code_kind {
            CodeKind::Ltc => 80,
            CodeKind::Vitc => 90,
        };
        if let Err(e) = validate_bits_payload(code_kind, bits) {
            diagnostics.push(format!("bits: {e}"));
        } else if bits.len() != want {
            diagnostics.push(format!("bits: expected {want} bits"));
        } else {
            let dec = decode_code(code_kind, bits).unwrap();
            if !dec.sync_ok {
                diagnostics.push("bits: sync word mismatch (corrupted code)".to_string());
            }
            if let Some(tc) = &raw_tc {
                if let Ok((lab, _)) = Label::parse(tc) {
                    if (lab.h, lab.m, lab.s, lab.f)
                        != (dec.label.h, dec.label.m, dec.label.s, dec.label.f)
                    {
                        diagnostics.push(format!(
                            "tc '{tc}' does not match decoded label {}; raw bits retained",
                            dec.label.format()
                        ));
                    }
                }
            }
        }
    }

    // ---- explicit tc range / drop-legality checks (soft, preserved) --------
    if let Some(tc) = &raw_tc {
        match Label::parse(tc) {
            Ok((lab, marker)) => {
                let nominal = spec.rate.nominal_fps();
                if let Err(e) = crate::timecode::validate_label(&lab, nominal) {
                    diagnostics.push(format!("tc: {e}"));
                }
                if marker && !spec.df {
                    diagnostics.push("tc uses drop marker but source df=0".into());
                }
                if !marker && spec.df {
                    diagnostics.push("source df=1 but tc uses non-drop separator".into());
                }
            }
            Err(e) => diagnostics.push(format!("tc: {e}")),
        }
    }

    // ---- clock normalization (raw always preserved) ------------------------
    let device_clock_ns = match &raw_clock {
        Some(c) => match parse_clock_ns(c) {
            Ok(ns) => Some(ns),
            Err(e) => {
                diagnostics.push(format!("clock: {e}"));
                None
            }
        },
        None => None,
    };

    if fingerprint.is_none() && rec == "V" {
        diagnostics.push("video summary without fp= cannot participate in fingerprint alignment".into());
    }
    if ub_override.is_some() {
        // user bits supplied outside a bit payload are informational only
        soft.push(format!("line {line_no}: ub= outside bits payload recorded as raw note"));
    }

    Ok(Observation {
        source_id,
        seq: 0,
        kind: spec.kind,
        raw_line: line.to_string(),
        raw_bits,
        raw_rate: spec.rate_raw,
        raw_tc,
        raw_device_clock: raw_clock,
        device_clock_ns,
        fingerprint,
        rate: Rat::parse(&spec.rate_raw).unwrap(),
        df: spec.df,
        field_mode,
        diagnostics,
    })
}

fn without_flags(l: &Label) -> Label {
    Label {
        h: l.h,
        m: l.m,
        s: l.s,
        f: l.f,
        drop_frame: l.drop_frame,
        color_frame: l.color_frame,
        field_flag: l.field_flag,
    }
}

/// Convenience used by the generator/case tools: virtual clock stamp helper.
pub fn stamp(_clock: &dyn Clock) {
    // placeholder retained for API symmetry; timestamps are assigned in storage.
}
