//! MD039 `no-space-in-links`.

use crate::Document;
use crate::ast::{LinkForm, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, line_of, message};

/// Spaces inside link text.
///
/// Whitespace at the start or end of a link's text (`[ text ](…)`, reference links
/// included, images not) is reported, once per end. The fix deletes it, unless it
/// is (or starts with) a line ending.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md039;

impl Rule for Md039 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD039",
        aliases: &["no-space-in-links"],
        tags: &["whitespace", "links"],
        description: "Spaces inside link text",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let source = doc.source();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::Link(payload) = ast.kind(id) else {
                continue;
            };
            if matches!(payload.form, LinkForm::Autolink | LinkForm::Bare) {
                continue;
            }
            let span = ast.span(id);
            if !text[span.start..].starts_with('[') {
                continue;
            }
            // The label ends at the first `]` after the link text.
            let after_children = ast
                .children(id)
                .last()
                .map_or(span.start + 1, |&last| ast.span(last).end);
            let Some(close) = text[after_children..span.end].find(']') else {
                continue;
            };
            let label = ByteRange::new(span.start + 1, after_children + close);
            let label_text = &text[label.start..label.end];
            let shown: String = text[span.start..=label.end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let lead = label_text.len() - label_text.trim_start().len();
            let trail = label_text.len() - label_text.trim_end().len();
            for (start, space) in [(true, lead), (false, trail)] {
                if space == 0 {
                    continue;
                }
                let (range, edge) = if start {
                    let horizontal =
                        label_text.len() - label_text.trim_start_matches(is_horizontal_space).len();
                    (
                        ByteRange::new(label.start, label.start + horizontal),
                        label.start,
                    )
                } else {
                    let horizontal =
                        label_text.len() - label_text.trim_end_matches(is_horizontal_space).len();
                    (ByteRange::new(label.end - horizontal, label.end), label.end)
                };
                let shown = ellipsify(&shown, start, !start);
                let message = message(&Self::META, None, Some(&shown));
                let diagnostic = if range.is_empty() {
                    // A line ending: markdownlint reports the line after (or before) it.
                    let line = line_of(doc, edge);
                    let line = if start {
                        line + 1
                    } else {
                        line.saturating_sub(1)
                    };
                    let range = source.line_range(line).unwrap_or(range);
                    Self::META.diagnostic(doc, range, message)
                } else {
                    Self::META
                        .diagnostic(doc, range, message)
                        .with_fix(Fix::new("Delete the spaces", vec![edit(range, "")]))
                };
                diagnostics.push(diagnostic);
            }
        }
        diagnostics
    }
}

/// Whitespace but line endings (`[^\S\r\n]`).
fn is_horizontal_space(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\r' | '\n')
}
