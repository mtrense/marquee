//! `json` and `ndjson`: diagnostics with byte ranges and resolved lines/columns.

use std::io::{self, Write};

use serde::Serialize;

use super::location::{LineCol, display_path, find_source, resolve};
use super::{Diagnostic, Severity};
use crate::source::{ByteRange, SourceFile};

/// The JSON shape of one diagnostic. This is a public contract: add fields, never
/// rename or remove them.
#[derive(Serialize)]
struct JsonDiagnostic<'a> {
    code: &'a str,
    severity: Severity,
    message: &'a str,
    file: String,
    range: JsonRange,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<JsonLabel<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<JsonFix<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
}

#[derive(Serialize)]
struct JsonRange {
    /// Byte offsets into the file.
    start_offset: usize,
    end_offset: usize,
    /// 1-based line/column (UTF-16 columns), absent when the source is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<LineCol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<LineCol>,
}

#[derive(Serialize)]
struct JsonLabel<'a> {
    message: &'a str,
    range: JsonRange,
}

#[derive(Serialize)]
struct JsonFix<'a> {
    message: &'a str,
    edits: Vec<JsonEdit<'a>>,
}

#[derive(Serialize)]
struct JsonEdit<'a> {
    range: JsonRange,
    replacement: &'a str,
}

fn json_range(source: Option<&SourceFile>, range: ByteRange) -> JsonRange {
    let resolved = resolve(source, range);
    JsonRange {
        start_offset: range.start,
        end_offset: range.end,
        start: resolved.map(|(start, _)| start),
        end: resolved.map(|(_, end)| end),
    }
}

fn to_json<'a>(diag: &'a Diagnostic, sources: &[SourceFile]) -> JsonDiagnostic<'a> {
    let source = find_source(sources, &diag.file);
    JsonDiagnostic {
        code: &diag.code,
        severity: diag.severity,
        message: &diag.message,
        file: display_path(&diag.file),
        range: json_range(source, diag.range),
        labels: diag
            .labels
            .iter()
            .map(|label| JsonLabel {
                message: &label.message,
                range: json_range(source, label.range),
            })
            .collect(),
        fix: diag.fix.as_ref().map(|fix| JsonFix {
            message: &fix.message,
            edits: fix
                .edits
                .iter()
                .map(|edit| JsonEdit {
                    range: json_range(source, edit.range),
                    replacement: &edit.replacement,
                })
                .collect(),
        }),
        help: diag.help.as_deref(),
        url: diag.url.as_deref(),
    }
}

/// Writes all diagnostics as one pretty-printed JSON array, followed by a newline.
pub fn emit_json(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    out: &mut impl Write,
) -> io::Result<()> {
    let items: Vec<_> = diagnostics.iter().map(|d| to_json(d, sources)).collect();
    serde_json::to_writer_pretty(&mut *out, &items)?;
    writeln!(out)
}

/// Writes one compact JSON object per diagnostic, one per line.
pub fn emit_ndjson(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    out: &mut impl Write,
) -> io::Result<()> {
    for diag in diagnostics {
        serde_json::to_writer(&mut *out, &to_json(diag, sources))?;
        writeln!(out)?;
    }
    Ok(())
}
