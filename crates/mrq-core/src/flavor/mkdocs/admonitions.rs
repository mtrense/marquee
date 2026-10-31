//! The `admonitions` pass: `!!! kind "title"` headers with indented bodies.
//!
//! comrak knows nothing of admonitions: it parses
//!
//! ```text
//! !!! note "Title"
//!     First line
//!
//!     Second paragraph.
//! ```
//!
//! as a paragraph (header plus lazy continuation lines) followed by an indented code
//! block. The pass looks for a header line in every paragraph: its first line, or a line
//! after a soft or hard break of the paragraph itself (Python-Markdown finds headers
//! mid-block too; the lines before stay a paragraph). The header is
//! `(!!!|???|???+) ?kind( class)*( +"title")? *`, with kind and classes made of word
//! characters and `-`.
//!
//! The body is then read from the source lines below the header. The header's *prefix*
//! is the text between its line start and the header: container markers and
//! indentation such as `> `, `- ` or `    `. A body line repeats the prefix (`>` where
//! the prefix has one, a space for anything else, so `- ` continues as two spaces) and
//! then has 4 more spaces or a tab, which are stripped. Blank lines (whitespace, or just
//! the quote markers) are kept; the first other line ends the body, and trailing blank
//! lines are not part of it. Lines past the end of the paragraph's container are never
//! read.
//!
//! The dedented body is parsed as a document of its own with every pass of the flavor
//! ([`PassContext::parse_mapped`]), so admonitions nest, and its spans are mapped back
//! line by line: every body node spans its original, indented source. The admonition
//! spans from the header to the end of its body and replaces every sibling block that
//! starts within that range. If the last of them runs past the body (lazy continuation
//! lines such as an unindented `after` right below the body), that rest is re-parsed
//! with the prefix stripped and put after the admonition, and searched for headers in
//! turn.

use std::collections::VecDeque;

use crate::ast::{Admonition, Ast, Fold, NodeId, NodeKind, NodeType, Span};
use crate::flavor::text::repeated_prefix;
use crate::flavor::{Pass, PassContext};
use crate::parse::Parser as _;

pub(super) struct Admonitions;

impl Pass for Admonitions {
    fn name(&self) -> &'static str {
        "admonitions"
    }

    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
        let mut pending: VecDeque<NodeId> = paragraphs(ast, ast.root()).into();
        while let Some(paragraph) = pending.pop_front() {
            if !ast.is_attached(paragraph) || ast.kind(paragraph) != &NodeKind::Paragraph {
                continue;
            }
            let Some((at, admonition)) = find_header(ast, cx.src(), paragraph) else {
                continue;
            };
            let reparsed = convert(ast, cx, paragraph, at, admonition);
            for id in reparsed.into_iter().rev() {
                pending.push_front(id);
            }
        }
    }
}

/// Every paragraph below `id` (inclusive), in document order.
fn paragraphs(ast: &Ast, id: NodeId) -> Vec<NodeId> {
    ast.descendants(id)
        .filter(|&id| ast.kind(id).node_type() == NodeType::Paragraph)
        .collect()
}

/// The first header line of `paragraph`: its source offset and the admonition.
fn find_header(ast: &Ast, src: &str, paragraph: NodeId) -> Option<(usize, Admonition)> {
    let children = ast.children(paragraph);
    let starts = std::iter::once(ast.span(paragraph).start).chain(
        children
            .windows(2)
            .filter(|pair| matches!(ast.kind(pair[0]), NodeKind::SoftBreak | NodeKind::LineBreak))
            .map(|pair| ast.span(pair[1]).start),
    );
    starts
        .into_iter()
        .find_map(|at| parse_header(line_at(src, at).0).map(|header| (at, header)))
}

/// The line content starting at `at` (without its line ending) and the start of the
/// next line (`src.len()` on the last line).
fn line_at(src: &str, at: usize) -> (&str, usize) {
    let rest = &src[at..];
    match rest.find('\n') {
        Some(nl) => (
            rest[..nl].strip_suffix('\r').unwrap_or(&rest[..nl]),
            at + nl + 1,
        ),
        None => (rest, src.len()),
    }
}

