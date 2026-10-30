//! The built-in rules, one module per rule, plus helpers they share.
//!
//! Rules follow markdownlint 0.42 (ids, aliases, tags, option names and defaults,
//! messages and fixes). markdownlint has dropped `MD002`, `MD006`, `MD008`, `MD015`,
//! `MD016` and `MD017` (deprecated or never released), so mrq has no rules with those ids.

mod md001;
mod md003;
mod md004;
mod md005;
mod md007;
mod md009;
mod md010;
mod md011;
mod md012;
mod md013;
mod md014;
mod md018;
mod md019;
mod md020;
mod md047;

pub use md001::{Md001, Md001Options};
pub use md003::{HeadingStyle, Md003, Md003Options};
pub use md004::{ListStyle, Md004, Md004Options};
pub use md005::Md005;
pub use md007::{Md007, Md007Options};
pub use md009::{Md009, Md009Options};
pub use md010::{Md010, Md010Options};
pub use md011::Md011;
pub use md012::{Md012, Md012Options};
pub use md013::{Md013, Md013Options};
pub use md014::Md014;
pub use md018::Md018;
pub use md019::Md019;
pub use md020::Md020;
pub use md047::Md047;

use crate::Document;
use crate::ast::{Ast, NodeId, NodeKind, Span};
use crate::source::{ByteRange, PositionEncoding, TextEdit};

use super::{Registry, RuleMeta};

/// Adds every built-in rule to `registry`.
pub(super) fn register_builtin(registry: &mut Registry) {
    registry.register(Md001);
    registry.register(Md003);
    registry.register(Md004);
    registry.register(Md005);
    registry.register(Md007);
    registry.register(Md009);
    registry.register(Md010);
    registry.register(Md011);
    registry.register(Md012);
    registry.register(Md013);
    registry.register(Md014);
    registry.register(Md018);
    registry.register(Md019);
    registry.register(Md020);
    registry.register(Md047);
}

/// The line ending the document uses: its first `\r\n` or `\n`, else `\n`.
pub(crate) fn line_ending(doc: &Document) -> &'static str {
    match doc.text().find('\n') {
        Some(newline) if doc.text()[..newline].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

/// The 0-based line of `offset`.
pub(crate) fn line_of(doc: &Document, offset: usize) -> usize {
    doc.source()
        .offset_to_position(offset, PositionEncoding::Utf8)
        .map_or(0, |position| position.line)
}

/// The lines markdownlint's line-based rules look at: every line after the frontmatter.
pub(crate) fn body_lines(doc: &Document) -> std::ops::Range<usize> {
    let first = doc
        .frontmatter()
        .map_or(0, |frontmatter| line_of(doc, frontmatter.span().end) + 1);
    first..doc.source().line_count()
}

/// Each body line (see [`body_lines`]) with its range, without the line ending.
pub(crate) fn lines(doc: &Document) -> impl Iterator<Item = (usize, ByteRange)> + '_ {
    body_lines(doc).filter_map(|line| Some((line, doc.source().line_range(line)?)))
}

/// The first and last line that `span` touches.
pub(crate) fn span_lines(doc: &Document, span: Span) -> (usize, usize) {
    // A span ending right after a newline does not cover the next line.
    let last = doc
        .text()
        .floor_char_boundary(span.end.saturating_sub(1).max(span.start));
    (line_of(doc, span.start), line_of(doc, last))
}

/// Marks in `lines` every line that `span` touches.
pub(crate) fn mark_lines(doc: &Document, lines: &mut [bool], span: Span) {
    let (first, last) = span_lines(doc, span);
    for line in lines.iter_mut().take(last + 1).skip(first) {
        *line = true;
    }
}

/// For each line, whether a node for which `select` holds touches it.
pub(crate) fn node_lines(doc: &Document, select: impl Fn(&Ast, NodeId) -> bool) -> Vec<bool> {
    let mut lines = vec![false; doc.source().line_count()];
    let ast = doc.ast();
    for id in ast.descendants(ast.root()) {
        if select(ast, id) {
            mark_lines(doc, &mut lines, ast.span(id));
        }
    }
    lines
}

/// For each line of `doc`, whether markdownlint leaves it alone in line-based rules:
/// frontmatter lines and the lines of code blocks (fences included).
pub(crate) fn skipped_lines(doc: &Document) -> Vec<bool> {
    let mut skipped = node_lines(doc, |ast, id| {
        matches!(ast.kind(id), NodeKind::CodeBlock(_))
    });
    if let Some(frontmatter) = doc.frontmatter() {
        mark_lines(doc, &mut skipped, frontmatter.span());
    }
    skipped
}

