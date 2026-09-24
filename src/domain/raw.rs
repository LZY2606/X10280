//! Raw, never-normalized inputs and the copyable sample format parser.
//!
//! Format (line oriented, UTF-8, `#` comments, blank lines ignored):
//!
//! ```text
//! BLOCK LTC camera_a RATE 30000/1001 DF
//! FIELD 1
//! 00:00:00;00 bits=1010.. quality=0.98 u=0x00 cf=0 fp=sha256:aaaa dev=2026-09-23T10:00:00+09:00
//! ...
//! END BLOCK
//! ```
//!
//! The parser keeps the ORIGINAL textual representation of every field. Values
//! that cannot be converted stay in place with a diagnostic; they are never
//! silently dropped to make a plot work.

use serde::{Deserialize, Serialize};

use crate::domain::FrameRate;

/// Provenance category of a source block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Ltc,
    Vitc,
    FrameSummary,
    RecordLog,
}

impl SourceKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s.to_ascii_uppercase().as_str() {
            "LTC" => SourceKind::Ltc,
            "VITC" => SourceKind::Vitc,
            "FRAME" | "FRAMESUMMARY" | "SUMMARY" => SourceKind::FrameSummary,
            "LOG" | "RECORDLOG" => SourceKind::RecordLog,
            other => return Err(format!("unknown source kind {other:?} (want LTC|VITC|FRAME|LOG)")),
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Ltc => "LTC",
            SourceKind::Vitc => "VITC",
            SourceKind::FrameSummary => "FRAME",
            SourceKind::RecordLog => "LOG",
        }
    }
}

/// One fully-preserved observation. Nothing here is normalized away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecord {
    /// 1-based line in the submitted text, for audit trails.
    pub line_no: usize,
    /// Original timecode token exactly as submitted (before any parsing).
    pub tc_raw: String,
    /// Original raw bits exactly as submitted (may be empty for non-TC sources).
    pub bits_raw: String,
    pub quality_raw: String,
    pub user_bits_raw: String,
    pub color_frame_raw: String,
    pub field_raw: String,
    /// Frame fingerprint token exactly as submitted (e.g. `sha256:ab12...`).
    pub fingerprint: String,
    /// Device clock string exactly as submitted.
    pub device_clock_raw: String,
    /// Direction marker exactly as submitted (`FWD`/`REV`/empty).
    pub direction_raw: String,
    /// Free-form retained extras (`key=value`), preserving unknown tokens.
    pub extras: Vec<(String, String)>,
}

/// A parsed source block. Header flags and records are preserved verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawBlock {
    pub kind: SourceKind,
    pub source_id: String,
    pub rate_num: u64,
    pub rate_den: u64,
    pub drop_frame: bool,
    /// Block-level field mode token exactly as submitted (empty if absent).
    pub field_mode_raw: String,
    pub records: Vec<RawRecord>,
}

impl RawBlock {
    pub fn rate(&self) -> FrameRate {
        FrameRate::new(self.rate_num, self.rate_den)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedImport {
    pub blocks: Vec<RawBlock>,
    /// Diagnostics that did NOT prevent import (value kept in place).
    pub warnings: Vec<ImportDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDiagnostic {
    pub line_no: usize,
    pub code: String,
    pub message: String,
    pub severity: String, // ERROR | WARNING
}

/// Result of parsing: hard errors block submission and name the exact line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line_no: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line_no, self.message)
    }
}

struct Tokens<'a> {
    items: Vec<&'a str>,
}