/// The admonition of a header line `!!! kind classes "title"`.
fn parse_header(line: &str) -> Option<Admonition> {
    let (fold, rest) = if let Some(rest) = line.strip_prefix("???+") {
        (Some(Fold::Open), rest)
    } else if let Some(rest) = line.strip_prefix("???") {
        (Some(Fold::Closed), rest)
    } else {
        (None, line.strip_prefix("!!!")?)
    };
    let rest = rest.strip_prefix(' ').unwrap_or(rest).trim_end_matches(' ');
    let (words, title) = match rest.find('"') {
        Some(open) => {
            let quoted = &rest[open..];
            if quoted.len() < 2 || !quoted.ends_with('"') || !rest[..open].ends_with(' ') {
                return None;
            }
            (&rest[..open], Some(&quoted[1..quoted.len() - 1]))
        }
        None => (rest, None),
    };
    let word = |w: &str| {
        !w.is_empty()
            && w.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    };
    let mut parts = words
        .trim_end_matches(' ')
        .split(' ')
        .filter(|w| !w.is_empty());
    let kind = parts.next().filter(|_| !words.starts_with(' '))?;
    let classes: Vec<&str> = parts.collect();
    if !word(kind) || !classes.iter().all(|c| word(c)) {
        return None;
    }
    Some(Admonition {
        kind: kind.to_owned(),
        classes: classes.into_iter().map(str::to_owned).collect(),
        title: title.map(str::to_owned),
        fold,
    })
}

/// How a source line below a header relates to its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// Part of the body; the first `n` bytes (prefix and indentation) are stripped.
    Body(usize),
    /// Blank (possibly with container markers).
    Blank,
    /// Ends the body.
    End,
}

/// Classifies a body candidate `line` of a header with `prefix`.
fn classify(prefix: &str, line: &str) -> Line {
    let n = repeated_prefix(prefix, line);
    let rest = &line[n..];
    if rest.trim().is_empty() {
        Line::Blank
    } else if n < prefix.len() {
        Line::End
    } else if rest.starts_with("    ") {
        Line::Body(n + 4)
    } else if rest.starts_with('\t') {
        Line::Body(n + 1)
    } else {
        Line::End
    }
}

/// Text assembled from pieces of the source, with the offsets to map it back.
#[derive(Debug, Default)]
struct Mapped {
    text: String,
    /// `(text offset, source offset)` of each piece, in order; the first at 0.
    pieces: Vec<(usize, usize)>,
}

impl Mapped {
    /// Appends `src[start..end]`.
    fn push(&mut self, src: &str, start: usize, end: usize) {
        self.pieces.push((self.text.len(), start));
        self.text.push_str(&src[start..end]);
    }

    /// The source span of the text span `span`. An end offset maps into the piece it
    /// ends, not the one starting there.
    fn map(&self, span: Span) -> Span {
        let locate = |offset: usize, end: bool| {
            let index = if end && offset > 0 {
                self.pieces.partition_point(|&(at, _)| at < offset)
            } else {
                self.pieces.partition_point(|&(at, _)| at <= offset)
            };
            let (at, start) = self.pieces[index - 1];
            start + offset - at
        };
        let start = locate(span.start, span.is_empty());
        Span::new(start, locate(span.end, !span.is_empty()).max(start))
    }
}

/// Replaces the header at `at` in `paragraph` and its body by an admonition. Returns
/// the paragraphs of the re-parsed rest, if any, to search for headers.
fn convert(
    ast: &mut Ast,
    cx: &mut PassContext<'_>,
    paragraph: NodeId,
    at: usize,
    admonition: Admonition,
) -> Vec<NodeId> {
    let src = cx.src();
    let container = ast
        .parent(paragraph)
        .expect("an attached paragraph has a parent");
    let limit = ast.span(container).end;
    let line_start = src[..at].rfind('\n').map_or(0, |nl| nl + 1);
    let prefix = &src[line_start..at];
    let (header, mut pos) = line_at(src, at);
    let mut end = (at + header.trim_end().len()).min(limit);

    // Read the body lines.
    let mut body = Mapped::default();
    // Pieces and text length up to the last non-blank line.
    let mut kept = (0, 0);
    while pos < src.len() && pos < limit {
        let (line, next) = line_at(src, pos);
        // Nothing past the container's end belongs to the body.
        let clamp = |offset: usize| offset.min(limit);
        match classify(prefix, line) {
            Line::End => break,
            Line::Blank => body.push(src, clamp(pos + line.len()), clamp(next)),
            Line::Body(strip) => {
                body.push(src, clamp(pos + strip), clamp(next));
                kept = (body.pieces.len(), body.text.len());
                end = clamp(pos + line.trim_end().len());
            }
        }
        pos = next;
    }
    body.pieces.truncate(kept.0);
    body.text.truncate(kept.1);
    let region_end = pos;

    let node = ast.create(NodeKind::Admonition(admonition), Span::new(at, end));
    if !body.pieces.is_empty() {
        let nested = cx.parse_mapped(&body.text, |span| body.map(span));
        for &child in nested.children(nested.root()) {
            let copy = ast.import(&nested, child, |span| span);
            end = end.max(ast.span(copy).end);
            ast.append_child(node, copy)
                .expect("an imported node is detached");
        }
        ast.set_span(node, Span::new(at, end));
    }

    // Replace the blocks the admonition covers.
    let index = ast
        .index_in_parent(paragraph)
        .expect("an attached paragraph has a parent");
    let covered: Vec<NodeId> = ast.children(container)[index + 1..]
        .iter()
        .copied()
        .take_while(|&id| ast.span(id).start < region_end)
        .collect();
    let last_end = covered
        .iter()
        .chain([&paragraph])
        .map(|&id| ast.span(id).end)
        .max()
        .expect("the paragraph is covered");
    if truncate_before(ast, paragraph, at) {
        ast.insert_after(paragraph, node)
            .expect("an attached paragraph has a parent");
    } else {
        ast.replace(paragraph, node)
            .expect("an attached paragraph has a parent");
    }
    for id in covered {
        ast.detach(id).expect("a child is not the root");
    }

    // Re-parse what the covered blocks hold past the body.
    if last_end <= region_end {
        return Vec::new();
    }
    let mut rest = Mapped::default();
    let mut pos = region_end;
    while pos < last_end {
        let (line, next) = line_at(src, pos);
        let end = next.min(last_end);
        rest.push(src, (pos + repeated_prefix(prefix, line)).min(end), end);
        pos = next;
    }
    let parsed = cx.flavor().parser().parse(&rest.text);
    let mut anchor = node;
    let mut reparsed = Vec::new();
    for &child in parsed.children(parsed.root()) {
        let copy = ast.import(&parsed, child, |span| rest.map(span));
        ast.insert_after(anchor, copy)
            .expect("the admonition is attached");
        anchor = copy;
        reparsed.extend(paragraphs(ast, copy));
    }
    reparsed
}

