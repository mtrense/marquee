//! MD037 `no-space-in-emphasis`.

use std::collections::BTreeMap;

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, line_of, message};

/// Spaces inside emphasis markers.
///
/// Like markdownlint, emphasis markers (`*`, `**`, `***` and the `_` ones) that did
/// not become emphasis are paired up in order per marker within each paragraph, heading
/// and table cell (not inside links or other inlines); whitespace after the opening
/// marker of a pair (before text) or before its closing marker (after text), on the
/// marker's line, is reported. The fix deletes it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md037;

impl Rule for Md037 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD037",
        aliases: &["no-space-in-emphasis"],
        tags: &["whitespace", "emphasis"],
        description: "Spaces inside emphasis markers",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let source = doc.source();
        let line_range = |offset: usize| {
            source
                .line_range(line_of(doc, offset))
                .unwrap_or(ByteRange::empty(offset))
        };
        let mut diagnostics = Vec::new();
        let mut report = |range: ByteRange, context: String| {
            let fix = Fix::new("Delete the spaces", vec![edit(range, "")]);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, range, message(&Self::META, None, Some(&context)))
                    .with_fix(fix),
            );
        };
        for parent in ast.descendants(ast.root()) {
            // Like markdownlint, only text right in a paragraph, heading or table cell.
            if !matches!(
                ast.kind(parent),
                NodeKind::Paragraph | NodeKind::Heading(_) | NodeKind::TableCell
            ) {
                continue;
            }
            // The leftover marker runs of the parent's text children, per marker.
            let mut markers: BTreeMap<&str, Vec<ByteRange>> = BTreeMap::new();
            for &child in ast.children(parent) {
                if matches!(ast.kind(child), NodeKind::Text(_)) {
                    let span = ast.span(child);
                    for run in marker_runs(text, ByteRange::new(span.start, span.end)) {
                        markers
                            .entry(&text[run.start..run.end])
                            .or_default()
                            .push(run);
                    }
                }
            }
            for (marker, runs) in markers {
                for &[open, close] in runs.as_chunks::<2>().0 {
                    let after = &text[open.end..line_range(open.end).end];
                    let space = after.len() - after.trim_start().len();
                    if let Some(next) = after[space..].chars().next().filter(|_| space > 0) {
                        report(
                            ByteRange::new(open.end, open.end + space),
                            format!("{marker}{}{next}", &after[..space]),
                        );
                    }
                    let before = &text[line_range(close.start).start..close.start];
                    let trimmed = before.trim_end();
                    let space = before.len() - trimmed.len();
                    if let Some(previous) = trimmed.chars().last().filter(|_| space > 0) {
                        report(
                            ByteRange::new(close.start - space, close.start),
                            format!("{previous}{}{marker}", &before[trimmed.len()..]),
                        );
                    }
                }
            }
        }
        diagnostics
    }
}

/// The runs of one to three `*` or `_` in the source `range` of a text node, skipping
/// backslash escapes: emphasis markers that did not pair up.
fn marker_runs(text: &str, range: ByteRange) -> Vec<ByteRange> {
    let source = &text[range.start..range.end];
    let bytes = source.as_bytes();
    let mut runs = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1).is_some_and(u8::is_ascii_punctuation) => index += 2,
            marker @ (b'*' | b'_') => {
                let start = index;
                while bytes.get(index) == Some(&marker) {
                    index += 1;
                }
                if index - start <= 3 {
                    runs.push(ByteRange::new(range.start + start, range.start + index));
                }
            }
            _ => index += 1,
        }
    }
    runs
}
