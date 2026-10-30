//! MD012 `no-multiple-blanks`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::{ByteRange, TextEdit};

use super::skipped_lines;

/// Multiple consecutive blank lines.
///
/// Every blank line (empty or whitespace only) beyond `maximum` in a row is reported,
/// outside frontmatter and code blocks; the fix deletes it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md012;

/// Options of MD012 `no-multiple-blanks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md012Options {
    /// Consecutive blank lines allowed.
    pub maximum: usize,
}

impl Default for Md012Options {
    fn default() -> Self {
        Self { maximum: 1 }
    }
}

impl Rule for Md012 {
    type Options = Md012Options;

    const META: RuleMeta = RuleMeta {
        id: "MD012",
        aliases: &["no-multiple-blanks"],
        tags: &["whitespace", "blank_lines"],
        description: "Multiple consecutive blank lines",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md012Options) -> Vec<Diagnostic> {
        let source = doc.source();
        let text = doc.text();
        let skipped = skipped_lines(doc);
        let mut diagnostics = Vec::new();
        let mut count = 0;
        for (line, skip) in skipped.into_iter().enumerate() {
            let Some(range) = source.line_range(line) else {
                break;
            };
            if skip || !text[range.start..range.end].trim().is_empty() {
                count = 0;
                continue;
            }
            count += 1;
            if count <= options.maximum {
                continue;
            }
            // The line with its line ending; the last line takes the one before it.
            let deletion = match source.line_range(line + 1) {
                Some(next) => ByteRange::new(range.start, next.start),
                None => ByteRange::new(range.start - line_ending_len(text, range.start), range.end),
            };
            let fix = Fix::new(
                "Delete the blank line",
                vec![TextEdit {
                    range: deletion,
                    replacement: String::new(),
                }],
            );
            let message = format!(
                "{} [Expected: {}; Actual: {count}]",
                Self::META.description,
                options.maximum
            );
            diagnostics.push(Self::META.diagnostic(doc, range, message).with_fix(fix));
        }
        diagnostics
    }
}

/// Length of the line ending that ends right before `offset` (0, 1 or 2 bytes).
fn line_ending_len(text: &str, offset: usize) -> usize {
    let before = &text[..offset];
    if before.ends_with("\r\n") {
        2
    } else {
        usize::from(before.ends_with('\n'))
    }
}
