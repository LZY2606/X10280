// tcimport/1 importer.
//
// The importer is deliberately format-only: it parses and preserves original
// spellings. Semantic interpretation (BCD decode, rate/drop compatibility,
// clock parsing, ...) happens in the derived layer so that unconvertible
// values stay in place with diagnostics instead of being dropped.
//
// Grammar (line oriented, `#` starts a comment):
//
//   @FORMAT tcimport/1
//   @SOURCE <LTC|VITC|VIDEO|LOG> <source-id>
//   @RATE <num/den | integer fps>           # required before OBS, frames/second
//   @DROP <true|false>                      # source-stated drop-frame flag
//   @COLOR <true|false>                     # source-stated color-frame flag
//   @NOTE <free text>
//   OBS ordinal=<u64> [bits=<0/1...>] [tc=<HH:MM:SS:FF>] [drop=<bool>]
//       [color=<bool>] [field=<1|2>] [ub=<hex up to 8 bytes>]
//       [fp=<fingerprint>] [clock=<ISO8601>] [q=<0..1>] [ubraw?]
//
// Multiple @SOURCE blocks may appear in one submission.

use crate::domain::{Field, Rate, Reading, Timecode};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SourceKind {
    Ltc,
    Vitc,
    Video,
    Log,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Ltc => "LTC",
            SourceKind::Vitc => "VITC",
            SourceKind::Video => "VIDEO",
            SourceKind::Log => "LOG",
        }
    }
    pub fn parse(s: &str) -> Option<SourceKind> {
        Some(match s {
            "LTC" => SourceKind::Ltc,
            "VITC" => SourceKind::Vitc,
            "VIDEO" => SourceKind::Video,
            "LOG" => SourceKind::Log,
            _ => return None,
        })
    }
    pub fn expected_bit_len(self) -> Option<usize> {
        match self {
            SourceKind::Ltc => Some(80),
            SourceKind::Vitc => Some(90),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ParsedSource {
    pub source_id: String,
    pub kind: SourceKind,
    pub rate: Rate,
    pub drop_stated: Option<bool>,
    pub color_stated: Option<bool>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedRow {
    pub source_index: usize,
    pub reading: Reading,
    /// Non-fatal, value-domain problems. Raw values are retained regardless.
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedImport {
    pub sources: Vec<ParsedSource>,
    pub rows: Vec<ParsedRow>,
}

fn parse_bool(line: usize, key: &str, v: &str) -> Result<bool, ImportError> {
    match v {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ImportError {
            line,
            message: format!("`{key}` must be true or false, got `{v}`"),
        }),
    }
}

fn parse_field(line: usize, v: &str) -> Result<Field, ImportError> {
    match v {
        "1" | "first" | "F1" => Ok(Field::First),
        "2" | "second" | "F2" => Ok(Field::Second),
        _ => Err(ImportError {
            line,
            message: format!("`field` must be 1/first or 2/second, got `{v}`"),
        }),
    }
}

pub fn parse(text: &str) -> Result<ParsedImport, Vec<ImportError>> {
    let mut out = ParsedImport::default();
    let mut errors: Vec<ImportError> = Vec::new();
    let mut cur: Option<usize> = None;
    let mut seen_ordinals: Vec<std::collections::HashSet<u64>> = Vec::new();
    let mut format_seen = false;

    for (idx, raw_line) in text.lines().enumerate() {
        let line = idx + 1;
        let l = raw_line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix('@') {
            let mut it = rest.split_whitespace();
            let directive = it.next().unwrap_or("");
            match directive {
                "FORMAT" => {
                    let v = it.next().unwrap_or("");
                    if v != "tcimport/1" {
                        errors.push(ImportError { line, message: format!("unsupported format `{v}` (expected tcimport/1)") });
                    }
                    format_seen = true;
                }
                "SOURCE" => {
                    let kind_s = it.next().unwrap_or("");
                    let id = it.next().unwrap_or("");
                    let kind = SourceKind::parse(kind_s).ok_or_else(|| ImportError {
                        line,
                        message: format!("unknown source kind `{kind_s}` (expected LTC|VITC|VIDEO|LOG)"),
                    });
                    if id.is_empty() {
                        errors.push(ImportError { line, message: "@SOURCE requires an id".into() });
                    } else if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                        errors.push(ImportError { line, message: format!("source id `{id}` must be alphanumeric/-/_") });
                    } else if out.sources.iter().any(|s| s.source_id == id) {
                        errors.push(ImportError { line, message: format!("source id `{id}` already declared in this submission") });
                    }
                    match kind {
                        Ok(kind) if !id.is_empty() && !out.sources.iter().any(|s| s.source_id == id) => {
                            out.sources.push(ParsedSource {
                                source_id: id.to_string(),
                                kind,
                                rate: Rate { num: 0, den: 1, raw: String::new() },
                                drop_stated: None,
                                color_stated: None,
                                note: None,
                            });
                            seen_ordinals.push(std::collections::HashSet::new());
                            cur = Some(out.sources.len() - 1);
                        }
                        _ => cur = None,
                    }
                }
                "RATE" => {
                    let v = it.next().unwrap_or("");
                    match Rate::parse(v) {
                        Ok(rate) => match cur {
                            Some(i) => out.sources[i].rate = rate,
                            None => errors.push(ImportError { line, message: "@RATE must follow @SOURCE".into() }),
                        },
                        Err(e) => errors.push(ImportError { line, message: e }),
                    }
                }
                "DROP" => {
                    let v = it.next().unwrap_or("");
                    match parse_bool(line, "DROP", v) {
                        Ok(b) => match cur {
                            Some(i) => out.sources[i].drop_stated = Some(b),
                            None => errors.push(ImportError { line, message: "@DROP must follow @SOURCE".into() }),
                        },
                        Err(e) => errors.push(e),
                    }
                }
                "COLOR" => {
                    let v = it.next().unwrap_or("");
                    match parse_bool(line, "COLOR", v) {
                        Ok(b) => match cur {
                            Some(i) => out.sources[i].color_stated = Some(b),
                            None => errors.push(ImportError { line, message: "@COLOR must follow @SOURCE".into() }),
                        },
                        Err(e) => errors.push(e),
                    }
                }
                "NOTE" => {
                    let note = rest.split_once(char::is_whitespace).map(|(_, n)| n.trim()).unwrap_or("");
                    match cur {
                        Some(i) => out.sources[i].note = Some(note.to_string()),
                        None => errors.push(ImportError { line, message: "@NOTE must follow @SOURCE".into() }),
                    }
                }
                other => errors.push(ImportError { line, message: format!("unknown directive `@{other}`") }),
            }
            continue;
        }

        if let Some(rest) = l.strip_prefix("OBS") {
            if !format_seen {
                errors.push(ImportError { line, message: "missing @FORMAT tcimport/1 header".into() });
            }
            let Some(si) = cur else {
                errors.push(ImportError { line, message: "OBS outside of a @SOURCE block".into() });
                continue;
            };
            if out.sources[si].rate.num == 0 {
                errors.push(ImportError { line, message: "OBS before @RATE for this source".into() });
                continue;
            }
            let mut reading = Reading {
                ordinal: 0,
                bits_raw: String::new(),
                tc_raw: String::new(),
                tc: None,
                tc_drop_spelled: None,
                ub_raw: String::new(),
                color_frame: None,
                field: Field::Unset,
                fp_raw: String::new(),
                clock_raw: String::new(),
                quality: None,
            };
            let mut diagnostics = Vec::new();
            let mut has_ordinal = false;
            for tok in rest.split_whitespace() {
                let Some((key, val)) = tok.split_once('=') else {
                    errors.push(ImportError { line, message: format!("malformed token `{tok}` (expected key=value)") });
                    continue;
                };
                match key {
                    "ordinal" => match val.parse::<u64>() {
                        Ok(o) => {
                            if !seen_ordinals[si].insert(o) {
                                errors.push(ImportError { line, message: format!("duplicate ordinal {o} for this source in this submission") });
                            }
                            reading.ordinal = o;
                            has_ordinal = true;
                        }
                        Err(e) => errors.push(ImportError { line, message: format!("bad ordinal: {e}") }),
                    },
                    "bits" => {
                        let expected = out.sources[si].kind.expected_bit_len();
                        if !val.bytes().all(|b| b == b'0' || b == b'1') {
                            errors.push(ImportError { line, message: "bits must contain only 0/1".into() });
                        } else if let Some(n) = expected {
                            if val.len() != n {
                                errors.push(ImportError { line, message: format!("{} bits must be {n} characters, got {}", out.sources[si].kind.as_str(), val.len()) });
                            }
                        }
                        reading.bits_raw = val.to_string();
                    }
                    "tc" => {
                        match Timecode::parse(val) {
                            Ok((tc, drop, _sep)) => {
                                reading.tc = Some(tc);
                                reading.tc_drop_spelled = Some(drop);
                                reading.tc_raw = val.to_string();
                            }
                            Err(e) => errors.push(ImportError { line, message: e }),
                        }
                    }
                    "drop" => match parse_bool(line, "drop", val) {
                        Ok(b) => reading.tc_drop_spelled = Some(b),
                        Err(e) => errors.push(e),
                    },
                    "color" => match parse_bool(line, "color", val) {
                        Ok(b) => reading.color_frame = Some(b),
                        Err(e) => errors.push(e),
                    },
                    "field" => match parse_field(line, val) {
                        Ok(f) => reading.field = f,
                        Err(e) => errors.push(e),
                    },
                    "ub" => {
                        if val.len() > 8 || !val.bytes().all(|b| b.is_ascii_hexdigit()) {
                            diagnostics.push(format!("user bits `{val}` are not 1..=8 hex digits; kept raw and left unconverted"));
                        }
                        reading.ub_raw = val.to_string();
                    }
                    "fp" => reading.fp_raw = val.to_string(),
                    "clock" => reading.clock_raw = val.to_string(),
                    "q" => match val.parse::<f64>() {
                        Ok(q) if (0.0..=1.0).contains(&q) => reading.quality = Some(q),
                        _ => errors.push(ImportError { line, message: format!("q must be a number in 0..1, got `{val}`") }),
                    },
                    other => errors.push(ImportError { line, message: format!("unknown OBS field `{other}`") }),
                }
            }
            if !has_ordinal {
                errors.push(ImportError { line, message: "OBS requires ordinal=<u64>".into() });
            }
            if reading.bits_raw.is_empty()
                && reading.tc_raw.is_empty()
                && reading.fp_raw.is_empty()
                && reading.clock_raw.is_empty()
            {
                errors.push(ImportError { line, message: "OBS carries no data (need one of bits/tc/fp/clock)".into() });
            }
            out.rows.push(ParsedRow { source_index: si, reading, diagnostics });
        } else {
            errors.push(ImportError { line, message: format!("unrecognized line `{l}`") });
        }
    }

    if !format_seen && errors.is_empty() {
        errors.push(ImportError { line: 0, message: "empty input or missing @FORMAT tcimport/1 header".into() });
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}
