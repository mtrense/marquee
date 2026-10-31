//! The built-in rules, one module per rule, plus helpers they share.
//!
//! Rules follow markdownlint 0.42 (ids, aliases, tags, option names and defaults,
//! messages and fixes). markdownlint has dropped `MD002`, `MD006`, `MD008`, `MD015`,
//! `MD016`, `MD017` and `MD057` (deprecated or never released), so mrq has no rules with
//! those ids; every other id up to `MD059` exists (`MD060` is beyond mrq's range).
//!
//! mrq's own rules, `MRQ001`–`MRQ005`, use mrq's model where markdownlint has none: the
//! flavor's anchors, frontmatter values and `mrq:` directives. Their messages follow
//! `mrq validate`'s style; `docs/rules/` documents them.

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
mod md021;
mod md022;
mod md023;
mod md024;
mod md025;
mod md026;
mod md027;
mod md028;
mod md029;
mod md030;
mod md031;
mod md032;
mod md033;
mod md034;
mod md035;
mod md036;
mod md037;
mod md038;
mod md039;
mod md040;
mod md041;
mod md042;
mod md043;
mod md044;
mod md045;
mod md046;
mod md047;
mod md048;
mod md049;
mod md051;
mod md052;
mod md053;
mod md054;
mod md055;
mod md056;
mod md058;
mod md059;
mod mrq001;
mod mrq002;
mod mrq003;
mod mrq004;
mod mrq005;

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
pub use md021::Md021;
pub use md022::{BlankLines, Md022, Md022Options};
pub use md023::Md023;
pub use md024::{Md024, Md024Options};
pub use md025::{Md025, Md025Options};
pub use md026::{Md026, Md026Options};
pub use md027::{Md027, Md027Options};
pub use md028::Md028;
pub use md029::{Md029, Md029Options, OrderedListStyle};
pub use md030::{Md030, Md030Options};
pub use md031::{Md031, Md031Options};
pub use md032::Md032;
pub use md033::{Md033, Md033Options};
pub use md034::Md034;
pub use md035::{Md035, Md035Options};
pub use md036::{Md036, Md036Options};
pub use md037::Md037;
pub use md038::Md038;
pub use md039::Md039;
pub use md040::{Md040, Md040Options};
pub use md041::{Md041, Md041Options};
pub use md042::Md042;
pub use md043::{Md043, Md043Options};
pub use md044::{Md044, Md044Options};
pub use md045::Md045;
pub use md046::{CodeBlockStyle, Md046, Md046Options};
pub use md047::Md047;
pub use md048::{FenceStyle, Md048, Md048Options};
pub use md049::{EmphasisStyle, Md049, Md049Options, Md050, Md050Options};
pub use md051::{Md051, Md051Options};
pub use md052::{Md052, Md052Options};
pub use md053::{Md053, Md053Options};
pub use md054::{Md054, Md054Options};
pub use md055::{Md055, Md055Options, PipeStyle};
pub use md056::Md056;
pub use md058::Md058;
pub use md059::{Md059, Md059Options};
pub use mrq001::{Mrq001, Mrq001Options};
pub use mrq002::{Mrq002, Mrq002Options};
pub use mrq003::{Mrq003, Mrq003Options};
pub use mrq004::Mrq004;
pub use mrq005::Mrq005;

use std::sync::LazyLock;

use regex::Regex;

use crate::Document;
use crate::ast::{Ast, LinkForm, NodeId, NodeKind, ReferenceStyle, Span};
use crate::source::{ByteRange, PositionEncoding, TextEdit};
use crate::validate::reference_definitions;

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
    registry.register(Md021);
    registry.register(Md022);
    registry.register(Md023);
    registry.register(Md024);
    registry.register(Md025);
    registry.register(Md026);
    registry.register(Md027);
    registry.register(Md028);
    registry.register(Md029);
    registry.register(Md030);
    registry.register(Md031);
    registry.register(Md032);
    registry.register(Md033);
    registry.register(Md034);
    registry.register(Md035);
    registry.register(Md036);
    registry.register(Md037);
    registry.register(Md038);
    registry.register(Md039);
    registry.register(Md040);
    registry.register(Md041);
    registry.register(Md042);
    registry.register(Md043);
    registry.register(Md044);
    registry.register(Md045);
    registry.register(Md046);
    registry.register(Md047);
    registry.register(Md048);
    registry.register(Md049);
    registry.register(Md050);
    registry.register(Md051);
    registry.register(Md052);
    registry.register(Md053);
    registry.register(Md054);
    registry.register(Md055);
    registry.register(Md056);
    registry.register(Md058);
    registry.register(Md059);
    registry.register(Mrq001);
    registry.register(Mrq002);
    registry.register(Mrq003);
    registry.register(Mrq004);
    registry.register(Mrq005);
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

