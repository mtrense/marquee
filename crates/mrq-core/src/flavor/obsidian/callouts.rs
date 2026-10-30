//! The `callouts` pass: `> [!kind]± title` block quotes become callouts.
//!
//! A block quote is a callout when its first child is a paragraph whose first line starts
//! with `[!kind]` as plain text (so `\[!note]` and a `[!note]` link are not). The kind is
//! anything up to `]` without whitespace or brackets, kept as written (`NOTE`, `tip`,
//! `my-kind`). A `-` or `+` right after the `]` makes it foldable (closed or open); the
//! rest of the line, trimmed, is the title, verbatim source (inline markup included).
//! The marker line is removed from the paragraph, and the paragraph too if nothing else is
//! left. The callout keeps the quote's span and children, so nested quotes nest callouts.

use crate::ast::{Ast, Callout, Fold, NodeId, NodeKind, NodeType};
use crate::flavor::{Pass, PassContext};

pub(super) struct Callouts;

impl Pass for Callouts {
    fn name(&self) -> &'static str {
        "callouts"
    }

    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
        let quotes: Vec<NodeId> = ast
            .descendants(ast.root())
            .filter(|&id| ast.kind(id).node_type() == NodeType::BlockQuote)
            .collect();
        for quote in quotes {
            convert(ast, cx.src(), quote);
        }
    }
}

/// Turns `quote` into a callout if it starts with a callout marker.
fn convert(ast: &mut Ast, src: &str, quote: NodeId) {
    let Some(&para) = ast.children(quote).first() else {
        return;
    };
    let span = ast.span(para);
    let starts_with_text = ast.children(para).first().is_some_and(|&first| {
        matches!(ast.kind(first), NodeKind::Text(_)) && ast.span(first).start == span.start
    });
    if ast.kind(para) != &NodeKind::Paragraph || !starts_with_text {
        return;
    }
    let text = &src[span.start..span.end];
    let line = &text[..text.find(['\r', '\n']).unwrap_or(text.len())];
    let Some(callout) = parse_marker(line) else {
        return;
    };
    ast.set_kind(quote, NodeKind::Callout(callout));

    // Drop the marker line: every inline up to and including the break that ends it.
    let children = ast.take_children(para);
    let body = children
        .iter()
        .position(|&id| matches!(ast.kind(id), NodeKind::SoftBreak | NodeKind::LineBreak))
        .map_or(&[][..], |end| &children[end + 1..]);
    match body.first() {
        Some(&first) => {
            ast.set_span(para, (ast.span(first).start..span.end).into());
            for &id in body {
                ast.append_child(para, id)
                    .expect("taken children are detached");
            }
        }
        None => ast.detach(para).expect("a paragraph is not the root"),
    }
}

/// The callout of a marker line `[!kind]± title`.
fn parse_marker(line: &str) -> Option<Callout> {
    let rest = line.strip_prefix("[!")?;
    let end = rest.find(']')?;
    let kind = &rest[..end];
    if kind.is_empty() || kind.contains(|c: char| c.is_whitespace() || c == '[') {
        return None;
    }
    let mut after = &rest[end + 1..];
    let fold = match after.chars().next() {
        Some('+') => Some(Fold::Open),
        Some('-') => Some(Fold::Closed),
        _ => None,
    };
    if fold.is_some() {
        after = &after[1..];
    }
    let title = after.trim();
    Some(Callout {
        kind: kind.to_owned(),
        title: (!title.is_empty()).then(|| title.to_owned()),
        fold,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_parse() {
        let marker = |line| parse_marker(line).map(|c| (c.kind, c.title, c.fold));
        assert_eq!(
            marker("[!tip]- A *b* "),
            Some(("tip".into(), Some("A *b*".into()), Some(Fold::Closed)))
        );
        assert_eq!(marker("[!x]+"), Some(("x".into(), None, Some(Fold::Open))));
        assert_eq!(
            marker("[!NOTE]Title"),
            Some(("NOTE".into(), Some("Title".into()), None))
        );
        assert_eq!(marker("[!]"), None);
        assert_eq!(marker("[!a b]"), None);
        assert_eq!(marker("[!a"), None);
        assert_eq!(marker("x [!a]"), None);
    }
}
