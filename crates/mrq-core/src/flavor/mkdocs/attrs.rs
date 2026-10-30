//! The `heading_attrs` pass: `{#id .class key=value}` at the end of a heading.
//!
//! As in Python-Markdown's `attr_list`, the list must be the end of the heading's text,
//! after at least one space: `## Title {#id}`, `Title {: .c }` over a setext underline,
//! or before the closing `##` of an ATX heading. It must not be empty and holds no `}`.
//! The pass reads it from the source of the heading's last child, which must be text;
//! the `{…}` and the spaces before it are cut from that text node (which is removed if
//! nothing is left), so [`Ast::text`] of the heading excludes them, and the heading's
//! [`HeadingAttrs`] spans the `{…}`. A heading that has attributes already is skipped.
//!
//! Inside the braces, tokens are separated by spaces, and an optional `:` may follow the
//! `{`: `#id` sets the id (the last one wins), `.class` adds a class, `key=value`,
//! `key="value"` or `key='value'` adds an attribute (`id=` sets the id and `class=` adds
//! its space-separated classes instead), and any other word `w` is the attribute `w=w`.

use crate::ast::{Ast, HeadingAttrs, NodeId, NodeKind, Span};
use crate::flavor::text::Pieces;
use crate::flavor::{Pass, PassContext};

pub(super) struct HeadingAttributes;

impl Pass for HeadingAttributes {
    fn name(&self) -> &'static str {
        "heading_attrs"
    }

    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
        let headings: Vec<NodeId> = ast
            .descendants(ast.root())
            .filter(|&id| matches!(ast.kind(id), NodeKind::Heading(h) if h.attrs.is_none()))
            .collect();
        for heading in headings {
            apply(ast, cx.src(), heading);
        }
    }
}

/// Moves a trailing attribute list from the text of `heading` into its payload.
fn apply(ast: &mut Ast, src: &str, heading: NodeId) {
    let Some(&last) = ast.children(heading).last() else {
        return;
    };
    let NodeKind::Text(literal) = ast.kind(last) else {
        return;
    };
    let span = ast.span(last);
    let source = &src[span.start..span.end];
    let Some((cut, open, mut attrs)) = trailing(source) else {
        return;
    };
    attrs.span = Span::new(span.start + open, span.start + source.trim_end().len());
    if cut == 0 {
        ast.detach(last).expect("a child is not the root");
    } else {
        let piece = Pieces::new(source, literal.clone(), span.start).create(
            ast,
            src,
            span.start,
            span.start + cut,
        );
        ast.replace(last, piece)
            .expect("an attached child has a parent");
    }
    if let NodeKind::Heading(h) = ast.kind_mut(heading) {
        h.attrs = Some(attrs);
    }
}

/// For text ending in ` {…}`: where the text before it ends (spaces trimmed), where the
/// `{` is, and the parsed attributes (with an empty span).
fn trailing(source: &str) -> Option<(usize, usize, HeadingAttrs)> {
    let body = source.trim_end_matches(' ').strip_suffix('}')?;
    let open = body.rfind('{')?;
    let before = &body[..open];
    if !before.ends_with(' ') {
        return None;
    }
    let inner = &body[open + 1..];
    let inner = inner.strip_prefix(':').unwrap_or(inner).trim_matches(' ');
    if inner.is_empty() || inner.contains(['}', '\n', '\r']) {
        return None;
    }
    Some((before.trim_end_matches(' ').len(), open, parse_list(inner)))
}

/// Parses the tokens of an attribute list (without the braces).
fn parse_list(mut rest: &str) -> HeadingAttrs {
    let mut attrs = HeadingAttrs::default();
    loop {
        rest = rest.trim_start_matches(' ');
        if rest.is_empty() {
            return attrs;
        }
        let word_end = rest.find([' ', '=']).unwrap_or(rest.len());
        let word = &rest[..word_end];
        rest = &rest[word_end..];
        let Some(after) = rest.strip_prefix('=') else {
            if let Some(id) = word.strip_prefix('#') {
                attrs.id = Some(id.to_owned());
            } else if let Some(class) = word.strip_prefix('.') {
                attrs.classes.push(class.to_owned());
            } else {
                attrs.attributes.push((word.to_owned(), word.to_owned()));
            }
            continue;
        };
        // The offset of the closing quote in `after[1..]`, for a quoted value.
        let close = after
            .chars()
            .next()
            .filter(|&q| q == '"' || q == '\'')
            .and_then(|q| after[1..].find(q));
        let value = if let Some(close) = close {
            rest = &after[close + 2..];
            &after[1..=close]
        } else {
            let end = after.find(' ').unwrap_or(after.len());
            rest = &after[end..];
            &after[..end]
        };
        match word {
            "id" => attrs.id = Some(value.to_owned()),
            "class" => attrs
                .classes
                .extend(value.split_whitespace().map(str::to_owned)),
            _ => attrs.attributes.push((word.to_owned(), value.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_lists_are_found() {
        let found = |s| trailing(s).map(|(cut, open, a)| (cut, open, a.id, a.classes));
        assert_eq!(
            found("A  {: #x .y }"),
            Some((1, 3, Some("x".into()), vec!["y".into()]))
        );
        assert_eq!(found(" {#x}"), Some((0, 1, Some("x".into()), vec![])));
        assert_eq!(found("A{#x}"), None);
        assert_eq!(found("A { }"), None);
        assert_eq!(found("A {:}"), None);
        assert_eq!(found("A {a}b}"), None);
        assert_eq!(found("A {#x} b"), None);
    }

    #[test]
    fn lists_parse() {
        let attrs = parse_list("#a #b .c id=d class=\"e f\" k='v w' x= \"y\" bare");
        assert_eq!(attrs.id.as_deref(), Some("d"));
        assert_eq!(attrs.classes, ["c", "e", "f"]);
        assert_eq!(
            attrs.attributes,
            [
                ("k".into(), "v w".into()),
                ("x".into(), String::new()),
                ("\"y\"".into(), "\"y\"".into()),
                ("bare".into(), "bare".into())
            ]
        );
        assert_eq!(
            parse_list("k=\"open").attributes,
            [("k".into(), "\"open".into())]
        );
    }
}