/// What a finding about `span` covers: all of it if it fits on one line, else up to the
/// end of its first line (markdownlint then reports only the line).
pub(crate) fn first_line_span(doc: &Document, span: Span) -> ByteRange {
    let (first, last) = span_lines(doc, span);
    let end = if first == last {
        span.end
    } else {
        doc.source()
            .line_range(first)
            .map_or(span.end, |line| line.end)
    };
    ByteRange::new(span.start, end)
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

/// The lines of `doc` (without line endings) as markdownlint's rules see them: with the
/// content of HTML comments replaced by `.` (markdownlint's `clearHtmlCommentText`),
/// which shows in contexts.
pub(crate) fn cleared_lines(doc: &Document) -> Vec<String> {
    clear_html_comments(doc.text())
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect()
}

/// markdownlint's `clearHtmlCommentText`: every character of a comment's content but
/// spaces and line endings (trailing spaces included) as `.`. Inline comments that
/// markdownlint does not take for comments (starting with `>` or `->`, ending with `-`,
/// holding `--`, or spanning table cells) stay as they are, and so does everything from
/// an unterminated `<!--` on.
fn clear_html_comments(text: &str) -> String {
    let mut text = text.to_owned();
    let mut from = 0;
    while let Some(found) = text.get(from..).and_then(|rest| rest.find("<!--")) {
        let start = from + found;
        let Some(close) = text[start + 2..].find("-->") else {
            break;
        };
        let mut end = start + 2 + close;
        if end > start + 4 {
            let content = &text[start + 4..end];
            let line_start = text[..start].rfind('\n').map_or(0, |newline| newline + 1);
            let before = &text[line_start..start];
            let spans_cells =
                before.trim_start_matches(' ').starts_with('|') && content.contains('\n');
            let valid = before.trim().is_empty()
                || !(spans_cells
                    || content.starts_with('>')
                    || content.starts_with("->")
                    || content.ends_with('-')
                    || content.contains("--"));
            if valid {
                let cleared = clear_comment_content(content);
                text.replace_range(start + 4..end, &cleared);
                end = start + 4 + cleared.len();
            }
        }
        from = end + 3;
    }
    text
}

/// Comment content with everything but line endings and the spaces before other
/// characters as `.`.
fn clear_comment_content(content: &str) -> String {
    let mut cleared = String::with_capacity(content.len());
    let mut spaces = 0;
    for c in content.chars() {
        match c {
            ' ' => spaces += 1,
            '\r' | '\n' => {
                cleared.extend(std::iter::repeat_n('.', spaces));
                spaces = 0;
                cleared.push(c);
            }
            _ => {
                cleared.extend(std::iter::repeat_n(' ', spaces));
                spaces = 0;
                cleared.push('.');
            }
        }
    }
    cleared.extend(std::iter::repeat_n(' ', spaces));
    cleared
}

/// `text` with each line ending as a space, as markdownlint-cli2 shows contexts.
pub(crate) fn one_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n'], " ")
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

/// The text of body line `line` (see [`body_lines`]) without its line ending, or `None`
/// for front matter lines and lines past the end, which markdownlint's `lines` lack.
pub(crate) fn body_line(doc: &Document, line: usize) -> Option<&str> {
    if !body_lines(doc).contains(&line) {
        return None;
    }
    let range = doc.source().line_range(line)?;
    Some(&doc.text()[range.start..range.end])
}

/// markdownlint's `isBlankLine`: missing, whitespace only, or nothing but HTML comments
/// (an unclosed one at either end included) and `>` markers.
pub(crate) fn is_blank_line(line: Option<&str>) -> bool {
    let Some(line) = line else {
        return true;
    };
    let mut rest = line.to_owned();
    loop {
        let start = rest.find("<!--");
        let end = rest.find("-->");
        rest = match (start, end) {
            // An unmatched end first, a comment, an unmatched start last.
            (None, Some(end)) => rest[end + 3..].to_owned(),
            (Some(start), Some(end)) if end < start => rest[end + 3..].to_owned(),
            (Some(start), Some(end)) => format!("{}{}", &rest[..start], &rest[end + 3..]),
            (Some(start), None) => rest[..start].to_owned(),
            (None, None) => break,
        };
    }
    rest.replace('>', "").trim().is_empty()
}

/// Whether `id` is a container whose prefixes start the lines inside it.
fn is_container(ast: &Ast, id: NodeId) -> bool {
    matches!(
        ast.kind(id),
        NodeKind::BlockQuote | NodeKind::Callout(_) | NodeKind::ListItem(_)
    )
}

/// For each line, the innermost block quote or list item that covers it.
pub(crate) fn line_containers(doc: &Document) -> Vec<Option<NodeId>> {
    let ast = doc.ast();
    let mut containers = vec![None; doc.source().line_count()];
    // Pre-order, so inner containers overwrite outer ones.
    for id in ast.descendants(ast.root()) {
        if is_container(ast, id) {
            let (first, last) = span_lines(doc, ast.span(id));
            for line in containers.iter_mut().take(last + 1).skip(first) {
                *line = Some(id);
            }
        }
    }
    containers
}

/// How the container prefixes of a line split it, as micromark tokenizes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinePrefix {
    /// Where the prefixes end: after the last `>` marker (and its space), list marker
    /// (and its spaces) or list item indentation.
    pub end: usize,
    /// The `>` markers with their whitespace, trimmed at the end: markdownlint's
    /// `getBlockQuotePrefixText`, which fixes insert before new lines.
    pub quote: String,
    /// Whitespace right after a `>` marker (and its space) that is part of no prefix:
    /// micromark's `linePrefix` after a `blockQuotePrefix`.
    pub extra: Option<ByteRange>,
}

