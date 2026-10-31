//! Byte spans from parser positions, and repair of missing or inconsistent spans.

use crate::ast::{Ast, NodeId, NodeKind, Span};

/// Byte offsets of line starts, for converting 1-based (line, byte column) positions.
/// Lines end at `\n`, `\r\n` or a lone `\r`, as in `CommonMark`.
#[derive(Debug)]
pub(super) struct LineIndex<'a> {
    src: &'a str,
    /// Offset of the first byte of each line; never empty, the first entry is 0.
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub(super) fn new(src: &'a str) -> Self {
        let bytes = src.as_bytes();
        let mut starts = vec![0];
        for (i, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' || (byte == b'\r' && bytes.get(i + 1) != Some(&b'\n')) {
                starts.push(i + 1);
            }
        }
        Self { src, starts }
    }

    /// The span from the 1-based `start` position to the 1-based, inclusive `end`
    /// position, or `None` if either is not a position in the text or falls inside a
    /// character.
    ///
    /// An end on the line ending takes all of it (`\r\n` included); an end past it is
    /// clamped to the end of the line's content. An end in column 0 (comrak's "before the first
    /// character") ends at the line start.
    pub(super) fn span(&self, start: (usize, usize), end: (usize, usize)) -> Option<Span> {
        let from = self.line_start(start.0)? + start.1.checked_sub(1)?;
        if from > self.line_limit(start.0) {
            return None;
        }
        let end_line_start = self.line_start(end.0)?;
        let mut to = end_line_start + end.1;
        if to > self.line_limit(end.0) {
            // An end past the line ending ends with the line's content.
            to = end_line_start + self.line_text(end.0).len();
        }
        let bytes = self.src.as_bytes();
        if to > 0 && bytes[to - 1] == b'\r' && bytes.get(to) == Some(&b'\n') {
            to += 1;
        }
        let valid = from <= to && self.src.is_char_boundary(from) && self.src.is_char_boundary(to);
        valid.then(|| Span::new(from, to))
    }

    /// The byte offset of the 1-based `(line, column)`, if it lies within the line or on
    /// its line ending.
    pub(super) fn offset(&self, (line, column): (usize, usize)) -> Option<usize> {
        let offset = self.line_start(line)? + column.checked_sub(1)?;
        (offset < self.line_limit(line)).then_some(offset)
    }

    /// The 1-based `(line, column)` of byte `offset`.
    pub(super) fn position(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&start| start <= offset);
        (line, offset - self.starts[line - 1] + 1)
    }

    /// The text of the 1-based `line` without its line ending; empty past the end.
    pub(super) fn line_text(&self, line: usize) -> &'a str {
        let Some(start) = self.line_start(line) else {
            return "";
        };
        self.src[start..self.line_limit(line)].trim_end_matches(['\n', '\r'])
    }

    /// Byte offset of the start of the 1-based `line`.
    fn line_start(&self, line: usize) -> Option<usize> {
        self.starts.get(line.checked_sub(1)?).copied()
    }

    /// Offset just past the line ending of the 1-based `line` (the text length for the
    /// last line).
    fn line_limit(&self, line: usize) -> usize {
        self.starts.get(line).copied().unwrap_or(self.src.len())
    }
}

