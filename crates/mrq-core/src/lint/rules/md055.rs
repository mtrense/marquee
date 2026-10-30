//! MD055 `table-pipe-style`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{expected_actual, message, table_lines};

/// Table pipe style.
///
/// Every table row (the delimiter row included) must have leading and trailing pipes
/// as `style` says; `consistent` takes the first row's style, `per_table` each table's
/// first row's. A missing or unexpected pipe is reported where the row starts or ends.
/// No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md055;

/// Options of MD055 `table-pipe-style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md055Options {
    /// Table pipe style.
    pub style: PipeStyle,
}

/// A table pipe style of MD055.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PipeStyle {
    /// Like the first row.
    #[default]
    Consistent,
    /// `| a | b`
    LeadingOnly,
    /// `a | b |`
    TrailingOnly,
    /// `| a | b |`
    LeadingAndTrailing,
    /// `a | b`
    NoLeadingOrTrailing,
    /// Like the first row of each table.
    PerTable,
}

impl PipeStyle {
    const fn of(leading: bool, trailing: bool) -> Self {
        match (leading, trailing) {
            (true, true) => Self::LeadingAndTrailing,
            (true, false) => Self::LeadingOnly,
            (false, true) => Self::TrailingOnly,
            (false, false) => Self::NoLeadingOrTrailing,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Consistent => "consistent",
            Self::LeadingOnly => "leading_only",
            Self::TrailingOnly => "trailing_only",
            Self::LeadingAndTrailing => "leading_and_trailing",
            Self::NoLeadingOrTrailing => "no_leading_or_trailing",
            Self::PerTable => "per_table",
        }
    }
}

impl Rule for Md055 {
    type Options = Md055Options;

    const META: RuleMeta = RuleMeta {
        id: "MD055",
        aliases: &["table-pipe-style"],
        tags: &["table"],
        description: "Table pipe style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md055Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut expected = options.style;
        // markdownlint derives these from the configured style, `consistent` and
        // `per_table` counting as both pipes until a row sets them.
        let mut leading = !matches!(
            expected,
            PipeStyle::NoLeadingOrTrailing | PipeStyle::TrailingOnly
        );
        let mut trailing = !matches!(
            expected,
            PipeStyle::NoLeadingOrTrailing | PipeStyle::LeadingOnly
        );
        let mut diagnostics = Vec::new();
        for table in ast.descendants(ast.root()) {
            if !matches!(ast.kind(table), NodeKind::Table(_)) {
                continue;
            }
            if options.style == PipeStyle::PerTable {
                expected = PipeStyle::Consistent;
            }
            for row in table_lines(doc, table) {
                let actual_leading = row.leading_pipe();
                let actual_trailing = row.trailing_pipe();
                let actual = PipeStyle::of(actual_leading, actual_trailing);
                if expected == PipeStyle::Consistent {
                    expected = actual;
                    leading = actual_leading;
                    trailing = actual_trailing;
                }
                // Reports the row's first or last character.
                let mut report = |range: ByteRange, wanted: bool, side: &str| {
                    let which = if wanted { "Missing" } else { "Unexpected" };
                    let detail = format!(
                        "{}; {which} {side} pipe",
                        expected_actual(expected.as_str(), actual.as_str())
                    );
                    diagnostics.push(Self::META.diagnostic(
                        doc,
                        range,
                        message(&Self::META, Some(&detail), None),
                    ));
                };
                let content = &text[row.content.start..row.content.end];
                let first = content.chars().next().map_or(0, char::len_utf8);
                let last = content.chars().next_back().map_or(0, char::len_utf8);
                if actual_leading != leading {
                    let start = row.content.start;
                    report(ByteRange::new(start, start + first), leading, "leading");
                }
                if actual_trailing != trailing {
                    let end = row.content.end;
                    report(ByteRange::new(end - last, end), trailing, "trailing");
                }
            }
        }
        diagnostics
    }
}