/// Splits the prefixes off line `line`, whose content sits inside `node` and its
/// ancestors: block quotes take their `>` and one space or tab (a line without `>` is
/// lazy and ends the prefixes), list items their marker and spaces on their first line
/// and their content indentation on later lines.
pub(crate) fn line_prefix(doc: &Document, line: usize, node: Option<NodeId>) -> LinePrefix {
    let ast = doc.ast();
    let text = doc.text();
    let range = doc
        .source()
        .line_range(line)
        .unwrap_or(ByteRange::empty(text.len()));
    let mut containers: Vec<NodeId> = node
        .into_iter()
        .flat_map(|node| std::iter::once(node).chain(ast.ancestors(node)))
        .filter(|&id| is_container(ast, id))
        .collect();
    containers.reverse();
    let mut pos = range.start;
    let mut quote = String::new();
    let mut extra = None;
    let mut after_quote = false;
    for container in containers {
        if let NodeKind::ListItem(_) = ast.kind(container) {
            let prefix = md005::ItemPrefix::of(doc, container);
            if prefix.line_start == range.start {
                let start = prefix.marker.start;
                let gap = text.get(pos..start).unwrap_or_default();
                if after_quote && !gap.is_empty() && gap.trim_matches([' ', '\t']).is_empty() {
                    extra = Some(ByteRange::new(pos, start));
                    quote.push_str(gap);
                }
                pos = prefix.end.max(pos);
            } else {
                let indent = js_len(&text[prefix.line_start..prefix.end]);
                while js_len(&text[range.start..pos]) < indent
                    && text[pos..range.end].starts_with([' ', '\t'])
                {
                    pos += 1;
                }
            }
            after_quote = false;
        } else {
            let rest = &text[pos..range.end];
            let indent = rest.len() - rest.trim_start_matches(' ').len();
            if indent > 3 || !rest[indent..].starts_with('>') {
                break;
            }
            let space = usize::from(rest[indent + 1..].starts_with([' ', '\t']));
            let end = pos + indent + 1 + space;
            quote.push_str(&text[pos..end]);
            pos = end;
            after_quote = true;
        }
    }
    if after_quote {
        let rest = &text[pos..range.end];
        let space = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        if space > 0 && space < rest.len() {
            extra = Some(ByteRange::new(pos, pos + space));
        }
    }
    LinePrefix {
        end: pos,
        quote: quote.trim_end().to_owned(),
        extra,
    }
}