/// Makes the spans of `ast` consistent; `reported[i]` is the span the parser reported for
/// arena index `i`, `None` where it reported none.
///
/// 0. A block never ends with a line ending (or blank lines after it). A fenced code
///    block covers at least its opening line and the lines of its code: comrak ends an
///    unclosed one that a blank line and the next list item close at its fence.
/// 1. Bottom-up, a node's span grows to cover its children's (a node without a span
///    gets their union). comrak's inline positions are more precise than the ends it
///    reports for some blocks, e.g. definition terms.
/// 2. Top-down, each child must lie within its parent, after its previous sibling, and
///    be non-empty; text must also match its literal (unless escapes or entities make
///    them differ). A child that doesn't is repaired from the parent's source: text is
///    searched for literally in the gap before the next good sibling, a break ends with
///    the gap's first line ending, anything else is clamped into the gap (or fills it if
///    nothing is left).
/// 3. Lists and list items end with their last child, never with trailing blank lines.
///
/// Every span ends up on `char` boundaries.
pub(super) fn repair(ast: &mut Ast, src: &str, reported: &[Option<Span>]) {
    let root = ast.root();
    let mut spans: Vec<Option<Span>> = reported
        .iter()
        .map(|span| span.filter(|s| s.start <= s.end && s.end <= src.len()))
        .collect();
    spans.resize(ast.node_count(), None);
    for id in ast.descendants(root) {
        if ast.kind(id).is_block()
            && let Some(span) = &mut spans[id.index()]
        {
            if let NodeKind::CodeBlock(code) = ast.kind(id)
                && code.fence.is_some()
            {
                span.end = span
                    .end
                    .max(fenced_code_end(src, span.start, &code.literal));
            }
            span.end = without_trailing_lines(src, *span);
        }
    }
    spans[root.index()] = Some(Span::new(0, src.len()));

    // 1. Grow containers to cover their children.
    let order: Vec<NodeId> = ast.descendants(root).collect();
    for &id in order.iter().rev() {
        spans[id.index()] = ast
            .children(id)
            .iter()
            .filter_map(|child| spans[child.index()])
            .chain(spans[id.index()])
            .reduce(union);
    }

    // 2. Fit children into parents, in pre-order so parents are final first.
    ast.set_span(root, Span::new(0, src.len()));
    for &parent in &order {
        let outer = ast.span(parent);
        let children = ast.children(parent).to_vec();
        let mut cursor = outer.start;
        for (index, &child) in children.iter().enumerate() {
            let good = |id: NodeId, cursor: usize| {
                spans[id.index()].filter(|&s| {
                    s.start >= cursor
                        && s.end <= outer.end
                        && !s.is_empty()
                        && plausible(ast, id, src, s)
                })
            };
            let span = good(child, cursor).unwrap_or_else(|| {
                // The gap this child may occupy: up to the next good sibling.
                let gap_end = children[index + 1..]
                    .iter()
                    .find_map(|&next| good(next, cursor))
                    .map_or(outer.end, |s| s.start);
                let reported = spans[child.index()];
                fit(ast.kind(child), src, reported, Span::new(cursor, gap_end))
            });
            let span = with_leading_escape(ast.kind(child), src, cursor, snap(src, span));
            ast.set_span(child, span);
            cursor = span.end;
        }
    }

    // 3. Trim lists and items to their content.
    for &id in order.iter().rev() {
        if matches!(ast.kind(id), NodeKind::List(_) | NodeKind::ListItem(_))
            && let Some(&last) = ast.children(id).last()
        {
            let span = ast.span(id);
            let end = ast.span(last).end.max(span.start);
            ast.set_span(id, Span::new(span.start, end));
        }
    }
}

/// Whether `span` can be the source of `id`: text must match its literal.
fn plausible(ast: &Ast, id: NodeId, src: &str, span: Span) -> bool {
    match ast.kind(id) {
        NodeKind::Text(literal) => text_matches(&src[span.start..span.end], literal),
        _ => true,
    }
}

/// Whether `source` can be the source of a text node with `literal`: equal once
/// backslash escapes are resolved, or, where entities make them differ, equal before the
/// first `&` and after the last `;`.
pub(super) fn text_matches(source: &str, literal: &str) -> bool {
    if source == literal {
        return true;
    }
    let mut unescaped = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(&next) if c == '\\' && next.is_ascii_punctuation() => {}
            _ => unescaped.push(c),
        }
    }
    if unescaped == literal {
        return true;
    }
    match (unescaped.find('&'), unescaped.rfind(';')) {
        (Some(first), Some(last)) if first < last => {
            literal.starts_with(&unescaped[..first]) && literal.ends_with(&unescaped[last + 1..])
        }
        _ => false,
    }
}

/// The end of `span` without the whitespace-only lines and the line ending it may end
/// with; trailing spaces on its last line stay.
fn without_trailing_lines(src: &str, span: Span) -> usize {
    let text = &src[span.start..span.end];
    let content = text.trim_end_matches([' ', '\t', '\n', '\r']);
    let trailing = &text[content.len()..];
    let cut = trailing.find(['\n', '\r']).unwrap_or(trailing.len());
    span.start + content.len() + cut
}

/// The end of the last line of `literal`'s code in a fenced block opening at `start`
/// (with its line ending): as many lines after the opening line as the code has,
/// without trailing blank ones. Lines end with `\n`, `\r\n` or `\r`.
fn fenced_code_end(src: &str, start: usize, literal: &str) -> usize {
    let code = literal.trim_end_matches(['\n', '\r']);
    let lines = if code.is_empty() {
        0
    } else {
        code.replace("\r\n", "\n").matches(['\n', '\r']).count() + 1
    };
    let mut end = start;
    for _ in 0..=lines {
        let Some(at) = src[end..].find(['\n', '\r']) else {
            return src.len();
        };
        end += at + 1 + usize::from(src[end + at..].starts_with("\r\n"));
    }
    end
}

