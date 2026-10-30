//! Footnote and reference link syntax that the parser left as text: [`unresolved`].
//!
//! A footnote reference or reference link whose label has no definition is not a link;
//! the parser keeps its brackets as plain text. The checks find them by scanning the
//! source of text nodes.

use crate::Document;
use crate::ast::{NodeKind, Span};

/// Bracket syntax in text that looks like a footnote reference or reference link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unresolved {
    /// `[^label]`: the whole reference.
    Footnote { label: String, span: Span },
    /// `[text][label]` (span: `[label]`) or `[label][]` (span: all of it). Shortcut
    /// references `[label]` are left out, as brackets in prose are common.
    Reference { label: String, span: Span },
}

/// Every unresolved footnote reference (only if `footnotes`) and full or collapsed
/// reference link in the text of `document`, in document order.
pub(super) fn unresolved(document: &Document, footnotes: bool) -> Vec<Unresolved> {
    let ast = document.ast();
    let text = document.text();
    let mut found = Vec::new();
    for node in ast.descendants(ast.root()) {
        if !matches!(ast.kind(node), NodeKind::Text(_)) {
            continue;
        }
        let block_start = ast
            .ancestors(node)
            .find(|&a| ast.kind(a).is_block())
            .map_or(0, |block| ast.span(block).start);
        scan(text, ast.span(node), block_start, footnotes, &mut found);
    }
    found
}

/// Scans `text[span]`; `block_start` bounds the search for the `[` that opens a full
/// reference's link text.
fn scan(text: &str, span: Span, block_start: usize, footnotes: bool, out: &mut Vec<Unresolved>) {
    let bytes = text.as_bytes();
    let mut i = span.start;
    while i < span.end {
        if escaped(bytes, i) {
            i += 1;
            continue;
        }
        match bytes[i] {
            b'[' => {
                let Some(close) = closing(bytes, i + 1, span.end) else {
                    i += 1;
                    continue;
                };
                let inner = &text[i + 1..close];
                if footnotes && inner.len() > 1 && inner.starts_with('^') {
                    out.push(Unresolved::Footnote {
                        label: inner[1..].to_owned(),
                        span: Span::new(i, close + 1),
                    });
                    i = close + 1;
                } else if !inner.trim().is_empty()
                    && text.get(close + 1..close + 3) == Some("[]")
                    && close + 3 <= span.end
                {
                    out.push(Unresolved::Reference {
                        label: inner.to_owned(),
                        span: Span::new(i, close + 3),
                    });
                    i = close + 3;
                } else {
                    i += 1;
                }
            }
            b']' if bytes.get(i + 1) == Some(&b'[') => {
                let label = closing(bytes, i + 2, span.end)
                    .filter(|&close| !text[i + 2..close].trim().is_empty())
                    .filter(|_| opening(bytes, block_start, i).is_some());
                match label {
                    Some(close) => {
                        out.push(Unresolved::Reference {
                            label: text[i + 2..close].to_owned(),
                            span: Span::new(i + 1, close + 1),
                        });
                        i = close + 1;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
}

/// The `]` closing a label that starts at `from`, before `end`: none if a `[` or line
/// break comes first.
fn closing(bytes: &[u8], from: usize, end: usize) -> Option<usize> {
    (from..end)
        .filter(|&i| !escaped(bytes, i))
        .find(|&i| matches!(bytes[i], b'[' | b']' | b'\n' | b'\r'))
        .filter(|&i| bytes[i] == b']')
}

/// The unescaped `[` matching the `]` at `close`, searching back to `start`.
fn opening(bytes: &[u8], start: usize, close: usize) -> Option<usize> {
    let mut depth = 0usize;
    for i in (start..close).rev() {
        if escaped(bytes, i) {
            continue;
        }
        match bytes[i] {
            b']' => depth += 1,
            b'[' if depth == 0 => return Some(i),
            b'[' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Whether the byte at `i` is backslash-escaped (an odd number of `\` before it).
fn escaped(bytes: &[u8], i: usize) -> bool {
    bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<(&'static str, String, String)> {
        let doc = Document::parse(text);
        unresolved(&doc, true)
            .into_iter()
            .map(|found| match found {
                Unresolved::Footnote { label, span } => {
                    ("footnote", label, text[span.start..span.end].to_owned())
                }
                Unresolved::Reference { label, span } => {
                    ("reference", label, text[span.start..span.end].to_owned())
                }
            })
            .collect()
    }

    fn item(kind: &'static str, label: &str, source: &str) -> (&'static str, String, String) {
        (kind, label.into(), source.into())
    }

    #[test]
    fn finds_undefined_footnotes_and_references() {
        assert_eq!(
            found("a[^n] [x][full] [col][] [**b**][em] [short] [x][]\n"),
            [
                item("footnote", "n", "[^n]"),
                item("reference", "full", "[full]"),
                item("reference", "col", "[col][]"),
                item("reference", "em", "[em]"),
                item("reference", "x", "[x][]"),
            ]
        );
    }

    #[test]
    fn escapes_and_code_are_skipped() {
        assert_eq!(
            found("\\[^n] \\[x][y] [x]\\[y] `[x][y]` [x][\\]\n\n    [x][y]\n"),
            []
        );
    }

    #[test]
    fn labels_do_not_span_lines() {
        assert_eq!(found("[x][a\nb] [^a\nb]\n"), []);
    }
}
