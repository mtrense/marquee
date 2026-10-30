//! `github`: GitHub Actions workflow commands that become check annotations.
//!
//! <https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands>

use std::fmt::Write as _;
use std::io::{self, Write};

use super::location::{display_path, find_source, resolve};
use super::{Diagnostic, Severity};
use crate::source::{ByteRange, SourceFile};

/// Escapes the message part of a workflow command.
fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escapes a `key=value` property of a workflow command.
fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

const fn command(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info | Severity::Hint => "notice",
    }
}

/// The range from the first to the last character of `range`, since GitHub's
/// `endColumn` is inclusive while byte ranges are half-open.
fn inclusive(source: Option<&SourceFile>, range: ByteRange) -> ByteRange {
    let last = source
        .filter(|_| !range.is_empty())
        .and_then(|source| source.text().get(range.start..range.end))
        .and_then(|text| text.char_indices().next_back())
        .map_or(range.start, |(i, _)| range.start + i);
    ByteRange::new(range.start, last)
}

fn render(diag: &Diagnostic, sources: &[SourceFile]) -> String {
    let source = find_source(sources, &diag.file);
    let mut line = format!(
        "::{} file={}",
        command(diag.severity),
        escape_property(&display_path(&diag.file))
    );
    if let Some((start, end)) = resolve(source, inclusive(source, diag.range)) {
        let _ = write!(
            line,
            ",line={},col={},endLine={},endColumn={}",
            start.line, start.column, end.line, end.column
        );
    }
    let mut message = diag.message.clone();
    if let Some(help) = &diag.help {
        let _ = write!(message, "\n\nhelp: {help}");
    }
    let _ = write!(
        line,
        ",title={}::{}",
        escape_property(&diag.code),
        escape_data(&message)
    );
    line
}

/// Writes one `::error`/`::warning`/`::notice` workflow command per diagnostic.
/// `info` and `hint` both map to `notice`; columns are 1-based and inclusive.
pub fn emit_github(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    out: &mut impl Write,
) -> io::Result<()> {
    for diag in diagnostics {
        writeln!(out, "{}", render(diag, sources))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn escapes_data() {
        assert_eq!(escape_data("50%\r\na:b,c"), "50%25%0D%0Aa:b,c");
    }

    #[test]
    fn escapes_properties() {
        assert_eq!(escape_property("C:\\a,b%\n"), "C%3A\\a%2Cb%25%0A");
    }

    #[test]
    fn maps_severities_to_commands() {
        assert_eq!(command(Severity::Error), "error");
        assert_eq!(command(Severity::Warning), "warning");
        assert_eq!(command(Severity::Info), "notice");
        assert_eq!(command(Severity::Hint), "notice");
    }

    #[test]
    fn empty_range_points_at_one_column() {
        let sources = [SourceFile::new(Some(PathBuf::from("a.md")), "abc")];
        let diag = Diagnostic::new("X", Severity::Error, "a.md", ByteRange::empty(1), "m");
        assert_eq!(
            render(&diag, &sources),
            "::error file=a.md,line=1,col=2,endLine=1,endColumn=2,title=X::m"
        );
    }

    #[test]
    fn end_column_is_the_last_character() {
        let sources = [SourceFile::new(Some(PathBuf::from("a.md")), "aé🦀\n")];
        let diag = Diagnostic::new("X", Severity::Warning, "a.md", ByteRange::new(0, 7), "m");
        assert_eq!(
            render(&diag, &sources),
            "::warning file=a.md,line=1,col=1,endLine=1,endColumn=3,title=X::m"
        );
    }
}