/// The parts of raw HTML in `CommonMark` inline content: comments, processing
/// instructions, declarations, CDATA, closing tags and opening tags.
static RAW_HTML: LazyLock<Regex> = LazyLock::new(|| {
    let attribute = r#"\s+[A-Za-z_:][A-Za-z0-9_.:-]*(?:\s*=\s*(?:[^\s"'=<>`]+|'[^']*'|"[^"]*"))?"#;
    Regex::new(&format!(
        r"(?s)^(?:<!-->|<!--->|<!--.*?-->|<\?.*?\?>|<![A-Za-z][^>]*>|<!\[CDATA\[.*?\]\]>|</[A-Za-z][A-Za-z0-9-]*\s*>|<[A-Za-z][A-Za-z0-9-]*(?:{attribute})*\s*/?>)"
    ))
    .expect("valid regex")
});

/// The length of the raw HTML (tag, comment, …) `text` starts with, if any.
pub(crate) fn raw_html(text: &str) -> Option<usize> {
    RAW_HTML.find(text).map(|html| html.end())
}

/// A character reference (`&amp;`, `&#123;`, `&#x1F;`), which markdownlint leaves out
/// of the text it compares.
pub(crate) static CHARACTER_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(?:#[xX][0-9A-Fa-f]{1,6}|#[0-9]{1,7}|[A-Za-z][A-Za-z0-9]{0,31});")
        .expect("valid regex")
});

/// markdownlint's `htmlText` tokens: every inline HTML node, and every piece of raw HTML
/// in an HTML block (whose text markdownlint parses again as inline content), with the
/// node it is in.
pub(crate) fn html_pieces(doc: &Document) -> Vec<(ByteRange, NodeId)> {
    let ast = doc.ast();
    let text = doc.text();
    let mut pieces = Vec::new();
    for id in ast.descendants(ast.root()) {
        let span = ast.span(id);
        match ast.kind(id) {
            NodeKind::HtmlInline(_) => pieces.push((ByteRange::new(span.start, span.end), id)),
            NodeKind::HtmlBlock(_) => {
                let block = &text[span.start..span.end];
                let mut pos = 0;
                while let Some(found) = block[pos..].find('<') {
                    let start = pos + found;
                    match raw_html(&block[start..]) {
                        Some(len) => {
                            let at = span.start + start;
                            pieces.push((ByteRange::new(at, at + len), id));
                            pos = start + len;
                        }
                        None => pos = start + 1,
                    }
                }
            }
            _ => {}
        }
    }
    pieces
}

/// markdownlint's `getHtmlTagInfo` name pattern.
static TAG_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^<([^!>][^/\s>]*)").expect("valid regex"));

/// markdownlint's `getHtmlTagInfo`: the element name of a piece of raw HTML and whether
/// it is a closing tag, or `None` for comments and the like.
pub(crate) fn html_tag_name(html: &str) -> Option<(&str, bool)> {
    let name = TAG_NAME.captures(html)?.get(1)?.as_str();
    Some(match name.strip_prefix('/') {
        Some(name) => (name, true),
        None => (name, false),
    })
}

/// markdownlint's `getHtmlAttributeRe`: a pattern for attribute `name` (ignoring case)
/// whose group 1 is the value, unquoted, up to the first quote, space or `>`; see
/// [`html_attribute`].
pub(crate) fn html_attribute_pattern(name: &str) -> Regex {
    let pattern = format!(r#"(?i)\s{}\s*=\s*['"]?([^'"\s>]*)"#, regex::escape(name));
    Regex::new(&pattern).expect("valid regex")
}

/// The value of the attribute `pattern` (from [`html_attribute_pattern`]) is for.
pub(crate) fn html_attribute<'a>(html: &'a str, pattern: &Regex) -> Option<&'a str> {
    Some(pattern.captures(html)?.get(1)?.as_str())
}