fn union(a: Span, b: Span) -> Span {
    Span::new(a.start.min(b.start), a.end.max(b.end))
}

/// A span for a node of `kind` within `gap`, given what the parser `reported`.
fn fit(kind: &NodeKind, src: &str, reported: Option<Span>, gap: Span) -> Span {
    let text = &src[gap.start..gap.end];
    // Text is searched for as is, then without the spaces a partly consumed tab leaves.
    if let NodeKind::Text(literal) = kind
        && let Some((at, len)) = [literal.as_str(), literal.trim_start_matches(' ')]
            .into_iter()
            .filter(|needle| !needle.is_empty())
            .find_map(|needle| Some((text.find(needle)?, needle.len())))
    {
        return Span::new(gap.start + at, gap.start + at + len);
    }
    if matches!(kind, NodeKind::SoftBreak | NodeKind::LineBreak)
        && let Some(at) = text.find(['\n', '\r'])
    {
        let len = if text[at..].starts_with("\r\n") { 2 } else { 1 };
        return Span::new(gap.start, gap.start + at + len);
    }
    if let Some(s) = reported {
        let start = s.start.clamp(gap.start, gap.end);
        let end = s.end.clamp(start, gap.end);
        if start < end {
            return Span::new(start, end);
        }
    }
    gap
}

/// For text whose span starts right after the backslash escaping its first character
/// (comrak reports text after an inline from behind the backslash, as in `*a*\)`),
/// the span with the backslash, if no earlier sibling claimed it (it lies after
/// `cursor`). Other spans are returned as they are.
fn with_leading_escape(kind: &NodeKind, src: &str, cursor: usize, span: Span) -> Span {
    let NodeKind::Text(literal) = kind else {
        return span;
    };
    let escapes = src[..span.start]
        .bytes()
        .rev()
        .take_while(|&b| b == b'\\')
        .count()
        .min(span.start - cursor.min(span.start));
    let first = src[span.start..span.end].chars().next();
    if escapes % 2 == 1 && first.is_some_and(|c| c.is_ascii_punctuation() && literal.starts_with(c))
    {
        Span::new(span.start - 1, span.end)
    } else {
        span
    }
}

