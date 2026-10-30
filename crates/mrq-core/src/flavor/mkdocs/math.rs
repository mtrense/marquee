//! The `math` pass: arithmatex's `\(…\)` (inline) and `\[…\]` (display) math.
//!
//! comrak reads `\(` as an escaped `(`, so the delimiters survive only in the source of
//! text nodes. The pass looks at the children of every node with text children, except
//! inside links and images: an unescaped `\(` or `\[` in a text node opens math, and the
//! next unescaped `\)` or `\]` (respectively) in the same or a later text child of the
//! same parent closes it. Everything in between, whatever comrak made of it (emphasis,
//! line breaks, …), becomes one [`Math`] node whose literal is the verbatim source
//! between the delimiters; `\[…\]` is display math, also when it spans several lines of
//! a paragraph of its own. Unclosed openers stay text.

use crate::ast::{Ast, Event, Math, NodeId, NodeKind, Span};
use crate::flavor::text::{escaped, splice};
use crate::flavor::{Pass, PassContext};

pub(super) struct BackslashMath;

impl Pass for BackslashMath {
    fn name(&self) -> &'static str {
        "math"
    }

    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
        let mut parents = Vec::new();
        let mut events = ast.traverse(ast.root());
        while let Some(event) = events.next() {
            let Event::Enter(id) = event else { continue };
            if matches!(ast.kind(id), NodeKind::Link(_) | NodeKind::Image(_)) {
                events.skip_children();
            } else if ast
                .children(id)
                .iter()
                .any(|&child| matches!(ast.kind(child), NodeKind::Text(_)))
            {
                parents.push(id);
            }
        }
        for parent in parents {
            replace(ast, cx.src(), parent);
        }
    }
}

/// Replaces the math in the children of `parent` by math nodes.
fn replace(ast: &mut Ast, src: &str, parent: NodeId) {
    let texts: Vec<Span> = ast
        .children(parent)
        .iter()
        .filter(|&&id| matches!(ast.kind(id), NodeKind::Text(_)))
        .map(|&id| ast.span(id))
        .collect();
    let mut matches = Vec::new();
    // The text node and source offset to search from.
    let (mut index, mut from) = (0, texts.first().map_or(0, |s| s.start));
    while index < texts.len() {
        let Some((open, display)) = find(src, texts[index], from, &["\\(", "\\["]) else {
            index += 1;
            from = texts.get(index).map_or(0, |s| s.start);
            continue;
        };
        let closer = if display { "\\]" } else { "\\)" };
        let close = (index..texts.len()).find_map(|i| {
            let start = if i == index { open + 2 } else { texts[i].start };
            find(src, texts[i], start, &[closer]).map(|(at, _)| (i, at))
        });
        let Some((close_index, close)) = close else {
            from = open + 2;
            continue;
        };
        let span = Span::new(open, close + 2);
        let math = NodeKind::Math(Math {
            display,
            literal: src[open + 2..close].to_owned(),
        });
        matches.push((span, ast.create(math, span)));
        (index, from) = (close_index, close + 2);
    }
    if !matches.is_empty() {
        splice(ast, src, parent, &matches);
    }
}

/// The first unescaped delimiter of `delimiters` in text `span` at or after `from`, and
/// whether it is the second one.
fn find(src: &str, span: Span, from: usize, delimiters: &[&str]) -> Option<(usize, bool)> {
    let text = &src[span.start..span.end];
    let bytes = text.as_bytes();
    let mut at = from - span.start;
    while let Some(rel) = text[at..].find('\\') {
        let pos = at + rel;
        if !escaped(bytes, pos) {
            let rest = &text[pos..];
            if let Some(which) = delimiters.iter().position(|d| rest.starts_with(d)) {
                return Some((span.start + pos, which == 1));
            }
        }
        at = pos + 1;
    }
    None
}
