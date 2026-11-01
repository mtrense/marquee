//! Formatting: `textDocument/formatting` with [`mrq_core::fmt`], the printer of `mrq fmt`,
//! and [`diff`], which turns a new text into edits of the lines that change (also for the
//! [`actions`](crate::actions) that rewrite a whole document).
//!
//! The `fmt` settings of the document's root folder win over the client's
//! `FormattingOptions` (tab size, spaces), which are ignored: Markdown has its own
//! indentation rules, and the result must be what `mrq fmt` writes. What fmt leaves as
//! written (code a formatter failed on, frontmatter it cannot normalize) is left as
//! written here too, without a warning.

use mrq_core::Document;
use mrq_core::config::FmtConfig;
use mrq_core::fmt::{CommandFormatter, format_with};
use mrq_core::source::{ByteRange, TextEdit};
use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::index::Root;

/// The edits that make `document` of `root` what `mrq fmt` prints: fenced code is
/// formatted by the configured `fmt.code_formatters`, as the CLI runs them. Empty if it
/// is formatted.
pub fn formatting(root: &Root, document: &Document) -> Vec<TextEdit> {
    let formatted = if let Some(loaded) = root.config() {
        let code = CommandFormatter::from_loaded(loaded);
        format_with(document, &loaded.config().fmt, &code)
    } else {
        let config = FmtConfig::default();
        format_with(document, &config, &CommandFormatter::from_config(&config))
    };
    diff(document.text(), &formatted.text)
}

/// The edits that turn `old` into `new`, line by line: one per run of changed lines,
/// replacing the old lines with the new ones; none if the texts are equal.
pub fn diff(old: &str, new: &str) -> Vec<TextEdit> {
    let (old_lines, new_lines) = (lines(old), lines(new));
    let ops = capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines);
    // The byte offset of each line start, and of the end.
    let starts = |lines: &[&str]| {
        let mut starts = vec![0];
        starts.extend(lines.iter().scan(0, |at, line| {
            *at += line.len();
            Some(*at)
        }));
        starts
    };
    let (old_starts, new_starts) = (starts(&old_lines), starts(&new_lines));
    let mut edits: Vec<TextEdit> = Vec::new();
    // The old and new line ranges of the run of changes being collected.
    let mut run: Option<(usize, usize, usize, usize)> = None;
    let mut flush = |run: &mut Option<(usize, usize, usize, usize)>| {
        if let Some((old_start, old_end, new_start, new_end)) = run.take() {
            edits.push(TextEdit {
                range: ByteRange::new(old_starts[old_start], old_starts[old_end]),
                replacement: new[new_starts[new_start]..new_starts[new_end]].to_owned(),
            });
        }
    };
    for op in &ops {
        if let DiffOp::Equal { .. } = op {
            flush(&mut run);
            continue;
        }
        let (old_range, new_range) = (op.old_range(), op.new_range());
        run = Some(match run {
            Some((old_start, _, new_start, _)) => {
                (old_start, old_range.end, new_start, new_range.end)
            }
            None => (
                old_range.start,
                old_range.end,
                new_range.start,
                new_range.end,
            ),
        });
    }
    flush(&mut run);
    edits
}

/// The lines of `text`, each with its line ending.
fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrq_core::source::apply_edits;

    fn round_trip(old: &str, new: &str) -> Vec<TextEdit> {
        let edits = diff(old, new);
        assert_eq!(apply_edits(old, &edits).unwrap(), new, "{edits:?}");
        edits
    }

    #[test]
    fn diff_edits_the_changed_lines() {
        assert_eq!(round_trip("a\nb\n", "a\nb\n"), []);
        assert_eq!(
            round_trip("a\nb\nc\nd\n", "a\nB\nC\nd\n"),
            [TextEdit {
                range: ByteRange::new(2, 6),
                replacement: "B\nC\n".into(),
            }]
        );
        // Separate runs, insertions, deletions and a missing final newline.
        assert_eq!(round_trip("a\nb\nc\nd\ne", "x\na\nc\nd\ne\n").len(), 3);
        round_trip("", "a\n");
        round_trip("a\n", "");
        round_trip("a\r\nb\r\n", "a\nb\n");
    }
}