/// Widens `span` to the nearest `char` boundaries.
fn snap(src: &str, span: Span) -> Span {
    let mut start = span.start;
    while !src.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = span.end.max(start);
    while !src.is_char_boundary(end) {
        end += 1;
    }
    Span::new(start, end)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{NodeKind, NodeType};

    fn span(start: usize, end: usize) -> Span {
        Span::new(start, end)
    }

    #[test]
    fn positions_convert_across_line_endings() {
        let src = "ab\ncd\r\nef\rgh";
        let index = LineIndex::new(src);
        assert_eq!(index.span((1, 1), (1, 2)), Some(span(0, 2)));
        assert_eq!(index.span((2, 1), (2, 2)), Some(span(3, 5)));
        assert_eq!(index.span((3, 1), (4, 2)), Some(span(7, 12)));
        // An end on the line ending covers all of it, `\r\n` included.
        assert_eq!(index.span((2, 2), (2, 3)), Some(span(4, 7)));
        assert_eq!(index.span((1, 1), (1, 3)), Some(span(0, 3)));
        assert_eq!(index.line_text(2), "cd");
        assert_eq!(index.line_text(4), "gh");
        assert_eq!(index.line_text(5), "");
    }

    #[test]
    fn offsets_and_positions_convert_both_ways() {
        let index = LineIndex::new("ab\ncd\r\nef");
        assert_eq!(index.offset((2, 2)), Some(4));
        assert_eq!(index.offset((2, 4)), Some(6));
        assert_eq!(index.offset((2, 5)), None);
        assert_eq!(index.offset((0, 1)), None);
        assert_eq!(index.position(0), (1, 1));
        assert_eq!(index.position(4), (2, 2));
        assert_eq!(index.position(7), (3, 1));
    }

    #[test]
    fn invalid_positions_are_none() {
        let index = LineIndex::new("ab\ncd");
        assert_eq!(index.span((0, 0), (0, 0)), None);
        assert_eq!(index.span((3, 1), (3, 1)), None);
        assert_eq!(index.span((2, 2), (1, 1)), None);
        assert_eq!(index.span((1, 9), (2, 1)), None);
        assert_eq!(index.span((1, 1), (2, 9)), Some(span(0, 5)));
        assert_eq!(index.span((1, 1), (1, 5)), Some(span(0, 2)));
        assert_eq!(index.span((1, 1), (2, 0)), Some(span(0, 3)));
    }

    #[test]
    fn empty_text_has_one_line() {
        let index = LineIndex::new("");
        assert_eq!(index.span((1, 1), (1, 1)), Some(span(0, 0)));
    }

    /// `document > paragraph > [text "ab", strong > [text "cd"], text "ef"]` over
    /// `"ab **cd** ef"`, with the given reported spans (in creation order).
    fn tree(reported: [Option<Span>; 6]) -> (Ast, Vec<Option<Span>>) {
        let mut ast = Ast::new(span(0, 12));
        let kinds = [
            NodeKind::Paragraph,
            NodeKind::Text("ab ".into()),
            NodeKind::Strong,
            NodeKind::Text("cd".into()),
            NodeKind::Text(" ef".into()),
        ];
        let ids: Vec<_> = kinds
            .into_iter()
            .map(|kind| ast.create(kind, Span::default()))
            .collect();
        ast.append_child(ast.root(), ids[0]).unwrap();
        ast.append_child(ids[0], ids[1]).unwrap();
        ast.append_child(ids[0], ids[2]).unwrap();
        ast.append_child(ids[2], ids[3]).unwrap();
        ast.append_child(ids[0], ids[4]).unwrap();
        (ast, reported.to_vec())
    }

    fn spans(ast: &Ast) -> Vec<(NodeType, Span)> {
        ast.descendants(ast.root())
            .map(|id| (ast.kind(id).node_type(), ast.span(id)))
            .collect()
    }

    const SRC: &str = "ab **cd** ef";

    #[test]
    fn reported_spans_are_kept() {
        let good = [
            Some(span(0, 12)),
            Some(span(0, 12)),
            Some(span(0, 3)),
            Some(span(3, 9)),
            Some(span(5, 7)),
            Some(span(9, 12)),
        ];
        let (mut ast, reported) = tree(good);
        repair(&mut ast, SRC, &reported);
        let got: Vec<_> = spans(&ast).into_iter().map(|(_, s)| Some(s)).collect();
        assert_eq!(got, good);
    }

    #[test]
    fn missing_text_spans_are_found_in_the_parent() {
        let (mut ast, reported) = tree([
            Some(span(0, 12)),
            Some(span(0, 12)),
            None,
            Some(span(3, 9)),
            None,
            None,
        ]);
        repair(&mut ast, SRC, &reported);
        let got: Vec<_> = spans(&ast).into_iter().map(|(_, s)| s).collect();
        assert_eq!(
            got,
            [
                span(0, 12),
                span(0, 12),
                span(0, 3),
                span(3, 9),
                span(5, 7),
                span(9, 12)
            ]
        );
    }

    #[test]
    fn missing_containers_cover_their_children() {
        let (mut ast, reported) = tree([
            Some(span(0, 12)),
            None,
            Some(span(0, 3)),
            None,
            Some(span(5, 7)),
            Some(span(9, 12)),
        ]);
        repair(&mut ast, SRC, &reported);
        assert_eq!(spans(&ast)[1].1, span(0, 12));
        assert_eq!(spans(&ast)[3].1, span(5, 7));
    }

    #[test]
    fn text_whose_source_differs_from_its_literal_is_relocated() {
        let (mut ast, reported) = tree([
            Some(span(0, 12)),
            Some(span(0, 12)),
            Some(span(0, 3)),
            Some(span(3, 9)),
            Some(span(3, 5)),
            Some(span(9, 12)),
        ]);
        repair(&mut ast, SRC, &reported);
        assert_eq!(spans(&ast)[4].1, span(5, 7));
    }

    #[test]
    fn text_after_an_inline_keeps_its_leading_escape() {
        let src = "*a*\\) b";
        let mut ast = Ast::new(span(0, 7));
        let para = ast.create(NodeKind::Paragraph, Span::default());
        let emph = ast.create(NodeKind::Emphasis, Span::default());
        let a = ast.create(NodeKind::Text("a".into()), Span::default());
        let rest = ast.create(NodeKind::Text(") b".into()), Span::default());
        ast.append_child(ast.root(), para).unwrap();
        ast.append_child(para, emph).unwrap();
        ast.append_child(emph, a).unwrap();
        ast.append_child(para, rest).unwrap();
        let reported = vec![
            Some(span(0, 7)),
            Some(span(0, 7)),
            Some(span(0, 3)),
            Some(span(1, 2)),
            Some(span(4, 7)),
        ];
        repair(&mut ast, src, &reported);
        assert_eq!(ast.span(rest), span(3, 7));
        // An escaped backslash before the text is not an escape of it.
        let src = "*a*\\\\) b";
        let mut ast2 = ast.clone();
        let reported = vec![
            Some(span(0, 8)),
            Some(span(0, 8)),
            Some(span(0, 3)),
            Some(span(1, 2)),
            Some(span(5, 8)),
        ];
        ast2.set_span(ast2.root(), span(0, 8));
        repair(&mut ast2, src, &reported);
        assert_eq!(ast2.span(rest), span(5, 8));
    }

    #[test]
    fn positions_inside_characters_are_none() {
        let index = LineIndex::new("aé");
        assert_eq!(index.span((1, 1), (1, 3)), Some(span(0, 3)));
        assert_eq!(index.span((1, 3), (1, 3)), None);
        assert_eq!(index.span((1, 1), (1, 2)), None);
    }

    #[test]
    fn text_matches_through_escapes_and_entities() {
        assert!(text_matches("ab", "ab"));
        assert!(text_matches("a\\#b\\\\", "a#b\\"));
        assert!(text_matches("x &amp; y &copy;", "x & y ©"));
        assert!(text_matches("b \\| c", "b | c"));
        assert!(!text_matches("[a]: ", "a#aa"));
        assert!(!text_matches("x &amp; z", "x & y"));
        assert!(!text_matches("\\a", "\\b"));
    }

    #[test]
    fn missing_breaks_take_the_line_ending_in_their_gap() {
        let src = "a  \r\n b";
        let mut ast = Ast::new(span(0, 0));
        let para = ast.create(NodeKind::Paragraph, Span::default());
        ast.append_child(ast.root(), para).unwrap();
        for kind in [
            NodeKind::Text("a".into()),
            NodeKind::LineBreak,
            NodeKind::Text("b".into()),
        ] {
            let id = ast.create(kind, Span::default());
            ast.append_child(para, id).unwrap();
        }
        repair(
            &mut ast,
            src,
            &[None, None, Some(span(0, 1)), None, Some(span(6, 7))],
        );
        assert_eq!(ast.span(ast.children(para)[1]), span(1, 5));
    }

    #[test]
    fn blocks_end_before_trailing_line_endings() {
        let src = "a  \n \n";
        let mut ast = Ast::new(span(0, 0));
        let para = ast.create(NodeKind::Paragraph, Span::default());
        ast.append_child(ast.root(), para).unwrap();
        repair(&mut ast, src, &[None, Some(span(0, 6))]);
        assert_eq!(ast.span(para), span(0, 3));
    }

    #[test]
    fn containers_grow_to_cover_their_children() {
        let (mut ast, reported) = tree([
            Some(span(0, 12)),
            Some(span(0, 1)),
            Some(span(0, 3)),
            Some(span(3, 9)),
            Some(span(5, 7)),
            Some(span(9, 12)),
        ]);
        repair(&mut ast, SRC, &reported);
        assert_eq!(spans(&ast)[1].1, span(0, 12));
    }

    #[test]
    fn spans_outside_the_parent_or_overlapping_are_clamped() {
        let (mut ast, reported) = tree([
            Some(span(0, 12)),
            Some(span(0, 9)),
            Some(span(0, 3)),
            Some(span(2, 9)),
            Some(span(5, 7)),
            Some(span(9, 12)),
        ]);
        repair(&mut ast, SRC, &reported);
        let got: Vec<_> = spans(&ast).into_iter().map(|(_, s)| s).collect();
        // The paragraph grew to cover " ef"; "**cd**" lost the byte "ab " overlaps.
        assert_eq!(got[1], span(0, 12));
        assert_eq!(got[2], span(0, 3));
        assert_eq!(got[3], span(3, 9));
        assert_eq!(got[5], span(9, 12));
    }

    #[test]
    fn spans_snap_to_char_boundaries() {
        let src = "é";
        let mut ast = Ast::new(span(0, 2));
        let text = ast.create(NodeKind::Text("é".into()), Span::default());
        ast.append_child(ast.root(), text).unwrap();
        repair(&mut ast, src, &[Some(span(0, 2)), Some(span(1, 1))]);
        assert_eq!(ast.span(text), span(0, 2));
    }
}
