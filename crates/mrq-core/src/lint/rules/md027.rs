//! MD027 `no-multiple-space-blockquote`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{Ast, NodeId, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, line_prefix, message, span_lines};

/// Multiple spaces after blockquote symbol.
///
/// Whitespace after a `>` marker and its space is reported where micromark sees it as
/// indentation: on the first line of a block, on paragraph lines, and in fenced code
/// up to the fence's indentation (not in HTML blocks, indented code or later table
/// rows). With `list_items` off, lines in or starting list items are left alone. The
/// fix deletes the whitespace.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md027;

/// Options of MD027 `no-multiple-space-blockquote`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md027Options {
    /// Include list items.
    pub list_items: bool,
}

impl Default for Md027Options {
    fn default() -> Self {
        Self { list_items: true }
    }
}

impl Rule for Md027 {
    type Options = Md027Options;

    const META: RuleMeta = RuleMeta {
        id: "MD027",
        aliases: &["no-multiple-space-blockquote"],
        tags: &["blockquote", "whitespace", "indentation"],
        description: "Multiple spaces after blockquote symbol",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md027Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        // The indentation found per line.
        let mut found = BTreeMap::new();
        for id in ast.descendants(ast.root()) {
            let quoted = ast
                .ancestors(id)
                .any(|id| matches!(ast.kind(id), NodeKind::BlockQuote | NodeKind::Callout(_)));
            if !ast.kind(id).is_block()
                || !quoted
                || (!options.list_items
                    && ast
                        .ancestors(id)
                        .any(|id| matches!(ast.kind(id), NodeKind::ListItem(_))))
            {
                continue;
            }
            let (first, last) = span_lines(doc, ast.span(id));
            // Fenced code content may keep the opening fence's indentation.
            let fence_indent = if matches!(ast.kind(id), NodeKind::CodeBlock(_)) {
                ast.span(id)
                    .start
                    .saturating_sub(line_prefix(doc, first, ast.parent(id)).end)
            } else {
                0
            };
            for (line, limit) in checked_lines(ast, id, first, last, fence_indent) {
                let prefix = line_prefix(doc, line, ast.parent(id));
                let Some(extra) = prefix.extra else {
                    continue;
                };
                let end = limit.map_or(extra.end, |limit| extra.end.min(extra.start + limit));
                if end > extra.start {
                    found
                        .entry(line)
                        .or_insert(ByteRange::new(extra.start, end));
                }
            }
        }
        let mut diagnostics = Vec::new();
        for (line, range) in found {
            let content = doc
                .source()
                .line_range(line)
                .map_or("", |line| &text[line.start..line.end]);
            let fix = Fix::new("Delete the extra whitespace", vec![edit(range, "")]);
            let shown = ellipsify(content, false, false);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, range, message(&Self::META, None, Some(&shown)))
                    .with_fix(fix),
            );
        }
        diagnostics
    }
}

/// The lines of leaf block `id` (spanning `first..=last`) where micromark tokenizes
/// indentation after a `>` as such, each with a limit on its length (`None` for none).
fn checked_lines(
    ast: &Ast,
    id: NodeId,
    first: usize,
    last: usize,
    fence_indent: usize,
) -> Vec<(usize, Option<usize>)> {
    match ast.kind(id) {
        NodeKind::Paragraph => (first..=last).map(|line| (line, None)).collect(),
        NodeKind::Heading(_) | NodeKind::ThematicBreak | NodeKind::Table(_) => {
            vec![(first, None)]
        }
        // Indentation of fenced code content beyond the opening fence's is code; the
        // closing fence is not checked.
        NodeKind::CodeBlock(code) if code.fence.is_some() => {
            let mut lines = vec![(first, None)];
            lines.extend((first + 1..last).map(|line| (line, Some(fence_indent))));
            lines
        }
        _ => Vec::new(),
    }
}
