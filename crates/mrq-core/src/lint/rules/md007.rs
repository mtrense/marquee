//! MD007 `ul-indent`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{ListKind, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::md005::ItemPrefix;
use super::{edit, expected_actual, message};

/// Unordered list indentation.
///
/// A bullet item nested in `n` bullet lists must be indented by `n * indent` spaces
/// (plus `start_indent` with `start_indented`, plus 4 in a footnote), counted after any
/// blockquote prefix on its line. Lists inside an ordered list are not checked, and a
/// blockquote starts the count again. The fix re-indents the marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md007;

/// Options of MD007 `ul-indent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md007Options {
    /// Spaces for indent.
    #[schemars(range(min = 1))]
    pub indent: usize,
    /// Whether to indent the first level of the list.
    pub start_indented: bool,
    /// Spaces for first level indent (when `start_indented` is set); defaults to
    /// `indent`.
    #[schemars(range(min = 1))]
    pub start_indent: Option<usize>,
}

impl Default for Md007Options {
    fn default() -> Self {
        Self {
            indent: 2,
            start_indented: false,
            start_indent: None,
        }
    }
}

impl Rule for Md007 {
    type Options = Md007Options;

    const META: RuleMeta = RuleMeta {
        id: "MD007",
        aliases: &["ul-indent"],
        tags: &["bullet", "ul", "indentation"],
        description: "Unordered list indentation",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md007Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let start_indent = if options.start_indented {
            options.start_indent.unwrap_or(options.indent)
        } else {
            0
        };
        let mut diagnostics = Vec::new();
        for list in ast.descendants(ast.root()) {
            let NodeKind::List(payload) = ast.kind(list) else {
                continue;
            };
            if !matches!(payload.kind, ListKind::Bullet { .. }) {
                continue;
            }
            let mut nesting = Some(0);
            for id in ast.ancestors(list) {
                match ast.kind(id) {
                    NodeKind::List(parent) if matches!(parent.kind, ListKind::Bullet { .. }) => {
                        nesting = nesting.map(|n| n + 1);
                    }
                    NodeKind::List(_) => {
                        nesting = None;
                        break;
                    }
                    NodeKind::BlockQuote | NodeKind::Callout(_) => break,
                    _ => {}
                }
            }
            let Some(nesting) = nesting else {
                continue;
            };
            let footnote = ast
                .ancestors(list)
                .any(|id| matches!(ast.kind(id), NodeKind::FootnoteDefinition(_)));
            let quotes = ast
                .ancestors(list)
                .filter(|&id| matches!(ast.kind(id), NodeKind::BlockQuote | NodeKind::Callout(_)))
                .count();
            let expected = if footnote { 4 } else { 0 } + start_indent + nesting * options.indent;
            for &item in ast.children(list) {
                let prefix = ItemPrefix::of(doc, item);
                let quote_end = prefix.line_start
                    + quote_prefix_len(&text[prefix.line_start..prefix.marker.start], quotes);
                let actual = prefix.marker.start - quote_end;
                if actual == expected {
                    continue;
                }
                let fix = Fix::new(
                    "Indent the list item",
                    vec![edit(
                        ByteRange::new(quote_end, quote_end + actual.saturating_sub(expected)),
                        " ".repeat(expected.saturating_sub(actual)),
                    )],
                );
                let detail = expected_actual(expected, actual);
                diagnostics.push(
                    Self::META
                        .diagnostic(
                            doc,
                            ByteRange::new(prefix.line_start, prefix.end),
                            message(&Self::META, Some(&detail), None),
                        )
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}

/// Length of the first `quotes` blockquote markers (`>` plus one optional space, after
/// up to three spaces) at the start of `line`; 0 if the line has fewer.
fn quote_prefix_len(line: &str, quotes: usize) -> usize {
    let mut end = 0;
    for _ in 0..quotes {
        let rest = &line[end..];
        let indent = rest.len() - rest.trim_start_matches(' ').len();
        if indent > 3 || !rest[indent..].starts_with('>') {
            return end;
        }
        end += indent + 1;
        if line[end..].starts_with([' ', '\t']) {
            end += 1;
        }
    }
    end
}
