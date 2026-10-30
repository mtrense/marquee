//! The `inlines` pass: wikilinks, embeds, tags and `%%` comments inside paragraphs.
//!
//! The pass works on the children of every node that has text children (paragraphs,
//! headings, table cells, emphasis, …), except inside links and images. It scans the
//! source of those children as one line of text in which every other child is a
//! placeholder, so syntax is only found in text: never in code spans, math, raw HTML,
//! autolinks or link destinations and titles. Each match is cut out of the text nodes it
//! touches and replaced by one node spanning exactly its source; the text before and
//! after it stays as text nodes with their own spans and literals.
//!
//! - `[[target#anchor|alias]]` is a [`WikiLink`], `![[…]]` an
//!   [`Embed`](NodeKind::Embed). Target, anchor and alias are trimmed and an empty anchor
//!   or alias is `None`; `\|` (as needed in tables) separates the alias too. The link
//!   must stay on one line, contain no `[` or `]`, and name a target or an anchor. It may
//!   span emphasis (`[[a *b*]]`) but not code, links or other atoms.
//! - `#tag` is a [`Tag`] when the `#` starts the text or follows whitespace, and the
//!   name (letters, digits, `_`, `-`, `/`) has a non-digit: `#1984` and `x#y` are not
//!   tags, and URL fragments are inside links anyway.
//! - `%%…%%` is a [`Comment`] with [`CommentSyntax::Percent`] and the verbatim source
//!   between the delimiters as literal. It may span breaks and other inlines of the same
//!   parent. A paragraph that consists of one such comment becomes a block-level
//!   comment.
//!
//! A delimiter preceded by an odd number of backslashes is escaped.
//!
//! Limits: syntax must start and end in text nodes of the same parent, so a wikilink or
//! comment that comrak's emphasis parsing split across nesting levels (`*[[a*]]`) is not
//! recognized; neither is a wikilink whose `[label]` comrak resolved as a reference
//! link.

use crate::ast::{Ast, Comment, CommentSyntax, Event, NodeId, NodeKind, Span, Tag, WikiLink};
use crate::flavor::text::{escaped, splice};
use crate::flavor::{Pass, PassContext};

pub(super) struct Inlines;

impl Pass for Inlines {
    fn name(&self) -> &'static str {
        "inlines"
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
        for &parent in &parents {
            split(ast, cx.src(), parent);
        }
        for parent in parents {
            promote_lone_comment(ast, parent);
        }
    }
}

/// Stands for a soft or hard line break in the scanned text.
const BREAK: char = '\n';
/// Stands for emphasis-like formatting, which a wikilink may span.
const FORMAT: char = '\u{2}';
/// Stands for any other non-text inline (code, math, links, HTML, …).
const ATOM: char = '\u{1}';

/// One child of the scanned parent: where it sits in the scanned text and the source.
#[derive(Debug, Clone, Copy)]
struct Segment {
    /// Offset in the scanned text.
    at: usize,
    span: Span,
    text: bool,
}

/// What a match in the scanned text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Syntax {
    WikiLink,
    Embed,
    Tag,
    Comment,
}

/// A match: `start..end` in the scanned text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Found {
    start: usize,
    end: usize,
    syntax: Syntax,
}

