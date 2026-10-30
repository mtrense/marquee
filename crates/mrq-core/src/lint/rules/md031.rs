//! MD031 `blanks-around-fences`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{body_line, edit, ellipsify, is_blank_line, message, span_lines};

/// Fenced code blocks should be surrounded by blank lines.
///
/// A fenced code block needs a blank line (or one with only comments or `>`) above
/// its opening fence and below its closing fence, except at the start or end of the
/// document; with `list_items` off, blocks in lists are left alone. The fix inserts a
/// line with the fence line's `>` markers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md031;

/// Options of MD031 `blanks-around-fences`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md031Options {
    /// Include list items.
    pub list_items: bool,
}

impl Default for Md031Options {
    fn default() -> Self {
        Self { list_items: true }
    }
}

impl Rule for Md031 {
    type Options = Md031Options;

    const META: RuleMeta = RuleMeta {
        id: "MD031",
        aliases: &["blanks-around-fences"],
        tags: &["code", "blank_lines"],
        description: "Fenced code blocks should be surrounded by blank lines",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md031Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let source = doc.source();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        // Reports fence line `line`; the fix inserts a line before line `insert_at`.
        let mut report = |line: usize, insert_at: usize| {
            let Some(range) = source.line_range(line) else {
                return;
            };
            let content = &text[range.start..range.end];
            let fix = content.find(['`', '~']).map(|fence| {
                let markers: String = content[..fence]
                    .chars()
                    .map(|c| if c == '>' { '>' } else { ' ' })
                    .collect();
                let at = source
                    .line_range(insert_at)
                    .map_or(text.len(), |range| range.start);
                Fix::new(
                    "Insert a blank line",
                    vec![edit(ByteRange::empty(at), format!("{}\n", markers.trim()))],
                )
            });
            let shown = ellipsify(content.trim(), false, false);
            let diagnostic =
                Self::META.diagnostic(doc, range, message(&Self::META, None, Some(&shown)));
            diagnostics.push(match fix {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            });
        };
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::CodeBlock(code) if code.fence.is_some()) {
                continue;
            }
            if !options.list_items
                && ast
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
            if !is_blank_line(body_line(doc, last + 1)) && !is_blank_line(body_line(doc, last)) {
                report(last, last + 1);
            }
        }
        diagnostics
    }
}
