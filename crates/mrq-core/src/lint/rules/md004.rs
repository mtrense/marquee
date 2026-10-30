//! MD004 `ul-style`.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{ListKind, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, expected_actual, message};

/// Unordered list style.
///
/// Every bullet list item marker must have the configured style; `consistent` takes the
/// first marker's, and `sublist` wants each nesting level to differ from its parent
/// (each level takes the style its first item has). The fix replaces the marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md004;

/// Options of MD004 `ul-style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md004Options {
    /// List style.
    pub style: ListStyle,
}

/// A bullet list style of MD004.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListStyle {
    /// Like the first marker.
    #[default]
    Consistent,
    /// `*`
    Asterisk,
    /// `+`
    Plus,
    /// `-`
    Dash,
    /// Each nesting level differs from its parent.
    Sublist,
}

impl ListStyle {
    fn of(marker: char) -> Self {
        match marker {
            '-' => Self::Dash,
            '+' => Self::Plus,
            _ => Self::Asterisk,
        }
    }

    const fn marker(self) -> char {
        match self {
            Self::Dash => '-',
            Self::Plus => '+',
            _ => '*',
        }
    }

    /// A style other than `self`, for a sublist whose first item repeats its parent's.
    const fn different(self) -> Self {
        match self {
            Self::Dash => Self::Plus,
            Self::Plus => Self::Asterisk,
            _ => Self::Dash,
        }
    }
}

impl fmt::Display for ListStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Consistent => "consistent",
            Self::Asterisk => "asterisk",
            Self::Plus => "plus",
            Self::Dash => "dash",
            Self::Sublist => "sublist",
        })
    }
}

impl Rule for Md004 {
    type Options = Md004Options;

    const META: RuleMeta = RuleMeta {
        id: "MD004",
        aliases: &["ul-style"],
        tags: &["bullet", "ul"],
        description: "Unordered list style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md004Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut expected = options.style;
        // The style of each nesting level, for `sublist`.
        let mut levels: Vec<Option<ListStyle>> = Vec::new();
        let mut diagnostics = Vec::new();
        for list in ast.descendants(ast.root()) {
            let NodeKind::List(payload) = ast.kind(list) else {
                continue;
            };
            if !matches!(payload.kind, ListKind::Bullet { .. }) {
                continue;
            }
            let nesting = ast
                .ancestors(list)
                .filter(|&id| matches!(ast.kind(id), NodeKind::List(_)))
                .count();
            for &item in ast.children(list) {
                let start = ast.span(item).start;
                let Some(marker) = text[start..].chars().next() else {
                    continue;
                };
                let actual = ListStyle::of(marker);
                if options.style == ListStyle::Sublist {
                    if levels.len() <= nesting {
                        levels.resize(nesting + 1, None);
                    }
                    let parent = nesting.checked_sub(1).and_then(|parent| levels[parent]);
                    expected = *levels[nesting].get_or_insert(if parent == Some(actual) {
                        actual.different()
                    } else {
                        actual
                    });
                } else if expected == ListStyle::Consistent {
                    expected = actual;
                }
                if actual == expected {
                    continue;
                }
                let range = ByteRange::new(start, start + marker.len_utf8());
                let fix = Fix::new(
                    format!("Use `{}` as the list marker", expected.marker()),
                    vec![edit(range, expected.marker().to_string())],
                );
                let detail = expected_actual(expected, actual);
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