/// markdownlint's `isHtmlFlowComment`: whether an HTML block is a single comment.
pub(crate) fn is_html_comment(html: &str) -> bool {
    let html = html.trim_end_matches(['\r', '\n']);
    html.strip_prefix("<!--")
        .and_then(|rest| rest.strip_suffix("-->"))
        .is_some_and(|comment| {
            !comment.starts_with('>') && !comment.starts_with("->") && !comment.ends_with('-')
        })
}

/// markdownlint's `getHeadingText`: the source of a heading's content without its
/// inline HTML (outside other inlines), line endings as spaces, trimmed.
pub(crate) fn heading_text(doc: &Document, heading: NodeId) -> String {
    let ast = doc.ast();
    let text = doc.text();
    let children = ast.children(heading);
    let (Some(&first), Some(&last)) = (children.first(), children.last()) else {
        return String::new();
    };
    let mut content = String::new();
    let mut pos = ast.span(first).start;
    for &child in children {
        if matches!(
            ast.kind(child),
            NodeKind::HtmlInline(_) | NodeKind::Comment(_)
        ) {
            let span = ast.span(child);
            content.push_str(&text[pos..span.start.max(pos)]);
            pos = span.end.max(pos);
        }
    }
    content.push_str(&text[pos..ast.span(last).end.max(pos)]);
    content
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_owned()
}

/// The parts of a link or image as written, as byte offsets into the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkSyntax {
    /// The text between `[` (after `!` for images) and `]`.
    pub text: ByteRange,
    /// Whether the link is inline, `[text](…)`.
    pub inline: bool,
    /// Inline links: the destination (inside `<…>`), if not empty.
    pub destination: Option<ByteRange>,
    /// Inline links: whether a title follows the destination.
    pub title: bool,
    /// Full references: the label in the second brackets; collapsed ones: an empty
    /// range; shortcuts and inline links: `None`.
    pub reference: Option<ByteRange>,
}

/// Splits the source of link or image `id` (not an autolink) into its parts.
pub(crate) fn link_syntax(doc: &Document, id: NodeId) -> Option<LinkSyntax> {
    let ast = doc.ast();
    let text = doc.text();
    let span = ast.span(id);
    let (NodeKind::Link(link) | NodeKind::Image(link)) = ast.kind(id) else {
        return None;
    };
    let open = span.start + usize::from(matches!(ast.kind(id), NodeKind::Image(_))) + 1;
    if !text[..open.min(text.len())].ends_with('[') {
        return None;
    }
    // The text ends at the first `]` after the children.
    let after_children = ast
        .children(id)
        .last()
        .map_or(open, |&last| ast.span(last).end.max(open));
    let close = after_children + text[after_children..span.end].find(']')?;
    let mut syntax = LinkSyntax {
        text: ByteRange::new(open, close),
        inline: false,
        destination: None,
        title: false,
        reference: None,
    };
    let rest = &text[close + 1..span.end];
    match &link.form {
        LinkForm::Inline if rest.starts_with('(') => {
            syntax.inline = true;
            let inner = &rest[1..];
            let start = close + 2 + (inner.len() - inner.trim_start().len());
            let after = &text[start..span.end];
            let (destination, end) = if let Some(literal) = after.strip_prefix('<') {
                let len = literal.find('>')?;
                (ByteRange::new(start + 1, start + 1 + len), start + len + 2)
            } else {
                let len = raw_destination_len(after);
                (ByteRange::new(start, start + len), start + len)
            };
            if !destination.is_empty() {
                syntax.destination = Some(destination);
            }
            let tail = text[end..span.end].trim_start();
            syntax.title = tail.starts_with(['"', '\'', '(']);
        }
        LinkForm::Reference { style, .. } if *style != ReferenceStyle::Shortcut => {
            let inner = rest.strip_prefix('[')?;
            let len = inner.find(']')?;
            syntax.reference = Some(ByteRange::new(close + 2, close + 2 + len));
        }
        _ => {}
    }
    Some(syntax)
}

/// The length of the raw (not `<…>`) link destination `text` starts with: up to
/// whitespace or an unbalanced `)`, skipping escapes.
fn raw_destination_len(text: &str) -> usize {
    let mut depth = 0usize;
    let mut chars = text.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '(' => depth += 1,
            ')' if depth == 0 => return at,
            ')' => depth -= 1,
            c if c.is_ascii_whitespace() || c.is_ascii_control() => return at,
            _ => {}
        }
    }
    text.len()
}