impl<'a> Tokens<'a> {
    fn new(line: &'a str) -> Self {
        Tokens { items: line.split_whitespace().collect() }
    }
    fn take_flag(&mut self, name: &str) -> bool {
        if let Some(pos) = self.items.iter().position(|t| t.eq_ignore_ascii_case(name)) {
            self.items.remove(pos);
            true
        } else {
            false
        }
    }
    fn take_value(&mut self, name: &str) -> Option<String> {
        let lname = name.to_ascii_lowercase();
        let pos = self.items.iter().position(|t| {
            let (k, _) = t.split_once('=').unwrap_or((t, ""));
            k.eq_ignore_ascii_case(&lname)
        })?;
        let tok = self.items.remove(pos);
        Some(match tok.split_once('=') {
            Some((_, v)) => v.to_string(),
            None => String::new(),
        });
    }
}

fn err<T>(line: usize, msg: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError { line_no: line, message: msg.into() })
}

/// Parse a sample document. Hard syntax/header errors return `ParseError`.
pub fn parse_import(text: &str) -> Result<ParsedImport, ParseError> {
    let mut blocks: Vec<RawBlock> = Vec::new();
    let mut warnings: Vec<ImportDiagnostic> = Vec::new();
    let mut current: Option<RawBlock> = None;

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let upper: Vec<String> = line.split_whitespace().map(|s| s.to_ascii_uppercase()).collect();

        if upper[0] == "BLOCK" {
            if current.is_some() {
                return err(line_no, "nested BLOCK without END BLOCK");
            }
            let mut toks = Tokens::new(line);
            toks.items.remove(0); // BLOCK
            let kind_tok = toks.items.first().copied();
            let kind = match kind_tok {
                Some(k) => SourceKind::parse(k).map_err(|m| ParseError { line_no, message: m })?,
                None => return err(line_no, "BLOCK requires a kind (LTC|VITC|FRAME|LOG)"),
            };
            toks.items.remove(0);
            let source_id = match toks.items.first().copied() {
                Some(s) if !s.to_ascii_uppercase().starts_with("RATE") => {
                    let v = s.to_string();
                    toks.items.remove(0);
                    v
                }
                _ => return err(line_no, "BLOCK requires a source id"),
            };
            let mut rate_num = 0u64;
            let mut rate_den = 1u64;
            if let Some(pos) = toks.items.iter().position(|t| t.eq_ignore_ascii_case("RATE")) {
                toks.items.remove(pos);
                let r = toks.items.get(pos).copied();
                match r {
                    Some(rv) => {
                        toks.items.remove(pos);
                        (rate_num, rate_den) = parse_rate(rv)
                            .map_err(|m| ParseError { line_no, message: m })?;
                    }
                    None => return err(line_no, "RATE requires num/den, e.g. RATE 30000/1001"),
                }
            } else {
                return err(line_no, "BLOCK requires RATE num/den");
            }
            let df = toks.take_flag("DF");
            let ndf = toks.take_flag("NDF");
            if df && ndf {
                return err(line_no, "BLOCK cannot be both DF and NDF");
            }
            let mut field_mode = String::new();
            if let Some(pos) = toks.items.iter().position(|t| t.eq_ignore_ascii_case("FIELD")) {
                toks.items.remove(pos);
                match toks.items.get(pos).copied() {
                    Some(v) => {
                        toks.items.remove(pos);
                        field_mode = v.to_string();
                        if v != "1" && v != "2" {
                            warnings.push(ImportDiagnostic {
                                line_no,
                                code: "FIELD_MODE_UNKNOWN".into(),
                                message: format!("FIELD {v} kept verbatim; expected 1 or 2"),
                                severity: "WARNING".into(),
                            });
                        }
                    }
                    None => return err(line_no, "FIELD requires 1 or 2"),
                }
            }
            if !toks.items.is_empty() {
                return err(line_no, format!("unexpected tokens in BLOCK header: {}", toks.items.join(" ")));
            }
            current = Some(RawBlock {
                kind,
                source_id,
                rate_num,
                rate_den,
                drop_frame: df,
                field_mode_raw: field_mode,
                records: Vec::new(),
            });
            continue;
        }

        if upper[0] == "END" && upper.get(1).map(|s| s.as_str()) == Some("BLOCK") {
            match current.take() {
                Some(b) if line.split_whitespace().count() == 2 => blocks.push(b),
                Some(_) => return err(line_no, "END BLOCK takes no extra tokens"),
                None => return err(line_no, "END BLOCK without BLOCK"),
            }
            continue;
        }

        // Data line belongs to current block.
        let block = match current.as_mut() {
            Some(b) => b,
            None => return err(line_no, "data line outside of a BLOCK"),
        };
        let record = parse_record(line, line_no, block, &mut warnings)?;
        block.records.push(record);
    }

    if current.is_some() {
        return err(usize::MAX, "unexpected end of input: missing END BLOCK");
    }
    if blocks.is_empty() {
        return err(0, "input contains no BLOCK");
    }
    Ok(ParsedImport { blocks, warnings })
}

