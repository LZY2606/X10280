//! Import layer.
//!
//! Input grammar (deliberately tiny and copy-pastable):
//!
//! ```text
//! MATERIAL id=cam_a kind=ltc rate=30000/1001 dropframe=true color=false
//! title="Camera A LTC"
//! 00:00:59;28 user=00 00 00 00 field=frame quality=98 fp=hash1 clock=2026-01-02T10:00:00
//! ...
//! MATERIAL id=log1 kind=log rate=30000/1001 dropframe=true color=false
//! note="recording log text"
//! ```
//!
//! Blank lines and `#` comments are allowed.  A parse error names the line and
//! column-ish token; a successfully-parsed but semantically invalid row is
//! imported verbatim and attached as a diagnostic instead of being dropped.

use crate::domain::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestError {
    pub line: usize,
    pub code: &'static str,
    pub message: String,
    pub raw: String,
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {} [{}]: {}", self.line, self.code, self.message)
    }
}

/// Split `key=value` tokens, honoring double-quoted values with spaces.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = false;
    let mut started = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                quote = !quote;
                started = true;
            }
            c if c.is_whitespace() && !quote => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quote {
        return Err("unterminated quoted value".to_string());
    }
    if started {
        out.push(cur);
    }
    Ok(out)
}

fn kv(token: &str) -> Option<(&str, &str)> {
    token.split_once('=')
}

/// Parse a complete paste into zero or more material versions.
pub fn parse_paste(text: &str) -> Result<Vec<MaterialVersion>, Vec<IngestError>> {
    let mut materials = Vec::new();
    let mut errors = Vec::new();

    let mut header: Option<Header> = None;
    let mut rows: Vec<(usize, String)> = Vec::new();

    for (idx, raw_full) in text.lines().enumerate() {
        let lineno = idx + 1;
        let strip_comment = |s: &str| -> String {
            // `#` only starts a comment outside quotes.
            let mut quote = false;
            let mut out = String::new();
            for c in s.chars() {
                if c == '"' {
                    quote = !quote;
                }
                if c == '#' && !quote {
                    break;
                }
                out.push(c);
            }
            out
        };
        let line_raw = strip_comment(raw_full);
        let line = line_raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("MATERIAL") {
            if let Some(h) = header.take() {
                commit(&mut materials, h, std::mem::take(&mut rows), &mut errors);
            }
            match parse_header(lineno, raw_full, line) {
                Ok(h) => header = Some(h),
                Err(e) => {
                    errors.push(e);
                    header = None;
                }
            }
            continue;
        }
        if header.is_none() {
            errors.push(IngestError {
                line: lineno,
                code: "orphan_row",
                message: "sample row appears before any MATERIAL header".to_string(),
                raw: raw_full.to_string(),
            });
            continue;
        }
        rows.push((lineno, raw_full.to_string()));
    }
    if let Some(h) = header.take() {
        commit(&mut materials, h, rows, &mut errors);
    }

    if errors.is_empty() {
        Ok(materials)
    } else {
        Err(errors)
    }
}

struct Header {
    id: String,
    kind: SourceKind,
    rate: String,
    df: bool,
    color: bool,
    title: String,
    note: Option<String>,
    line: usize,
}

fn parse_header(lineno: usize, raw_full: &str, line: &str) -> Result<Header, IngestError> {
    let tokens = tokenize(line).map_err(|m| IngestError {
        line: lineno,
        code: "bad_header",
        message: m,
        raw: raw_full.to_string(),
    })?;
    let mut id = None;
    let mut kind = None;
    let mut rate = None;
    let mut df = false;
    let mut color = false;
    let mut title = None;
    let mut note = None;
    for tok in &tokens[1..] {
        let (k, v) = kv(tok).ok_or_else(|| IngestError {
            line: lineno,
            code: "bad_header_token",
            message: format!("'{tok}' is not key=value"),
            raw: raw_full.to_string(),
        })?;
        match k {
            "id" => id = Some(v.to_string()),
            "kind" => {
                kind = Some(SourceKind::parse(v).ok_or_else(|| IngestError {
                    line: lineno,
                    code: "unknown_kind",
                    message: format!("kind '{v}' is not ltc|vitc|video|log"),
                    raw: raw_full.to_string(),
                })?);
            }
            "rate" => rate = Some(v.to_string()),
            "dropframe" | "df" => {
                df = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "df")
            }
            "color" | "colorframe" => {
                color = matches!(v.to_ascii_lowercase().as_str(), "true" | "1")
            }
            "title" => title = Some(v.to_string()),
            "note" => note = Some(v.to_string()),
            other => {
                return Err(IngestError {
                    line: lineno,
                    code: "unknown_header_field",
                    message: format!("unknown header field '{other}'"),
                    raw: raw_full.to_string(),
                });
            }
        }
    }
    let id = id.ok_or_else(|| IngestError {
        line: lineno,
        code: "missing_id",
        message: "MATERIAL header requires id=".to_string(),
        raw: raw_full.to_string(),
    })?;
    let kind = kind.ok_or_else(|| IngestError {
        line: lineno,
        code: "missing_kind",
        message: "MATERIAL header requires kind=".to_string(),
        raw: raw_full.to_string(),
    })?;
    let rate = rate.ok_or_else(|| IngestError {
        line: lineno,
        code: "missing_rate",
        message: "MATERIAL header requires rate= as a rational e.g. 30000/1001".to_string(),
        raw: raw_full.to_string(),
    })?;
    Ok(Header {
        id,
        kind,
        rate,
        df,
        color,
        title: title.unwrap_or_else(|| "untitled".to_string()),
        note,
        line: lineno,
    })
}

