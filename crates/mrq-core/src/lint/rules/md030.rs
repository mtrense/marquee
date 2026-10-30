//! MD030 `list-marker-space`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{ListKind, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::md005::ItemPrefix;
use super::{edit, expected_actual, js_len, message, span_lines};

/// Spaces after list markers.
///
/// The whitespace after each item's marker must be `ul_single`/`ol_single` long in
/// lists whose items are all one line (one line per item, no blank lines), else
/// `ul_multi`/`ol_multi`. Empty items and items starting with indented code (one space
/// counts) are fine. The fix sets the whitespace to the expected spaces.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md030;

/// Options of MD030 `list-marker-space`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md030Options {
    /// Spaces for single-line unordered list items.
    pub ul_single: usize,
    /// Spaces for single-line ordered list items.
    pub ol_single: usize,
    /// Spaces for multi-line unordered list items.
    pub ul_multi: usize,
    /// Spaces for multi-line ordered list items.
    pub ol_multi: usize,
}

impl Default for Md030Options {
    fn default() -> Self {
        Self {
            ul_single: 1,
            ol_single: 1,
            ul_multi: 1,
            ol_multi: 1,
        }
    }
}

impl Rule for Md030 {
    type Options = Md030Options;

    const META: RuleMeta = RuleMeta {
        id: "MD030",
        aliases: &["list-marker-space"],
        tags: &["ol", "ul", "whitespace"],
        description: "Spaces after list markers",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md030Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for list in ast.descendants(ast.root()) {
            let NodeKind::List(payload) = ast.kind(list) else {
                continue;
            };
            let items = ast.children(list);
            let (first, last) = span_lines(doc, ast.span(list));
            let single = last - first + 1 == items.len();
            let expected = match (payload.kind, single) {
                (ListKind::Ordered { .. }, true) => options.ol_single,
                (ListKind::Ordered { .. }, false) => options.ol_multi,
                (ListKind::Bullet { .. }, true) => options.ul_single,
                (ListKind::Bullet { .. }, false) => options.ul_multi,
            };
            for &item in items {
                let prefix = ItemPrefix::of(doc, item);
                let space = ByteRange::new(prefix.marker.end, prefix.end);
                let actual = js_len(&text[space.start..space.end]);
                if actual == 0 || actual == expected {
                    continue;
                }
                let fix = Fix::new(
                    "Set the spaces after the marker",
                    vec![edit(space, " ".repeat(expected))],
                );
                diagnostics.push(
                    Self::META
                        .diagnostic(
                            doc,
                            ByteRange::new(prefix.marker.start, prefix.end),
                            message(&Self::META, Some(&expected_actual(expected, actual)), None),
                        )
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}
