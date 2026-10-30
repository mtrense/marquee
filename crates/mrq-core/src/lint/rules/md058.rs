//! MD058 `blanks-around-tables`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{
    body_line, cleared_lines, edit, ellipsify, is_blank_line, line_containers, line_prefix,
    message, span_lines,
};

/// Tables should be surrounded by blank lines.
///
/// A table needs a blank line (or one with only comments or `>`) above its first line
/// and below its last, except at the start or end of the document. The fix inserts a
/// line with the table line's `>` markers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md058;

impl Rule for Md058 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD058",
        aliases: &["blanks-around-tables"],
        tags: &["table"],
        description: "Tables should be surrounded by blank lines",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let source = doc.source();
        let text = doc.text();
        let containers = line_containers(doc);
        let lines = cleared_lines(doc);
        let mut diagnostics = Vec::new();
        // Reports table line `line`; the fix inserts a line before line `insert_at`.
        let mut report = |line: usize, insert_at: usize| {
            let Some(range) = source.line_range(line) else {
                return;
            };
            let quote = line_prefix(doc, line, containers.get(line).copied().flatten()).quote;
            let at = source
                .line_range(insert_at)
                .map_or(text.len(), |range| range.start);
            let fix = Fix::new(
                "Insert a blank line",
                vec![edit(ByteRange::empty(at), format!("{quote}\n"))],
            );
            let shown = ellipsify(lines.get(line).map_or("", |line| line.trim()), false, false);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, range, message(&Self::META, None, Some(&shown)))
                    .with_fix(fix),
            );
        };
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Table(_)) {
                continue;
            }
            let (first, last) = span_lines(doc, ast.span(id));
            if !first
                .checked_sub(1)
                .is_none_or(|above| is_blank_line(body_line(doc, above)))
            {
                report(first, first);
            }
            if !is_blank_line(body_line(doc, last + 1)) {
                report(last, last + 1);
            }
        }
        diagnostics
    }
}
