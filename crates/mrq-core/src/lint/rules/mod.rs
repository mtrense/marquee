//! The built-in rules, one module per rule, plus helpers they share.

mod md012;
mod md047;

pub use md012::{Md012, Md012Options};
pub use md047::Md047;

use crate::Document;
use crate::ast::NodeKind;

use super::Registry;

/// Adds every built-in rule to `registry`.
pub(super) fn register_builtin(registry: &mut Registry) {
    registry.register(Md012);
    registry.register(Md047);
}

/// The line ending the document uses: its first `\r\n` or `\n`, else `\n`.
pub(crate) fn line_ending(doc: &Document) -> &'static str {
    match doc.text().find('\n') {
        Some(newline) if doc.text()[..newline].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

/// For each line of `doc`, whether markdownlint leaves it alone in line-based rules:
/// frontmatter lines and the lines of code blocks (fences included).
pub(crate) fn skipped_lines(doc: &Document) -> Vec<bool> {
    let source = doc.source();
    let mut skipped = vec![false; source.line_count()];
    let line_of = |offset: usize| {
        source
            .offset_to_position(offset, crate::source::PositionEncoding::Utf8)
            .map_or(0, |position| position.line)
    };
    let mut mark = |start: usize, end: usize| {
        let first = line_of(start);
        // A span ending right after a newline does not cover the next line.
        let last = line_of(
            doc.text()
                .floor_char_boundary(end.saturating_sub(1).max(start)),
        );
        for line in skipped.iter_mut().take(last + 1).skip(first) {
            *line = true;
        }
    };
    if let Some(frontmatter) = doc.frontmatter() {
        let span = frontmatter.span();
        mark(span.start, span.end);
    }
    let ast = doc.ast();
    for id in ast.descendants(ast.root()) {
        if let NodeKind::CodeBlock(_) = ast.kind(id) {
            let span = ast.span(id);
            mark(span.start, span.end);
        }
    }
    skipped
}
