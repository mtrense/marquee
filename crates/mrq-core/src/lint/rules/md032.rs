//! MD032 `blanks-around-lists`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{
    body_line, edit, ellipsify, is_blank_line, line_containers, line_prefix, message, span_lines,
};

/// Lists should be surrounded by blank lines.
///
/// A list (outside other lists) needs a blank line (or one with only comments or `>`)
/// above its first line and below its last, except at the start or end of the
/// document. The fix inserts a line with the list line's `>` markers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md032;

impl Rule for Md032 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD032",
        aliases: &["blanks-around-lists"],
        tags: &["bullet", "ul", "ol", "blank_lines"],
        description: "Lists should be surrounded by blank lines",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let source = doc.source();
        let text = doc.text();
        let containers = line_containers(doc);
        let mut diagnostics = Vec::new();
        // Reports list line `line`; the fix inserts a line before line `insert_at`.
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
            let shown = ellipsify(text[range.start..range.end].trim(), false, false);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, range, message(&Self::META, None, Some(&shown)))
                    .with_fix(fix),
            );
        };
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::List(_))
                || ast
                    .ancestors(id)
                    .any(|id| matches!(ast.kind(id), NodeKind::List(_)))
            {
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
