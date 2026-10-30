//! MD028 `no-blanks-blockquote`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{NoOptions, Rule, RuleMeta};

use super::span_lines;

/// Blank line inside blockquote.
///
/// Blank lines between two block quotes that follow each other directly (nothing but
/// blank lines between) are reported, one finding per line: they may look like one
/// quote but are two. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md028;

impl Rule for Md028 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD028",
        aliases: &["no-blanks-blockquote"],
        tags: &["blockquote", "whitespace"],
        description: "Blank line inside blockquote",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let source = doc.source();
        let text = doc.text();
        let is_quote = |id| matches!(ast.kind(id), NodeKind::BlockQuote | NodeKind::Callout(_));
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let Some(next) = ast.next_sibling(id) else {
                continue;
            };
            if !is_quote(id) || !is_quote(next) {
                continue;
            }
            let (_, end) = span_lines(doc, ast.span(id));
            let (start, _) = span_lines(doc, ast.span(next));
            let between: Vec<_> = (end + 1..start)
                .filter_map(|line| source.line_range(line))
                .collect();
            // Anything else between (like a reference definition) separates the quotes.
            if between
                .iter()
                .any(|range| !text[range.start..range.end].trim().is_empty())
            {
                continue;
            }
            for range in between {
                diagnostics.push(Self::META.diagnostic(doc, range, Self::META.description));
            }
        }
        diagnostics
    }
}
