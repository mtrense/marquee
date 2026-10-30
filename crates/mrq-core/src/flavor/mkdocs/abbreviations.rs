//! The `abbreviations` pass: `*[abbr]: expansion` lines in paragraphs.
//!
//! As in Python-Markdown's `abbr`, a definition is one line of a paragraph:
//! `*[abbr]: expansion`, with an optional space before the `:`. The abbreviation is
//! everything up to the `]` before that `:` (non-empty, without backslashes); the
//! expansion is the rest of the line, trimmed (possibly empty). The pass splits every
//! paragraph at its line breaks (only those that are its own children, so a line inside
//! a multi-line emphasis is never a definition), turns each definition line into a
//! block-level [`AbbreviationDefinition`] spanning the trimmed line, and keeps the runs
//! of other lines as paragraphs, in order.
//!
//! Uses of an abbreviation in text are not marked; consumers look the definitions up.

use crate::ast::{AbbreviationDefinition, Ast, NodeId, NodeKind, Span};
use crate::flavor::{Pass, PassContext};

pub(super) struct Abbreviations;

impl Pass for Abbreviations {
    fn name(&self) -> &'static str {
        "abbreviations"
    }

    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
        let paragraphs: Vec<NodeId> = ast
            .descendants(ast.root())
            .filter(|&id| ast.kind(id) == &NodeKind::Paragraph)
            .collect();
        for paragraph in paragraphs {
            split(ast, cx.src(), paragraph);
        }
    }
}

/// Replaces `paragraph` by its definitions and the paragraphs between them, if it has
/// any definition lines.
fn split(ast: &mut Ast, src: &str, paragraph: NodeId) {
    // Each line: its children and the break after it.
    let mut lines: Vec<(Vec<NodeId>, Option<NodeId>)> = vec![(Vec::new(), None)];
    for &child in ast.children(paragraph) {
        if matches!(ast.kind(child), NodeKind::SoftBreak | NodeKind::LineBreak) {
            lines.last_mut().expect("never empty").1 = Some(child);
            lines.push((Vec::new(), None));
        } else {
            lines.last_mut().expect("never empty").0.push(child);
        }
    }
    let definitions: Vec<Option<(Span, AbbreviationDefinition)>> = lines
        .iter()
        .map(|(children, _)| {
            let (&first, &last) = (children.first()?, children.last()?);
            let span = Span::new(ast.span(first).start, ast.span(last).end);
            let line = src[span.start..span.end].trim_end();
            let definition = parse_definition(line)?;
            Some((Span::new(span.start, span.start + line.len()), definition))
        })
        .collect();
    if definitions.iter().all(Option::is_none) {
        return;
    }

    let mut blocks = Vec::new();
    let mut run: Vec<NodeId> = Vec::new();
    for ((children, brk), definition) in lines.into_iter().zip(definitions) {
        match definition {
            Some((span, definition)) => {
                flush(ast, &mut run, &mut blocks);
                blocks.push(ast.create(NodeKind::AbbreviationDefinition(definition), span));
            }
            None => run.extend(children.into_iter().chain(brk)),
        }
    }
    flush(ast, &mut run, &mut blocks);
    ast.take_children(paragraph);
    for block in blocks {
        ast.insert_before(paragraph, block)
            .expect("an attached paragraph has a parent");
    }
    ast.detach(paragraph).expect("a paragraph is not the root");
}

/// Wraps `run` (inlines of consecutive lines) in a new paragraph appended to `blocks`,
/// without its trailing line break.
fn flush(ast: &mut Ast, run: &mut Vec<NodeId>, blocks: &mut Vec<NodeId>) {
    while run
        .last()
        .is_some_and(|&id| matches!(ast.kind(id), NodeKind::SoftBreak | NodeKind::LineBreak))
    {
        run.pop();
    }
    let (Some(&first), Some(&last)) = (run.first(), run.last()) else {
        return;
    };
    let span = Span::new(ast.span(first).start, ast.span(last).end);
    let paragraph = ast.create(NodeKind::Paragraph, span);
    for id in run.drain(..) {
        ast.detach(id).expect("an inline is not the root");
        ast.append_child(paragraph, id)
            .expect("the inline was just detached");
    }
    blocks.push(paragraph);
}

/// The definition on a line `*[abbr]: expansion`.
fn parse_definition(line: &str) -> Option<AbbreviationDefinition> {
    let rest = line.strip_prefix("*[")?;
    rest.match_indices(']').find_map(|(close, _)| {
        let abbr = &rest[..close];
        let after = &rest[close + 1..];
        let expansion = after.strip_prefix(' ').unwrap_or(after).strip_prefix(':')?;
        (!abbr.is_empty() && !abbr.contains('\\')).then(|| AbbreviationDefinition {
            abbr: abbr.to_owned(),
            expansion: expansion.trim().to_owned(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_parse() {
        let def = |line| parse_definition(line).map(|d| (d.abbr, d.expansion));
        assert_eq!(def("*[A]: b c "), Some(("A".into(), "b c".into())));
        assert_eq!(def("*[A] :b"), Some(("A".into(), "b".into())));
        assert_eq!(def("*[a]b]: c"), Some(("a]b".into(), "c".into())));
        assert_eq!(def("*[A]:"), Some(("A".into(), String::new())));
        assert_eq!(def("*[A]  : b"), None);
        assert_eq!(def("*[]: b"), None);
        assert_eq!(def("*[a\\b]: c"), None);
        assert_eq!(def("[A]: b"), None);
    }
}