fn parse_rate(rv: &str) -> Result<(u64, u64), String> {
    let (n, d) = rv.split_once('/').ok_or_else(|| format!("RATE {rv:?} must be num/den"))?;
    let num = n.parse::<u64>().map_err(|_| format!("RATE numerator {n:?} not an integer"))?;
    let den = d.parse::<u64>().map_err(|_| format!("RATE denominator {d:?} not an integer"))?;
    if num == 0 || den == 0 {
        return Err(format!("RATE {rv} must be positive"));
    }
    Ok((num, den))
}

fn parse_record(
    line: &str,
    line_no: usize,
    block: &RawBlock,
    warnings: &mut Vec<ImportDiagnostic>,
) -> Result<RawRecord, ParseError> {
    let mut toks = Tokens::new(line);
    // First token is the timecode label (required for LTC/VITC; optional for
    // FRAME/LOG where a fingerprint or dev clock may lead).
    let first = toks.items.first().copied().unwrap_or("").to_string();
    let looks_like_tc = first.contains(':') || first.contains(';') || first.contains('.');
    let tc_raw = if looks_like_tc {
        toks.items.remove(0);
        first.clone()
    } else {
        String::new()
    };

    let bits_raw = toks.take_value("bits").unwrap_or_default();
    let quality_raw = toks.take_value("quality").unwrap_or_default();
    let user_bits_raw = toks
        .take_value("u")
        .or_else(|| toks.take_value("user"))
        .or_else(|| toks.take_value("ubits"))
        .unwrap_or_default();
    let color_frame_raw = toks.take_value("cf").unwrap_or_default();
    let field_raw = toks.take_value("field").unwrap_or_default();
    let fingerprint = toks
        .take_value("fp")
        .or_else(|| toks.take_value("fingerprint"))
        .unwrap_or_default();
    let device_clock_raw = toks
        .take_value("dev")
        .or_else(|| toks.take_value("clock"))
        .unwrap_or_default();
    let direction_raw = toks.take_value("dir").unwrap_or_default();

    // Everything left must be key=value extras; bare tokens are a hard error so
    // typos cannot silently vanish.
    let mut extras = Vec::new();
    for tok in toks.items.drain(..) {
        match tok.split_once('=') {
            Some((k, v)) => extras.push((k.to_string(), v.to_string())),
            None => {
                return Err(ParseError {
                    line_no,
                    message: format!("unrecognized bare token {tok:?}; use key=value"),
                })
            }
        }
    }

    if tc_raw.is_empty() && matches!(block.kind, SourceKind::Ltc | SourceKind::Vitc) {
        return Err(ParseError { line_no, message: "LTC/VITC record must start with a timecode".into() });
    }
    if !tc_raw.is_empty() {
        if let Err(m) = crate::domain::TcLabel::parse(&tc_raw) {
            // Keep the raw token in place; downstream leaves it un-normalized.
            warnings.push(ImportDiagnostic {
                line_no,
                code: "TC_UNPARSEABLE".into(),
                message: format!("timecode {tc_raw:?} kept raw: {m}"),
                severity: "WARNING".into(),
            });
        }
    }
    if !bits_raw.is_empty() && !bits_raw.bytes().all(|b| b == b'0' || b == b'1') {
        warnings.push(ImportDiagnostic {
            line_no,
            code: "BITS_NOT_BINARY".into(),
            message: format!("bits {bits_raw:?} kept raw; expected only 0/1"),
            severity: "WARNING".into(),
        });
    }
    if !quality_raw.is_empty() && quality_raw.parse::<f64>().map(|q| (0.0..=1.0).contains(&q)).unwrap_or(false) == false
    {
        warnings.push(ImportDiagnostic {
            line_no,
            code: "QUALITY_RANGE".into(),
            message: format!("quality {quality_raw:?} kept raw; expected number in 0..=1"),
            severity: "WARNING".into(),
        });
    }
    if block.drop_frame
        && block.rate().df_allowed()
        && !tc_raw.is_empty()
        && crate::domain::TcLabel::parse(&tc_raw).map(|(l, _)| l).ok().is_some()
    {
        if let Ok((label, _)) = crate::domain::TcLabel::parse(&tc_raw) {
            if !label.drop_frame {
                warnings.push(ImportDiagnostic {
                    line_no,
                    code: "TC_DF_FLAG_MISMATCH".into(),
                    message: format!("block is DF but label {tc_raw} uses ':' separators"),
                    severity: "WARNING".into(),
                });
            }
        }
    }

    Ok(RawRecord {
        line_no,
        tc_raw,
        bits_raw,
        quality_raw,
        user_bits_raw,
        color_frame_raw,
        field_raw,
        fingerprint,
        device_clock_raw,
        direction_raw,
        extras,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# header comment
BLOCK LTC cam_a RATE 30000/1001 DF
00:00:00;00 bits=1010 quality=0.99 u=0x00 cf=0 fp=sha256:a1 dev=2026-09-23T10:00:00+09:00
00:00:59;29 bits=1011 quality=0.97 u=0x00 cf=0 fp=sha256:a2 dev=2026-09-23T10:00:01+09:00
END BLOCK
BLOCK FRAME vsum RATE 30000/1001
fp=sha256:a1 dev=2026-09-23T10:00:00+09:00
END BLOCK
";

    #[test]
    fn parses_valid_sample() {
        let p = parse_import(SAMPLE).unwrap();
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(p.blocks[0].kind, SourceKind::Ltc);
        assert!(p.blocks[0].drop_frame);
        assert_eq!(p.blocks[0].records[1].bits_raw, "1011");
        assert_eq!(p.blocks[1].kind, SourceKind::FrameSummary);
    }

    #[test]
    fn hard_errors_name_lines() {
        assert!(parse_import("BLOCK LTC x RATE 30/1\nEND BLOCK\nnonsense").is_err());
        let e = parse_import("BLOCK FOO x RATE 30/1\nEND BLOCK").unwrap_err();
        assert_eq!(e.line_no, 1);
        let e = parse_import("BLOCK LTC\nEND BLOCK").unwrap_err();
        assert!(e.message.contains("source id"));
        let e = parse_import("BLOCK LTC x RATE 30/1\nBLOCK LTC y RATE 30/1").unwrap_err();
        assert!(e.message.contains("nested"));
        let e = parse_import("00:00:00;00 bits=10").unwrap_err();
        assert!(e.message.contains("outside"));
    }

    #[test]
    fn unconvertible_value_stays_with_diagnostic() {
        let txt = "BLOCK LTC x RATE 30000/1001 DF\n00:00:00;00 bits=12AB quality=2.0\nEND BLOCK";
        let p = parse_import(txt).unwrap();
        assert_eq!(p.blocks[0].records[0].bits_raw, "12AB");
        assert!(p.warnings.iter().any(|w| w.code == "BITS_NOT_BINARY"));
        assert!(p.warnings.iter().any(|w| w.code == "QUALITY_RANGE"));
    }
}
