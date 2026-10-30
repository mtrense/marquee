//! MD009 `no-trailing-spaces`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, lines, mark_lines, message, span_lines};

/// Trailing spaces.
///
/// Trailing whitespace is reported unless it is exactly `br_spaces` long (a hard
/// break; with `strict` only where it really is one: inside a paragraph, not on its last
/// line, and outside code spans). Code block content and, with
/// `list_item_empty_lines`, the lines of list items are left alone. The fix deletes the
/// whitespace.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md009;

/// Options of MD009 `no-trailing-spaces`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md009Options {
    /// Spaces for line break (values below 2 allow none).
    pub br_spaces: usize,
    /// Include code blocks.
    pub code_blocks: bool,
    /// Allow spaces for empty lines in list items.
    pub list_item_empty_lines: bool,
    /// Include unnecessary breaks.
    pub strict: bool,
}

impl Default for Md009Options {
    fn default() -> Self {
        Self {
            br_spaces: 2,
            code_blocks: false,
            list_item_empty_lines: false,
            strict: false,
        }
    }
}

impl Rule for Md009 {
    type Options = Md009Options;

    const META: RuleMeta = RuleMeta {
        id: "MD009",
        aliases: &["no-trailing-spaces"],
        tags: &["whitespace"],
        description: "Trailing spaces",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md009Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let count = doc.source().line_count();
        let mut code = vec![false; count];
        let mut list_items = vec![false; count];
        // Lines of a paragraph but its last, and of a code span but its last.
        let mut breaks = vec![false; count];
        let mut code_spans = vec![false; count];
        for id in ast.descendants(ast.root()) {
            let span = ast.span(id);
            let (first, last) = span_lines(doc, span);
            match ast.kind(id) {
                NodeKind::CodeBlock(block) if !options.code_blocks => {
                    // The content of a fenced block, every line of an indented one.
                    let (first, last) = match block.fence {
                        Some(_) => (first + 1, last.saturating_sub(1)),
                        None => (first, last),
                    };
                    for line in code.iter_mut().take(last + 1).skip(first) {
                        *line = true;
                    }
                }
                NodeKind::ListItem(_) if options.list_item_empty_lines => {
                    mark_lines(doc, &mut list_items, span);
                }
                NodeKind::Paragraph if options.strict => {
                    for line in breaks.iter_mut().take(last).skip(first) {
                        *line = true;
                    }
                }
                NodeKind::Code(_) if options.strict => {
                    for line in code_spans.iter_mut().take(last).skip(first) {
                        *line = true;
                    }
                }
                _ => {}
            }
        }
        let expected = if options.br_spaces < 2 {
            0
        } else {
            options.br_spaces
        };
        let mut diagnostics = Vec::new();
        for (line, range) in lines(doc) {
            let content = &text[range.start..range.end];
            let trimmed = content.trim_end();
            let trailing = content[trimmed.len()..].chars().count();
            if trailing == 0
                || code[line]
                || list_items[line]
                || (expected == trailing
                    && !(options.strict && (!breaks[line] || code_spans[line])))
            {
                continue;
            }
            let spaces = ByteRange::new(range.start + trimmed.len(), range.end);
            let detail = if expected == 0 {
                format!("Expected: 0; Actual: {trailing}")
            } else {
                format!("Expected: 0 or {expected}; Actual: {trailing}")
            };
            let fix = Fix::new("Delete the trailing spaces", vec![edit(spaces, "")]);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, spaces, message(&Self::META, Some(&detail), None))
                    .with_fix(fix),
            );
        }
        diagnostics
    }
}
