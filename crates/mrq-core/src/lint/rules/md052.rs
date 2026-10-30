//! MD052 `reference-links-images`.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{LinkForm, NodeKind};
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{CHARACTER_REFERENCE, Definitions, line_of, message, normalize_label};

/// Reference links and images should use a label that is defined.
///
/// A full (`[text][label]`) or collapsed (`[label][]`) reference link or image whose
/// label (normalized: lowercase, whitespace collapsed) has no definition is reported,
/// shortcuts (`[label]`) too with `shortcut_syntax`; labels in `ignored_labels` are
/// fine. The parser leaves such references as text, so they are found in the source of
/// text nodes. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md052;

/// Options of MD052 `reference-links-images`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md052Options {
    /// Ignored link labels.
    pub ignored_labels: Vec<String>,
    /// Include shortcut syntax.
    pub shortcut_syntax: bool,
}

impl Default for Md052Options {
    fn default() -> Self {
        Self {
            ignored_labels: vec!["x".to_owned()],
            shortcut_syntax: false,
        }
    }
}

impl Rule for Md052 {
    type Options = Md052Options;

    const META: RuleMeta = RuleMeta {
        id: "MD052",
        aliases: &["reference-links-images"],
        tags: &["images", "links"],
        description: "Reference links and images should use a label that is defined",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md052Options) -> Vec<Diagnostic> {
        let text = doc.text();
        let definitions = Definitions::of(doc);
        let ignored: HashSet<&str> = options.ignored_labels.iter().map(String::as_str).collect();
        let mut diagnostics = Vec::new();
        for reference in undefined_references(doc) {
            if reference.shortcut && !options.shortcut_syntax
                || definitions.get(&reference.label).is_some()
                || ignored.contains(reference.label.as_str())
            {
                continue;
            }
            // The context and range end with the reference's first line.
            let line_end = doc
                .source()
                .line_range(line_of(doc, reference.range.start))
                .map_or(reference.range.end, |line| line.end);
            let range = ByteRange::new(reference.range.start, reference.range.end.min(line_end));
            let detail = format!(
                "Missing link or image reference definition: \"{}\"",
                reference.label
            );
            diagnostics.push(Self::META.diagnostic(
                doc,
                range,
                message(
                    &Self::META,
                    Some(&detail),
                    Some(&text[range.start..range.end]),
                ),
            ));
        }
        diagnostics
    }
}

/// Reference syntax the parser left as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UndefinedReference {
    /// The whole reference, `!` of an image included.
    pub range: ByteRange,
    /// The label, normalized.
    pub label: String,
    /// Whether it is a shortcut, `[label]`.
    pub shortcut: bool,
}

/// Every full, collapsed or shortcut reference (link or image) in text: brackets that
/// the parser did not turn into a link because their label has no definition.
pub(super) fn undefined_references(doc: &Document) -> Vec<UndefinedReference> {
    let ast = doc.ast();
    let text = doc.text();
    let bytes = text.as_bytes();
    // Text nodes outside autolinks, in document order.
    let text_nodes: Vec<_> = ast
        .descendants(ast.root())
        .filter(|&id| {
            matches!(ast.kind(id), NodeKind::Text(_))
                && !ast.ancestors(id).any(|ancestor| {
                    matches!(ast.kind(ancestor), NodeKind::Link(link)
                        if matches!(link.form, LinkForm::Autolink | LinkForm::Bare))
                })
        })
        .collect();
    let texts: Vec<(usize, usize)> = text_nodes
        .iter()
        .map(|&id| (ast.span(id).start, ast.span(id).end))
        .collect();
    let in_text = |at: usize, block: ByteRange| {
        let after = texts.partition_point(|&(start, _)| start <= at);
        at < block.end && after > 0 && at < texts[after - 1].1
    };
    let mut found = Vec::new();
    // Where the last reference ended: brackets before it are part of it.
    let mut consumed = 0;
    for id in text_nodes {
        let span = ast.span(id);
        let block = ast
            .ancestors(id)
            .find(|&ancestor| ast.kind(ancestor).is_block())
            .map_or(ByteRange::new(0, text.len()), |block| {
                let span = ast.span(block);
                ByteRange::new(span.start, span.end)
            });
        for open in span.start.max(consumed)..span.end {
            if open < consumed || bytes[open] != b'[' || escaped(bytes, open) {
                continue;
            }
            let Some(close) = closing(bytes, open + 1, block.end, true) else {
                continue;
            };
            if !in_text(close, block) {
                continue;
            }
            let start = if open > 0 && bytes[open - 1] == b'!' && !escaped(bytes, open - 1) {
                open - 1
            } else {
                open
            };
            let first = &text[open + 1..close];
            let second = (bytes.get(close + 1) == Some(&b'['))
                .then(|| closing(bytes, close + 2, block.end, false))
                .flatten()
                .filter(|&end| in_text(end, block));
            // Labels cannot hold brackets; the text of a full reference can.
            let (label, end, shortcut) = match second {
                Some(end) if end == close + 2 => (first, end + 1, false),
                Some(end) => (&text[close + 2..end], end + 1, false),
                None => (first, close + 1, true),
            };
            let label_is_text = second.is_none_or(|end| end == close + 2);
            if label.trim().is_empty() || label_is_text && first.contains('[') {
                continue;
            }
            found.push(UndefinedReference {
                range: ByteRange::new(start, end),
                label: normalize_label(&label_data(label)),
                shortcut,
            });
            consumed = end;
        }
    }
    found
}

/// The parts of a label markdownlint keeps: its text without backslash escapes,
/// character references and `*`, `_` or `~` (which micromark takes for emphasis).
fn label_data(label: &str) -> String {
    let mut data = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek().is_some_and(char::is_ascii_punctuation) => {
                chars.next();
            }
            '*' | '_' | '~' => {}
            c => data.push(c),
        }
    }
    CHARACTER_REFERENCE.replace_all(&data, "").into_owned()
}

/// The unescaped `]` closing the brackets opened before `from`, before `end`. With
/// `nested`, balanced brackets may come first; otherwise any `[` ends the search.
fn closing(bytes: &[u8], from: usize, end: usize, nested: bool) -> Option<usize> {
    let mut depth = 0usize;
    for i in from..end {
        if escaped(bytes, i) {
            continue;
        }
        match bytes[i] {
            b'[' if nested => depth += 1,
            b'[' => return None,
            b']' if depth == 0 => return Some(i),
            b']' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Whether the byte at `i` is backslash-escaped (an odd number of `\` before it).
fn escaped(bytes: &[u8], i: usize) -> bool {
    bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1
}
