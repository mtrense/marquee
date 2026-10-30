//! MD005 `list-indent`.

use crate::Document;
use crate::ast::{ListKind, NodeId, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, expected_actual, js_len, line_of, message};

/// Inconsistent indentation for list items at the same level.
///
/// The items of a list must start in the same column as the first. Ordered lists may
/// instead right-align their markers (`9.` under ` 10.`), which is accepted once an item
/// is out of line with the first but ends its marker in the same column. Only ordered
/// items get a fix (markdownlint leaves bullets to MD007), which moves the marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md005;

/// Where a list item's marker sits on its line.
pub(crate) struct ItemPrefix {
    /// Start of the item's line.
    pub line_start: usize,
    /// Characters (UTF-16 units) before the marker on the line, container prefixes
    /// included.
    pub column: usize,
    /// The marker (`-`, `10.`).
    pub marker: ByteRange,
    /// End of the whitespace after the marker, where markdownlint's `listItemPrefix`
    /// ends.
    pub end: usize,
}

impl ItemPrefix {
    pub(crate) fn of(doc: &Document, item: NodeId) -> Self {
        let text = doc.text();
        let start = doc.ast().span(item).start;
        let line_start = doc
            .source()
            .line_range(line_of(doc, start))
            .map_or(start, |line| line.start);
        let rest = &text[start..];
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let marker = ByteRange::new(start, start + digits + 1);
        let after = &text[marker.end..];
        let line_rest = &after[..after.find(['\r', '\n']).unwrap_or(after.len())];
        let space = line_rest.len() - line_rest.trim_start_matches([' ', '\t']).len();
        let space = match space {
            _ if space == line_rest.len() => 0,
            5.. => 1,
            space => space,
        };
        Self {
            line_start,
            column: js_len(&text[line_start..start]),
            marker,
            end: marker.end + space,
        }
    }
}

impl Rule for Md005 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD005",
        aliases: &["list-indent"],
        tags: &["bullet", "ul", "indentation"],
        description: "Inconsistent indentation for list items at the same level",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let mut diagnostics = Vec::new();
        for list in ast.descendants(ast.root()) {
            let NodeKind::List(payload) = ast.kind(list) else {
                continue;
            };
            let ordered = matches!(payload.kind, ListKind::Ordered { .. });
            let items: Vec<ItemPrefix> = ast
                .children(list)
                .iter()
                .map(|&item| ItemPrefix::of(doc, item))
                .collect();
            let Some(expected_indent) = items.first().map(|item| item.column) else {
                continue;
            };
            let mut expected_end = None;
            let mut end_matching = false;
            for item in &items {
                let range = ByteRange::new(item.line_start, item.end);
                let actual_indent = item.column;
                if !ordered {
                    if actual_indent != expected_indent {
                        let detail = expected_actual(expected_indent, actual_indent);
                        diagnostics.push(Self::META.diagnostic(
                            doc,
                            range,
                            message(&Self::META, Some(&detail), None),
                        ));
                    }
                    continue;
                }
                let marker_len = item.marker.len();
                let actual_end = actual_indent + marker_len;
                let expected_end = *expected_end.get_or_insert(actual_end);
                if actual_indent == expected_indent && !end_matching {
                    continue;
                }
                if expected_end == actual_end {
                    end_matching = true;
                    continue;
                }
                let (detail, expected, actual) = if end_matching {
                    (
                        format!("Expected: ({expected_end}); Actual: ({actual_end})"),
                        expected_end - marker_len,
                        actual_end - marker_len,
                    )
                } else {
                    (
                        expected_actual(expected_indent, actual_indent),
                        expected_indent,
                        actual_indent,
                    )
                };
                let at = item.line_start + actual.min(expected);
                let replace = ByteRange::new(at, at + actual.saturating_sub(expected));
                let fix = Fix::new(
                    "Align the list marker",
                    vec![edit(replace, " ".repeat(expected.saturating_sub(actual)))],
                );
                diagnostics.push(
                    Self::META
                        .diagnostic(doc, range, message(&Self::META, Some(&detail), None))
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}
