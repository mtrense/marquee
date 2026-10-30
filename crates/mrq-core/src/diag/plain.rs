//! Plain-text rendering: compiler-style `file:line:col: severity[code]: message` lines.
//!
//! This is the human format when the `human` feature (miette) is disabled.

use std::io::{self, Write};
use std::path::Path;

use super::Diagnostic;
use super::location::{display_path, find_source, resolve};
use crate::source::{ByteRange, SourceFile};

fn location(path: &Path, source: Option<&SourceFile>, range: ByteRange) -> String {
    let path = display_path(path);
    match resolve(source, range) {
        Some((start, _)) => format!("{path}:{}:{}", start.line, start.column),
        None => path,
    }
}

fn indent(text: &str) -> String {
    text.replace('\n', "\n  ")
}

/// Writes each diagnostic as a `file:line:col: severity[code]: message` line, followed
/// by indented `label`, `fix`, `help` and `see` lines.
pub fn emit_plain(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    out: &mut impl Write,
) -> io::Result<()> {
    for diag in diagnostics {
        let source = find_source(sources, &diag.file);
        writeln!(
            out,
            "{}: {}[{}]: {}",
            location(&diag.file, source, diag.range),
            diag.severity,
            diag.code,
            indent(&diag.message)
        )?;
        for label in &diag.labels {
            writeln!(
                out,
                "  label: {}: {}",
                location(&diag.file, source, label.range),
                indent(&label.message)
            )?;
        }
        if let Some(fix) = &diag.fix {
            writeln!(out, "  fix: {}", indent(&fix.message))?;
        }
        if let Some(help) = &diag.help {
            writeln!(out, "  help: {}", indent(help))?;
        }
        if let Some(url) = &diag.url {
            writeln!(out, "  see: {url}")?;
        }
    }
    Ok(())
}