fn commit(
    materials: &mut Vec<MaterialVersion>,
    h: Header,
    rows: Vec<(usize, String)>,
    errors: &mut Vec<IngestError>,
) {
    // Validate the rational here (still part of "explicit format errors").
    if Rate::parse(&h.rate).is_none() {
        errors.push(IngestError {
            line: h.line,
            code: "bad_rate",
            message: format!("rate '{}' is not a positive rational", h.rate),
            raw: String::new(),
        });
        return;
    }
    let mut samples = Vec::new();
    for (lineno, raw_full) in rows {
        match parse_sample(lineno, &raw_full) {
            Ok(s) => samples.push(s),
            Err(e) => errors.push(e),
        }
    }
    materials.push(MaterialVersion {
        material_id: h.id,
        version: 1,
        parent_version: None,
        kind: h.kind,
        rate_text: h.rate,
        drop_frame: h.df,
        color_frame: h.color,
        title: h.title,
        samples,
        note: h.note,
    });
}

fn parse_sample(lineno: usize, raw_full: &str) -> Result<Sample, IngestError> {
    let line = raw_full.trim();
    let tokens = tokenize(line).map_err(|m| IngestError {
        line: lineno,
        code: "bad_row",
        message: m,
        raw: raw_full.to_string(),
    })?;
    if tokens.is_empty() {
        return Err(IngestError {
            line: lineno,
            code: "empty_row",
            message: "empty sample row".to_string(),
            raw: raw_full.to_string(),
        });
    }
    let label_text = tokens[0].clone();
    // Structural label check up front; semantic legality (DF skips, ranges) is
    // handled downstream as a diagnostic so the raw row is retained.
    if Label::parse(&label_text).is_err() {
        return Err(IngestError {
            line: lineno,
            code: "bad_label",
            message: format!("'{label_text}' is not HH:MM:SS:FF"),
            raw: raw_full.to_string(),
        });
    }
    let mut user_bits = String::new();
    let mut field = FieldKind::Frame;
    let mut quality = None;
    let mut quality_text = None;
    let mut fingerprint = None;
    let mut device_clock = None;
    for tok in &tokens[1..] {
        let (k, v) = kv(tok).ok_or_else(|| IngestError {
            line: lineno,
            code: "bad_row_token",
            message: format!("'{tok}' is not key=value"),
            raw: raw_full.to_string(),
        })?;
        match k {
            "user" | "ub" | "userbits" => user_bits = v.to_string(),
            "field" => field = FieldKind::parse(v),
            "q" | "quality" => {
                quality_text = Some(v.to_string());
                quality = v.parse::<u8>().ok().filter(|n| *n <= 100);
                if quality.is_none() {
                    return Err(IngestError {
                        line: lineno,
                        code: "bad_quality",
                        message: format!("quality '{v}' must be an integer 0..100"),
                        raw: raw_full.to_string(),
                    });
                }
            }
            "fp" | "fingerprint" => fingerprint = Some(v.to_string()),
            "clock" | "device_clock" => device_clock = Some(v.to_string()),
            other => {
                return Err(IngestError {
                    line: lineno,
                    code: "unknown_row_field",
                    message: format!("unknown field '{other}' (expected user= field= quality= fp= clock=)"),
                    raw: raw_full.to_string(),
                });
            }
        }
    }
    Ok(Sample {
        ordinal: 0, // assigned by project after successful import
        label_text,
        user_bits,
        field,
        quality,
        quality_text,
        fingerprint,
        device_clock,
        raw_line: raw_full.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_material() {
        let paste = "MATERIAL id=a kind=ltc rate=30000/1001 df=true\n00:00:00;00 user=ABCD\n";
        let ms = parse_paste(paste).unwrap();
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].samples.len(), 1);
        assert_eq!(ms[0].samples[0].user_bits, "ABCD");
    }

    #[test]
    fn explicit_errors_name_line_and_field() {
        let paste = "MATERIAL id=a kind=ltc\n00:00:00:00\n";
        let errs = parse_paste(paste).unwrap_err();
        assert_eq!(errs[0].line, 1);
        assert_eq!(errs[0].code, "missing_rate");
        let paste2 = "00:00:00:00\n";
        assert_eq!(parse_paste(paste2).unwrap_err()[0].code, "orphan_row");
        let paste3 = "MATERIAL id=a kind=ltc rate=25/1\n99:00:00:00\n";
        assert_eq!(parse_paste(paste3).unwrap_err()[0].code, "bad_label");
    }
}
