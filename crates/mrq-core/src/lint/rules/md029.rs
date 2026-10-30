//! MD029 `ol-prefix`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{ListKind, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::md005::ItemPrefix;
use super::{edit, expected_actual, js_len, message};

/// Ordered list item prefix.
///
/// The numbers of an ordered list's items must follow the configured style;
/// `one_or_ordered` takes `ordered` if the second item is not `1` or the first is `0`,
/// else `one`. `ordered` counts up from 1 (from 0 if the first item is `0`). The fix
/// renumbers the item, keeping right-aligned numbers aligned (with zeros if the first
/// number has a leading zero).
#[derive(Debug, Clone, Copy, Default)]
pub struct Md029;

/// Options of MD029 `ol-prefix`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md029Options {
    /// List style.
    pub style: OrderedListStyle,
}

/// A numbering style of MD029.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderedListStyle {
    /// `ordered` if the list counts up, else `one`.
    #[default]
    OneOrOrdered,
    /// `1. 1. 1.`
    One,
    /// `1. 2. 3.` (or `0. 1. 2.`)
    Ordered,
    /// `0. 0. 0.`
    Zero,
}

impl OrderedListStyle {
    /// markdownlint's example of a resolved style, for messages.
    const fn example(self) -> &'static str {
        match self {
            Self::One | Self::OneOrOrdered => "1/1/1",
            Self::Ordered => "1/2/3",
            Self::Zero => "0/0/0",
        }
    }
}

impl Rule for Md029 {
    type Options = Md029Options;

    const META: RuleMeta = RuleMeta {
        id: "MD029",
        aliases: &["ol-prefix"],
        tags: &["ol"],
        description: "Ordered list item prefix",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md029Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for list in ast.descendants(ast.root()) {
            let NodeKind::List(payload) = ast.kind(list) else {
                continue;
            };
            if !matches!(payload.kind, ListKind::Ordered { .. }) {
                continue;
            }
            let items: Vec<ItemPrefix> = ast
                .children(list)
                .iter()
                .map(|&item| ItemPrefix::of(doc, item))
                .collect();
            let digits =
                |prefix: &ItemPrefix| ByteRange::new(prefix.marker.start, prefix.marker.end - 1);
            let value = |prefix: &ItemPrefix| {
                let range = digits(prefix);
                &text[range.start..range.end]
            };
            let number = |prefix: &ItemPrefix| value(prefix).parse::<u64>().unwrap_or(0);
            let end_column = |prefix: &ItemPrefix| js_len(&text[prefix.line_start..prefix.end]);
            let Some(first) = items.first() else {
                continue;
            };
            let right_aligned = items
                .iter()
                .all(|prefix| end_column(prefix) == end_column(first));
            let mut expected = 1;
            let mut incrementing = false;
            let mut zero_prefixed = false;
            if let Some(second) = items.get(1) {
                if number(second) != 1 || number(first) == 0 {
                    incrementing = true;
                    if number(first) == 0 {
                        expected = 0;
                    }
                }
                zero_prefixed = value(first).starts_with('0');
            }
            let style = match options.style {
                OrderedListStyle::OneOrOrdered if incrementing => OrderedListStyle::Ordered,
                OrderedListStyle::OneOrOrdered => OrderedListStyle::One,
                style => style,
            };
            match style {
                OrderedListStyle::Zero => expected = 0,
                OrderedListStyle::One => expected = 1,
                _ => {}
            }
            for prefix in &items {
                let actual = number(prefix);
                if actual != expected {
                    let range = digits(prefix);
                    let mut replacement = expected.to_string();
                    if right_aligned {
                        let pad = if zero_prefixed { "0" } else { " " };
                        let width = js_len(value(prefix));
                        while replacement.len() < width {
                            replacement.insert_str(0, pad);
                        }
                    }
                    let fix = Fix::new("Renumber the item", vec![edit(range, replacement)]);
                    let detail = format!(
                        "{}; Style: {}",
                        expected_actual(expected, actual),
                        style.example()
                    );
                    diagnostics.push(
                        Self::META
                            .diagnostic(
                                doc,
                                ByteRange::new(prefix.marker.start, prefix.end),
                                message(&Self::META, Some(&detail), None),
                            )
                            .with_fix(fix),
                    );
                }
                if style == OrderedListStyle::Ordered {
                    expected += 1;
                }
            }
        }
        diagnostics
    }
}