/// The definitions markdownlint's reference rules know, by normalized label (see
/// [`normalize_label`]): each label's first link reference definition (with its
/// destination as written) or footnote definition (`^label`, no destination), in
/// document order, then the later definitions of a label already defined.
pub(crate) struct Definitions {
    /// `(label, line, destination)` of each first definition.
    pub first: Vec<(String, usize, Option<String>)>,
    /// `(label, line)` of each later definition.
    pub duplicates: Vec<(String, usize)>,
}

impl Definitions {
    pub(crate) fn of(doc: &Document) -> Self {
        let ast = doc.ast();
        let mut all: Vec<(usize, String, Option<String>)> = reference_definitions(doc)
            .into_iter()
            .map(|definition| {
                (
                    definition.span.start,
                    normalize_label(&definition.label),
                    Some(definition.url),
                )
            })
            .collect();
        for id in ast.descendants(ast.root()) {
            if let NodeKind::FootnoteDefinition(footnote) = ast.kind(id) {
                let label = normalize_label(&format!("^{}", footnote.label));
                all.push((ast.span(id).start, label, None));
            }
        }
        all.sort_by_key(|(start, _, _)| *start);
        let mut definitions = Self {
            first: Vec::new(),
            duplicates: Vec::new(),
        };
        for (start, label, destination) in all {
            let line = line_of(doc, start);
            if definitions.get(&label).is_some() {
                definitions.duplicates.push((label, line));
            } else {
                definitions.first.push((label, line, destination));
            }
        }
        definitions
    }

    /// The destination of label `label` (looked up as given, not normalized, like
    /// markdownlint): `Some("")` for footnotes.
    pub(crate) fn get(&self, label: &str) -> Option<&str> {
        self.first
            .iter()
            .find(|(first, _, _)| first == label)
            .map(|(_, _, destination)| destination.as_deref().unwrap_or_default())
    }
}

/// One line of a table as written (the delimiter row included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableLine {
    /// The 0-based line.
    pub line: usize,
    /// The row without container prefixes and surrounding whitespace.
    pub content: ByteRange,
    /// The offset of every unescaped `|` in it, which all divide cells (in code spans
    /// too, as in GFM).
    pub pipes: Vec<usize>,
}

impl TableLine {
    /// Whether the row starts with a pipe.
    pub(crate) fn leading_pipe(&self) -> bool {
        self.pipes.first() == Some(&self.content.start)
    }

    /// Whether the row ends with a pipe (not the leading one).
    pub(crate) fn trailing_pipe(&self) -> bool {
        self.pipes.last() == Some(&(self.content.end.saturating_sub(1)))
            && !(self.pipes.len() == 1 && self.leading_pipe())
    }
}

/// The lines of table `table`.
pub(crate) fn table_lines(doc: &Document, table: NodeId) -> Vec<TableLine> {
    let text = doc.text();
    let containers = line_containers(doc);
    let (first, last) = span_lines(doc, doc.ast().span(table));
    let mut lines = Vec::new();
    for line in first..=last {
        let Some(range) = doc.source().line_range(line) else {
            continue;
        };
        let start = if line == first {
            doc.ast().span(table).start
        } else {
            line_prefix(doc, line, containers.get(line).copied().flatten()).end
        };
        let row = &text[start..range.end];
        let content_start = start + (row.len() - row.trim_start().len());
        let content_end = start + row.trim_end().len();
        let bytes = text.as_bytes();
        let pipes = (content_start..content_end)
            .filter(|&at| {
                bytes[at] == b'|'
                    && bytes[content_start..at]
                        .iter()
                        .rev()
                        .take_while(|&&b| b == b'\\')
                        .count()
                        % 2
                        == 0
            })
            .collect();
        lines.push(TableLine {
            line,
            content: ByteRange::new(content_start, content_end.max(content_start)),
            pipes,
        });
    }
    lines
}

/// markdownlint's label normalization: lowercase, trimmed, whitespace runs as one space.
pub(crate) fn normalize_label(label: &str) -> String {
    label
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