/// Cuts `paragraph` down to the children before the line break that precedes `at`
/// (and any breaks before that, as a hard break leaves). Returns `false`, leaving it
/// alone, if nothing would be left.
fn truncate_before(ast: &mut Ast, paragraph: NodeId, at: usize) -> bool {
    let children = ast.children(paragraph);
    let first = children
        .iter()
        .position(|&id| ast.span(id).start >= at)
        .unwrap_or(0);
    let keep = children[..first]
        .iter()
        .rposition(|&id| !matches!(ast.kind(id), NodeKind::SoftBreak | NodeKind::LineBreak))
        .map_or(0, |last| last + 1);
    if keep == 0 {
        return false;
    }
    let mut children = ast.take_children(paragraph);
    children.truncate(keep);
    let span = ast.span(paragraph);
    ast.set_span(
        paragraph,
        Span::new(span.start, ast.span(children[keep - 1]).end),
    );
    for child in children {
        ast.append_child(paragraph, child)
            .expect("taken children are detached");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_parse() {
        let header = |line| parse_header(line).map(|a| (a.kind, a.classes, a.title, a.fold));
        assert_eq!(
            header("???+ a b-c \"T \"x\"\" "),
            Some((
                "a".into(),
                vec!["b-c".into()],
                Some("T \"x\"".into()),
                Some(Fold::Open)
            ))
        );
        assert_eq!(header("!!!note"), Some(("note".into(), vec![], None, None)));
        assert_eq!(header("!!! a\"t\""), None);
        assert_eq!(header("!!! a \""), None);
        assert_eq!(header("!!! \"t\""), None);
        assert_eq!(header("!!!"), None);
        assert_eq!(header("!! a"), None);
    }

    #[test]
    fn lines_classify_against_the_prefix() {
        assert_eq!(classify("", "    x"), Line::Body(4));
        assert_eq!(classify("", "\tx"), Line::Body(1));
        assert_eq!(classify("", "   x"), Line::End);
        assert_eq!(classify("", "  "), Line::Blank);
        assert_eq!(classify("- ", "      x"), Line::Body(6));
        assert_eq!(classify("- ", "- x"), Line::End);
        assert_eq!(classify("> ", ">     x"), Line::Body(6));
        assert_eq!(classify("> ", ">"), Line::Blank);
        assert_eq!(classify("> ", "      x"), Line::End);
    }

    #[test]
    fn mapped_text_maps_spans_back() {
        let src = "ab\n    cd\n\n    ef\n";
        let mut mapped = Mapped::default();
        mapped.push(src, 7, 10);
        mapped.push(src, 10, 11);
        mapped.push(src, 15, 18);
        assert_eq!(mapped.text, "cd\n\nef\n");
        let map = |a, b| {
            let span = mapped.map(Span::new(a, b));
            &src[span.start..span.end]
        };
        assert_eq!(map(0, 2), "cd");
        assert_eq!(map(0, 6), "cd\n\n    ef");
        assert_eq!(map(4, 6), "ef");
        assert_eq!(map(3, 3), "");
    }
}
