//! MD022 `blanks-around-headings`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{
    body_line, edit, expected_actual, heading_line, is_blank_line, line_containers, line_prefix,
    message, span_lines,
};

/// Headings should be surrounded by blank lines.
///
/// Each heading needs `lines_above` blank lines above and `lines_below` below (fewer at
/// the start or end of the document are fine; `-1` turns a side off). Lines with only
/// comments or `>` count as blank, and front matter right above a heading counts as
/// blank unless `include_front_matter`. The fix inserts the missing lines, with the
/// block quote markers of the neighbouring line.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md022;

/// Options of MD022 `blanks-around-headings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md022Options {
    /// Blank lines above heading.
    pub lines_above: BlankLines,
    /// Blank lines below heading.
    pub lines_below: BlankLines,
    /// Include front matter content.
    pub include_front_matter: bool,
}

impl Default for Md022Options {
    fn default() -> Self {
        Self {
            lines_above: BlankLines::Count(1),
            lines_below: BlankLines::Count(1),
            include_front_matter: false,
        }
    }
}

/// Blank lines for every heading, or per level (index 0 for level 1; missing levels
/// take 1). `-1` turns the check off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum BlankLines {
    /// The same for every level.
    Count(i64),
    /// One count per level.
    PerLevel(Vec<i64>),
}

impl BlankLines {
    fn for_level(&self, level: u8) -> i64 {
        match self {
            Self::Count(count) => *count,
            Self::PerLevel(counts) => counts
                .get(usize::from(level.saturating_sub(1)))
                .copied()
                .unwrap_or(1),
        }
    }
}

impl Rule for Md022 {
    type Options = Md022Options;

    const META: RuleMeta = RuleMeta {
        id: "MD022",
        aliases: &["blanks-around-headings"],
        tags: &["headings", "blank_lines"],
        description: "Headings should be surrounded by blank lines",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md022Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let source = doc.source();
        let containers = line_containers(doc);
        let first_body = super::body_lines(doc).start;
        // markdownlint's line above or below: blank outside the body, except front
        // matter above with `include_front_matter`.
        let line_text = |line: usize| match body_line(doc, line) {
            None if options.include_front_matter && line < first_body => source
                .line_range(line)
                .map(|range| &text[range.start..range.end]),
            other => other,
        };
        let insert = |line: usize, prefix_line: Option<usize>, count: usize| {
            let prefix = prefix_line.map_or_else(String::new, |prefix_line| {
                line_prefix(
                    doc,
                    prefix_line,
                    containers.get(prefix_line).copied().flatten(),
                )
                .quote
            });
            let at = source
                .line_range(line)
                .map_or(text.len(), |range| range.start);
            Fix::new(
                "Insert blank lines",
                vec![edit(
                    ByteRange::empty(at),
                    format!("{prefix}\n").repeat(count),
                )],
            )
        };
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::Heading(heading) = ast.kind(id) else {
                continue;
            };
            let (first, last) = span_lines(doc, ast.span(id));
            let range = heading_line(doc, id);
            let context = source
                .line_range(first)
                .map_or("", |line| text[line.start..line.end].trim());
            let mut report = |expected: i64, actual: i64, side: &str, fix: Fix| {
                let detail = format!("{}; {side}", expected_actual(expected, actual));
                diagnostics.push(
                    Self::META
                        .diagnostic(
                            doc,
                            range,
                            message(&Self::META, Some(&detail), Some(context)),
                        )
                        .with_fix(fix),
                );
            };

            let above = options.lines_above.for_level(heading.level);
            if above >= 0 {
                let actual = (1..=above)
                    .take_while(|&i| {
                        usize::try_from(i)
                            .ok()
                            .and_then(|i| first.checked_sub(i))
                            .is_none_or(|line| is_blank_line(line_text(line)))
                    })
                    .count();
                let actual = i64::try_from(actual).unwrap_or(i64::MAX);
                if actual != above {
                    let missing = usize::try_from(above - actual).unwrap_or(0);
                    report(
                        above,
                        actual,
                        "Above",
                        insert(first, first.checked_sub(1), missing),
                    );
                }
            }

            let below = options.lines_below.for_level(heading.level);
            if below >= 0 {
                let actual = (1..=below)
                    .take_while(|&i| {
                        usize::try_from(i).is_ok_and(|i| is_blank_line(body_line(doc, last + i)))
                    })
                    .count();
                let actual = i64::try_from(actual).unwrap_or(i64::MAX);
                if actual != below {
                    let missing = usize::try_from(below - actual).unwrap_or(0);
                    report(
                        below,
                        actual,
                        "Below",
                        insert(last + 1, Some(last + 1), missing),
                    );
                }
            }
        }
        diagnostics
    }
}