/// Replaces the flavor syntax in the children of `parent` by nodes.
fn split(ast: &mut Ast, src: &str, parent: NodeId) {
    let mut scanned = String::new();
    let segments: Vec<Segment> = ast
        .children(parent)
        .iter()
        .map(|&child| {
            let span = ast.span(child);
            let at = scanned.len();
            let kind = ast.kind(child);
            match kind {
                NodeKind::Text(_) => scanned.push_str(&src[span.start..span.end]),
                NodeKind::SoftBreak | NodeKind::LineBreak => scanned.push(BREAK),
                NodeKind::Emphasis
                | NodeKind::Strong
                | NodeKind::Strikethrough
                | NodeKind::Highlight
                | NodeKind::Superscript
                | NodeKind::Subscript => scanned.push(FORMAT),
                _ => scanned.push(ATOM),
            }
            Segment {
                at,
                span,
                text: matches!(kind, NodeKind::Text(_)),
            }
        })
        .collect();
    let found = scan(&scanned);
    if found.is_empty() {
        return;
    }
    // Matches start and end with text, so both ends map into text segments.
    let to_src = |offset: usize, probe: usize| {
        let segment = segments[segments.partition_point(|s| s.at <= probe) - 1];
        debug_assert!(segment.text);
        segment.span.start + offset - segment.at
    };
    let matches: Vec<(Span, NodeId)> = found
        .iter()
        .map(|f| {
            let span = Span::new(to_src(f.start, f.start), to_src(f.end, f.end - 1));
            let node = ast.create(node_kind(f.syntax, &src[span.start..span.end]), span);
            (span, node)
        })
        .collect();

    splice(ast, src, parent, &matches);
}

/// The node for `syntax` with source `source`.
fn node_kind(syntax: Syntax, source: &str) -> NodeKind {
    let inner = |open: usize| source[open..source.len() - 2].to_owned();
    match syntax {
        Syntax::WikiLink => NodeKind::WikiLink(parse_wikilink(&source[2..source.len() - 2])),
        Syntax::Embed => NodeKind::Embed(parse_wikilink(&source[3..source.len() - 2])),
        Syntax::Tag => NodeKind::Tag(Tag {
            name: source[1..].to_owned(),
        }),
        Syntax::Comment => NodeKind::Comment(Comment {
            syntax: CommentSyntax::Percent,
            literal: inner(2),
        }),
    }
}

/// Splits the inside of `[[…]]` into target, anchor and alias.
fn parse_wikilink(inner: &str) -> WikiLink {
    let (link, alias) = match inner.find('|') {
        Some(bar) => {
            let link = &inner[..bar];
            (
                link.strip_suffix('\\').unwrap_or(link),
                Some(&inner[bar + 1..]),
            )
        }
        None => (inner, None),
    };
    let (target, anchor) = match link.split_once('#') {
        Some((target, anchor)) => (target, Some(anchor)),
        None => (link, None),
    };
    let part = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    WikiLink {
        target: target.trim().to_owned(),
        anchor: part(anchor),
        alias: part(alias),
    }
}

/// The matches in `text`, in order and without overlap.
fn scan(text: &str) -> Vec<Found> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let next = match bytes[at] {
            b'%' => comment_at(text, at),
            b'[' => wikilink_at(text, at).map(|end| (end, Syntax::WikiLink)),
            b'!' if !escaped(bytes, at) => {
                wikilink_at(text, at + 1).map(|end| (end, Syntax::Embed))
            }
            b'#' => tag_at(text, at),
            _ => None,
        };
        match next {
            Some((end, syntax)) => {
                found.push(Found {
                    start: at,
                    end,
                    syntax,
                });
                at = end;
            }
            None => at += 1,
        }
    }
    found
}

/// Offsets of the unescaped `%%` delimiters in `text`, left to right without overlap.
pub(super) fn percent_delimiters(text: &str) -> Vec<usize> {
    let mut delimiters = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find("%%") {
        let at = from + rel;
        if escaped(text.as_bytes(), at) {
            from = at + 1;
        } else {
            delimiters.push(at);
            from = at + 2;
        }
    }
    delimiters
}

/// The end of the `%%…%%` comment opening at `at`.
fn comment_at(text: &str, at: usize) -> Option<(usize, Syntax)> {
    if escaped(text.as_bytes(), at) {
        return None;
    }
    let delimiters = percent_delimiters(&text[at..]);
    match delimiters[..] {
        [0, close, ..] => Some((at + close + 2, Syntax::Comment)),
        _ => None,
    }
}