/// The lines of code blocks, HTML blocks and math blocks, which markdownlint skips when
/// it looks for heading or link syntax in raw lines (micromark's `codeFenced`,
/// `codeIndented`, `htmlFlow` and `mathFlow`).
pub(crate) fn opaque_block_lines(doc: &Document) -> Vec<bool> {
    node_lines(doc, |ast, id| match ast.kind(id) {
        NodeKind::CodeBlock(_)
        | NodeKind::HtmlBlock(_)
        | NodeKind::IncludeDirective(_)
        | NodeKind::TocDirective(_)
        | NodeKind::DirectiveEnd => true,
        NodeKind::Comment(_) => ast
            .parent(id)
            .is_some_and(|parent| ast.kind(parent).is_block() && !is_inline_container(ast, parent)),
        NodeKind::Math(math) => math.display,
        _ => false,
    })
}

/// Whether `id` is a block whose children are inlines.
fn is_inline_container(ast: &Ast, id: NodeId) -> bool {
    matches!(
        ast.kind(id),
        NodeKind::Paragraph | NodeKind::Heading(_) | NodeKind::TableCell
    )
}

/// The range of `heading`'s first line from the heading's start: the whole ATX heading,
/// or the text line of a setext heading.
pub(crate) fn heading_line(doc: &Document, heading: NodeId) -> ByteRange {
    let start = doc.ast().span(heading).start;
    let end = doc
        .source()
        .line_range(line_of(doc, start))
        .map_or(start, |line| line.end);
    ByteRange::new(start, end)
}

/// The parts of an ATX heading line, as byte offsets into the line (which starts at
/// the opening `#`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Atx {
    /// The opening `#` sequence.
    pub open: ByteRange,
    /// The whitespace after it.
    pub open_space: ByteRange,
    /// The closing `#` sequence, if any, and the whitespace before it.
    pub close: Option<(ByteRange, ByteRange)>,
}

impl Atx {
    /// Splits `line`, which starts at the `#` of an ATX heading. A closing sequence is a
    /// trailing run of `#` (before trailing whitespace) after a space or tab.
    pub(crate) fn parse(line: &str) -> Self {
        let open_len = line.len() - line.trim_start_matches('#').len();
        let rest = &line[open_len..];
        let space_len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let body = line[open_len..].trim_end_matches([' ', '\t']);
        let close_len = body.len() - body.trim_end_matches('#').len();
        let before = &body[..body.len() - close_len];
        let close = (close_len > 0 && before.ends_with([' ', '\t'])).then(|| {
            let close_start = open_len + before.len();
            let space_start = open_len + before.trim_end_matches([' ', '\t']).len();
            (
                ByteRange::new(close_start, close_start + close_len),
                ByteRange::new(space_start, close_start),
            )
        });
        Self {
            open: ByteRange::new(0, open_len),
            open_space: ByteRange::new(open_len, open_len + space_len),
            close,
        }
    }
}

/// markdownlint's message: the rule description, then `[detail]` and
/// `[Context: "context"]` when given.
pub(crate) fn message(meta: &RuleMeta, detail: Option<&str>, context: Option<&str>) -> String {
    let mut message = meta.description.to_owned();
    if let Some(detail) = detail {
        message.push_str(" [");
        message.push_str(detail);
        message.push(']');
    }
    if let Some(context) = context {
        message.push_str(" [Context: \"");
        message.push_str(context);
        message.push_str("\"]");
    }
    message
}

/// `Expected: {expected}; Actual: {actual}`.
pub(crate) fn expected_actual(
    expected: impl std::fmt::Display,
    actual: impl std::fmt::Display,
) -> String {
    format!("Expected: {expected}; Actual: {actual}")
}

/// markdownlint's `ellipsify`: context longer than 30 characters keeps its start, its
/// end or both (`start` and `end` say which matter), with `...` for the rest.
pub(crate) fn ellipsify(text: &str, start: bool, end: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 30 {
        return text.to_owned();
    }
    let take = |range: std::ops::Range<usize>| chars[range].iter().collect::<String>();
    if start && end {
        format!("{}...{}", take(0..15), take(chars.len() - 15..chars.len()))
    } else if end {
        format!("...{}", take(chars.len() - 30..chars.len()))
    } else {
        format!("{}...", take(0..30))
    }
}

/// An edit replacing `range` with `replacement`.
pub(crate) fn edit(range: ByteRange, replacement: impl Into<String>) -> TextEdit {
    TextEdit {
        range,
        replacement: replacement.into(),
    }
}

/// Number of UTF-16 code units in `text`, markdownlint's (JavaScript's) string length.
pub(crate) fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}