/// The end of the `[[…]]` wikilink opening at `at`.
fn wikilink_at(text: &str, at: usize) -> Option<usize> {
    if !text[at..].starts_with("[[") || escaped(text.as_bytes(), at) {
        return None;
    }
    let inner_start = at + 2;
    let close = inner_start + text[inner_start..].find([']', '[', BREAK, ATOM])?;
    if !text[close..].starts_with("]]") {
        return None;
    }
    let link = parse_wikilink(&text[inner_start..close]);
    (!link.target.is_empty() || link.anchor.is_some()).then_some(close + 2)
}

/// The end of the `#tag` at `at`.
fn tag_at(text: &str, at: usize) -> Option<(usize, Syntax)> {
    let after_space = text[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    if !after_space || escaped(text.as_bytes(), at) {
        return None;
    }
    let name_start = at + 1;
    let len = text[name_start..]
        .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/')))
        .unwrap_or(text.len() - name_start);
    let name = &text[name_start..name_start + len];
    name.contains(|c: char| !c.is_ascii_digit())
        .then_some((name_start + len, Syntax::Tag))
}

/// Replaces a paragraph whose only child is a `%%` comment by the comment.
fn promote_lone_comment(ast: &mut Ast, paragraph: NodeId) {
    if !ast.is_attached(paragraph) || ast.kind(paragraph) != &NodeKind::Paragraph {
        return;
    }
    let &[comment] = ast.children(paragraph) else {
        return;
    };
    if let NodeKind::Comment(Comment {
        syntax: CommentSyntax::Percent,
        ..
    }) = ast.kind(comment)
    {
        ast.detach(comment).expect("a child is not the root");
        ast.replace(paragraph, comment)
            .expect("an attached paragraph has a parent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(text: &str) -> Vec<(&str, Syntax)> {
        scan(text)
            .into_iter()
            .map(|f| (&text[f.start..f.end], f.syntax))
            .collect()
    }

    #[test]
    fn scanning_finds_each_syntax() {
        use Syntax::{Comment, Embed, Tag, WikiLink};
        assert_eq!(
            matches("[[a]] ![[b|1]] #c %%d%% e"),
            [
                ("[[a]]", WikiLink),
                ("![[b|1]]", Embed),
                ("#c", Tag),
                ("%%d%%", Comment)
            ]
        );
        assert_eq!(matches("%%[[a]] #b%%"), [("%%[[a]] #b%%", Comment)]);
        assert_eq!(matches("[[a [[b]]"), [("[[b]]", WikiLink)]);
        assert_eq!(
            matches("[[a\u{2}b\u{2}]]"),
            [("[[a\u{2}b\u{2}]]", WikiLink)]
        );
        assert_eq!(matches("\\![[a]]"), [("[[a]]", WikiLink)]);
        assert_eq!(matches("[[a\u{1}]] [[a\nb]] [[]] [[#]] [[|x]] \\[[a]]"), []);
        assert_eq!(matches("\\%%a %%b"), []);
        assert_eq!(matches("a#b \\#c #1 #1a\u{2}#d"), [("#1a", Tag)]);
    }

    #[test]
    fn wikilinks_parse() {
        let link = |s| {
            let l = parse_wikilink(s);
            (l.target, l.anchor, l.alias)
        };
        assert_eq!(
            link(" a # b | c "),
            ("a".into(), Some("b".into()), Some("c".into()))
        );
        assert_eq!(link("a\\|b"), ("a".into(), None, Some("b".into())));
        assert_eq!(link("#^x"), (String::new(), Some("^x".into()), None));
        assert_eq!(link("a#b#c|"), ("a".into(), Some("b#c".into()), None));
    }

    #[test]
    fn percent_delimiters_skip_escapes() {
        assert_eq!(percent_delimiters("%%a\\%%b%%%"), [0, 7]);
        assert_eq!(percent_delimiters("\\\\%%"), [2]);
    }
}
