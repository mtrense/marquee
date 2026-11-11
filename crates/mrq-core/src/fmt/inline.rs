//! The inline printer and escaping: the inline children of a block become one string,
//! with `\n` where the block continues on the next line, or a [`Flow`] of words for
//! wrapping.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Range;
use std::sync::OnceLock;

use unicode_general_category::{GeneralCategory, get_general_category};

use super::Printer;
use super::refs::collapse_whitespace;
use super::wrap::Flow;
use crate::ast::{CommentSyntax, Link, LinkForm, NodeId, NodeKind, ReferenceStyle, WikiLink};
use crate::flavor::EscapeContext;
use crate::parse::ParseOptions;

impl Printer<'_> {
    /// The inline children of `parent` (a paragraph, heading or table cell). In a table
    /// cell every `|` is escaped, also inside code spans and links, as GFM requires.
    pub(super) fn inlines(&self, parent: NodeId, cell: bool) -> String {
        let mut writer = Inlines::new(self);
        writer.children(parent);
        if cell {
            writer.out.replace('|', "\\|")
        } else {
            writer.out
        }
    }

    /// An inline node that stands where a block is expected.
    pub(super) fn inline_block(&self, id: NodeId) -> String {
        let mut writer = Inlines::new(self);
        writer.node(id);
        writer.out
    }

    /// The inline children of the paragraph `parent` as a [`Flow`]: on one line except
    /// after hard breaks (and inside raw HTML, math or comments that span lines), with
    /// the whitespace where a line may break.
    pub(super) fn flow(&self, parent: NodeId) -> Flow {
        let mut writer = Inlines::new(self);
        writer.flow = Some(Flow::default());
        writer.children(parent);
        let mut flow = writer.flow.take().unwrap_or_default();
        flow.text = writer.out;
        // Alone on the first line, raw HTML would be an HTML block.
        if let Some(first) = flow.raw.first().filter(|raw| raw.start == 0) {
            let end = first.end;
            flow.breaks.retain(|gap| gap.start != end);
        }
        flow
    }
}

/// Writes inlines into `out`, which is also the context for escaping: whether a text is
/// at the start of a line, what it follows.
struct Inlines<'p, 'a> {
    printer: &'p Printer<'a>,
    out: String,
    /// The delimiter characters of the emphasis nodes being written, innermost last.
    delimiters: Vec<char>,
    /// When writing a flow: soft breaks are spaces, and the places a line may break
    /// are recorded here.
    flow: Option<Flow>,
}

/// Where a text node sits, for [`Inlines::escape`].
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)] // independent facts about the position
struct TextContext {
    /// It starts a line, where block markers count.
    line_start: bool,
    /// Leading whitespace would be stripped (start of a line, a block or emphasis).
    strip_start: bool,
    /// Trailing whitespace would be stripped.
    strip_end: bool,
    /// A line ends right after it (a break or the end of the block follows).
    line_end: bool,
    /// A hard break follows, printed as `\`.
    before_hard_break: bool,
    /// Inlines follow it in its parent that may print a `>` (closing raw HTML).
    followed: bool,
    /// It follows a `]`, so `(` or `:` would turn the bracket into a link.
    after_bracket: bool,
    /// A `[` follows, so a trailing `!` would make an image.
    before_bracket: bool,
    /// It is the first child of its parent.
    starts_parent: bool,
    /// It is the last child of a heading.
    ends_heading: bool,
}

impl<'p, 'a> Inlines<'p, 'a> {
    const fn new(printer: &'p Printer<'a>) -> Self {
        Self {
            printer,
            out: String::new(),
            delimiters: Vec::new(),
            flow: None,
        }
    }

    /// Records `range` of `out` as whitespace where a line may break, joined to a break
    /// right before it.
    fn breakable(&mut self, range: Range<usize>) {
        let Some(flow) = &mut self.flow else {
            return;
        };
        match flow.breaks.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => flow.breaks.push(range),
        }
    }

    /// Records the `\n` just written as the end of a hard break.
    fn hard_break(&mut self) {
        if let Some(flow) = &mut self.flow {
            flow.hard.push(self.out.len() - 1);
        }
    }

    /// Raw HTML or a comment, kept on the line of what comes before: at the start of a
    /// line it could start an HTML block. Where it does start a line (as it did in the
    /// source), it is never escaped; where it could start an HTML block there (so the
    /// source indented it by four columns or more), it is indented by four columns.
    fn raw(&mut self, html: &str) {
        if self.out.ends_with('\n') && starts_html_block(html) {
            self.out.push_str("    ");
        }
        let start = self.out.len();
        self.out.push_str(html);
        if let Some(flow) = &mut self.flow {
            if flow.breaks.last().is_some_and(|last| last.end == start) {
                flow.breaks.pop();
            }
            flow.raw.push(start..self.out.len());
        }
    }

    fn children(&mut self, parent: NodeId) {
        for &child in self.printer.ast.children(parent) {
            self.node(child);
        }
    }

    fn node(&mut self, id: NodeId) {
        let ast = self.printer.ast;
        if self.printer.is_verbatim(id) {
            let span = ast.span(id);
            let source = &self.printer.src[span.start..span.end];
            let text = self.printer.unprefixed(source, self.block_start(id));
            self.raw(&text);
            return;
        }
        match ast.kind(id) {
            NodeKind::Text(text) => {
                let context = self.text_context(id);
                let escaped = self.escape(text, context);
                let start = self.out.len();
                self.out.push_str(&escaped);
                // Whitespace left as it is (not a character reference) may break.
                let mut at = 0;
                while let Some(offset) = escaped[at..].find([' ', '\t']) {
                    let begin = at + offset;
                    let end = escaped[begin..]
                        .find(|c| c != ' ' && c != '\t')
                        .map_or(escaped.len(), |len| begin + len);
                    self.breakable(start + begin..start + end);
                    at = end;
                }
            }
            // Before a hard break, a space would be stripped (and `&#32;` would join a
            // bare URL), so the line break stays.
            NodeKind::SoftBreak
                if self.flow.is_some()
                    && !ast
                        .next_sibling(id)
                        .is_some_and(|next| matches!(ast.kind(next), NodeKind::LineBreak)) =>
            {
                self.out.push(' ');
                self.breakable(self.out.len() - 1..self.out.len());
            }
            NodeKind::SoftBreak => self.out.push('\n'),
            // A `\` right after a bare URL would be part of it.
            NodeKind::LineBreak if ast.prev_sibling(id).is_some_and(|prev| {
                matches!(ast.kind(prev), NodeKind::Link(link) if link.form == LinkForm::Bare)
            }) =>
            {
                self.out.push_str("  \n");
                self.hard_break();
            }
            NodeKind::LineBreak => {
                self.out.push_str("\\\n");
                self.hard_break();
            }
            NodeKind::Code(code) => self.out.push_str(&code_span(code)),
            NodeKind::Emphasis => self.emphasis(id, false),
            NodeKind::Strong => self.emphasis(id, true),
            NodeKind::Strikethrough => self.wrap(id, "~~"),
            NodeKind::Highlight => self.wrap(id, "=="),
            NodeKind::Superscript => self.wrap(id, "^"),
            NodeKind::Subscript => self.wrap(id, "~"),
            NodeKind::Link(link) => self.link(id, link),
            NodeKind::Image(link) => {
                self.out.push('!');
                self.link(id, link);
            }
            NodeKind::HtmlInline(html) => self.raw(html),
            NodeKind::FootnoteReference(reference) => {
                self.out.push_str("[^");
                self.out.push_str(&reference.label);
                self.out.push(']');
            }
            NodeKind::WikiLink(link) => self.out.push_str(&wikilink(link)),
            NodeKind::Embed(link) => {
                self.out.push('!');
                self.out.push_str(&wikilink(link));
            }
            NodeKind::Tag(tag) => {
                self.out.push('#');
                self.out.push_str(&tag.name);
            }
            NodeKind::Math(math) => {
                // mkdocs' `\(…\)` and `\[…\]` keep their form: their literal is the
                // source between the delimiters, which `$` may not delimit.
                let span = ast.span(id);
                let source = self.printer.src.get(span.start..span.start + 2);
                let (open, close) = match source.filter(|_| self.printer.backslash_math) {
                    Some("\\(") => ("\\(", "\\)"),
                    Some("\\[") => ("\\[", "\\]"),
                    _ if math.display => ("$$", "$$"),
                    _ => ("$", "$"),
                };
                let literal = if open.starts_with('\\') {
                    self.printer.unprefixed(&math.literal, self.block_start(id))
                } else {
                    math.literal.clone()
                };
                self.out.push_str(open);
                self.out.push_str(&literal);
                self.out.push_str(close);
            }
            NodeKind::Comment(comment) if comment.syntax == CommentSyntax::Percent => {
                // The literal is the source between the delimiters, prefixes included.
                let literal = self.printer.unprefixed(&comment.literal, self.block_start(id));
                self.raw(&format!("%%{literal}%%"));
            }
            NodeKind::Comment(comment) => self.raw(&format!("<!--{}-->", comment.literal)),
            // Blocks never appear among inlines.
            _ => {}
        }
    }

    /// The start of the block whose inlines `id` is among.
    fn block_start(&self, id: NodeId) -> usize {
        let ast = self.printer.ast;
        let block = std::iter::successors(ast.parent(id), |&p| ast.parent(p))
            .find(|&p| ast.kind(p).is_block())
            .unwrap_or(id);
        ast.span(block).start
    }

    /// `delimiter`, the children, `delimiter`.
    fn wrap(&mut self, id: NodeId, delimiter: &str) {
        self.out.push_str(delimiter);
        self.children(id);
        self.out.push_str(delimiter);
    }

    /// Emphasis or strong emphasis with the configured character, unless that would
    /// touch an adjacent delimiter of the same character (`*a**b*`, `**a**` for nested
    /// emphasis), or `_` would stand inside a word, where it cannot open or close.
    fn emphasis(&mut self, id: NodeId, strong: bool) {
        let ast = self.printer.ast;
        let config = self.printer.config;
        let preferred = if strong {
            config.strong.as_str()
        } else {
            config.emphasis.as_str()
        };
        let preferred = preferred.chars().next().unwrap_or('*');
        let other = if preferred == '*' { '_' } else { '*' };
        let parent = ast.parent(id);
        let parent_is_emphasis =
            parent.is_some_and(|p| matches!(ast.kind(p), NodeKind::Emphasis | NodeKind::Strong));
        // `***a***` is emphasis around strong emphasis and `****a****` strong around
        // strong, so strong emphasis inside either may share its character.
        let strong_in_emphasis = strong && parent_is_emphasis;
        let touches_before = self.out.ends_with(preferred)
            && !(strong_in_emphasis && ast.prev_sibling(id).is_none());
        let touches_after = parent_is_emphasis
            && ast.next_sibling(id).is_none()
            && self.delimiters.last() == Some(&preferred)
            && !strong_in_emphasis;
        let mut c = if touches_before || touches_after {
            other
        } else {
            preferred
        };
        // Touching its parent's delimiter, it shares the parent's flanking: one run.
        if strong_in_emphasis
            && (ast.prev_sibling(id).is_none() || ast.next_sibling(id).is_none())
            && let Some(&outer) = self.delimiters.last()
        {
            c = outer;
        }
        if c == '_' && self.intraword(id) {
            c = '*';
        }
        let delimiter = c.to_string().repeat(if strong { 2 } else { 1 });
        self.out.push_str(&delimiter);
        self.delimiters.push(c);
        self.children(id);
        self.delimiters.pop();
        self.out.push_str(&delimiter);
    }

    /// Whether the delimiters of `id` would stand between alphanumerics.
    fn intraword(&self, id: NodeId) -> bool {
        let before = self
            .out
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after = self.printer.ast.next_sibling(id).is_some_and(|next| {
            matches!(self.printer.ast.kind(next), NodeKind::Text(text)
                if text.chars().next().is_some_and(char::is_alphanumeric))
        });
        before || after
    }

    /// A link or image (its `!` is already written) in the form it was written in.
    fn link(&mut self, id: NodeId, link: &Link) {
        let ast = self.printer.ast;
        match &link.form {
            LinkForm::Autolink => {
                self.out.push('<');
                self.out.push_str(&ast.text(id));
                self.out.push('>');
            }
            LinkForm::Bare => self.children(id),
            LinkForm::Inline => {
                self.out.push('[');
                self.children(id);
                self.out.push_str("](");
                let destination = destination(&link.url);
                match &link.title {
                    Some(title) => {
                        let destination = if destination.is_empty() {
                            "<>"
                        } else {
                            &destination
                        };
                        self.out.push_str(destination);
                        self.out.push(' ');
                        self.out.push_str(&title_string(title));
                    }
                    None => self.out.push_str(&destination),
                }
                self.out.push(')');
            }
            LinkForm::Reference { label, style } => {
                self.out.push('[');
                let start = self.out.len();
                self.children(id);
                // Labels match with their whitespace collapsed.
                let same = collapse_whitespace(&self.out[start..]) == collapse_whitespace(label);
                self.out.push(']');
                // A shortcut before a bracket would take it as its label.
                let bracket_follows = ast.next_sibling(id).is_some_and(|next| {
                    matches!(
                        ast.kind(next),
                        NodeKind::Link(_) | NodeKind::Image(_) | NodeKind::FootnoteReference(_)
                    )
                });
                match style {
                    ReferenceStyle::Shortcut if same && !bracket_follows => {}
                    ReferenceStyle::Shortcut | ReferenceStyle::Collapsed if same => {
                        self.out.push_str("[]");
                    }
                    _ => {
                        self.out.push('[');
                        self.out.push_str(&collapse_whitespace(label));
                        self.out.push(']');
                    }
                }
            }
        }
    }

    fn text_context(&self, id: NodeId) -> TextContext {
        let ast = self.printer.ast;
        let breaks = |node: Option<NodeId>| {
            node.is_some_and(|n| matches!(ast.kind(n), NodeKind::SoftBreak | NodeKind::LineBreak))
        };
        // Whitespace inside link text is kept by the parser.
        let in_link = ast
            .parent(id)
            .is_some_and(|p| matches!(ast.kind(p), NodeKind::Link(_) | NodeKind::Image(_)));
        let in_block = ast.parent(id).is_some_and(|p| ast.kind(p).is_block());
        let prev = ast.prev_sibling(id);
        let next = ast.next_sibling(id);
        let line_start = self.out.is_empty() || self.out.ends_with('\n');
        TextContext {
            line_start,
            strip_start: line_start || breaks(prev) || (prev.is_none() && !in_link),
            strip_end: breaks(next) || (next.is_none() && !in_link),
            line_end: breaks(next) || (next.is_none() && in_block),
            before_hard_break: next.is_some_and(|n| matches!(ast.kind(n), NodeKind::LineBreak)),
            // Text and soft breaks print a `>` only where the text has one, so this does
            // not depend on where the lines of the source broke.
            followed: std::iter::successors(next, |&n| ast.next_sibling(n)).any(|n| {
                match ast.kind(n) {
                    NodeKind::Text(text) => text.contains('>'),
                    NodeKind::SoftBreak => false,
                    _ => true,
                }
            }),
            after_bracket: self.out.ends_with(']'),
            before_bracket: next.is_some_and(|n| match ast.kind(n) {
                NodeKind::Link(link) => !matches!(link.form, LinkForm::Autolink | LinkForm::Bare),
                NodeKind::FootnoteReference(_) | NodeKind::WikiLink(_) => true,
                _ => false,
            }),
            starts_parent: prev.is_none(),
            ends_heading: next.is_none()
                && ast
                    .parent(id)
                    .is_some_and(|p| matches!(ast.kind(p), NodeKind::Heading(_))),
        }
    }

    /// `text` with every character escaped that would otherwise be read as markup here.
    fn escape(&self, text: &str, context: TextContext) -> String {
        let syntax = self.printer.syntax;
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        // A flow's lines are escaped once they are known.
        let marker_at = if context.line_start && self.flow.is_none() {
            self.printer.line_start_marker(text)
        } else {
            None
        };
        let before = if context.line_start {
            Some('\n')
        } else {
            self.out.chars().next_back()
        };
        let after = if context.before_hard_break {
            Some('\\')
        } else {
            context.line_end.then_some('\n')
        };
        let hooks = self.printer.hooks;
        let spot = EscapeContext {
            literal: text,
            before: if context.starts_parent && !context.line_start {
                None
            } else {
                before
            },
            after,
            ends_heading: context.ends_heading,
        };
        let mut out = String::with_capacity(text.len() + 8);
        for (i, &(at, c)) in chars.iter().enumerate() {
            // The characters around `c` in the output; `None` past the end of the text
            // when the next node is unknown.
            let prev = i.checked_sub(1).map(|j| chars[j].1).or(before);
            let next = chars.get(i + 1).map(|&(_, c)| c).or(after);
            let first = i == 0;
            let last = i + 1 == chars.len();
            if matches!(c, ' ' | '\t')
                && ((first && context.strip_start) || (last && context.strip_end))
            {
                out.push_str(if c == ' ' { "&#32;" } else { "&#9;" });
                continue;
            }
            match c {
                '\n' => {
                    out.push_str("&#10;");
                    continue;
                }
                '\r' => {
                    out.push_str("&#13;");
                    continue;
                }
                _ => {}
            }
            let escape = match c {
                '\\' | '`' | '[' | ']' => true,
                '*' | '_' => self.may_delimit(c, prev, next),
                // Raw HTML and autolinks need a closing `>`.
                '<' => {
                    !next.is_some_and(char::is_whitespace)
                        && (text[at..].contains('>') || context.followed)
                }
                '&' => entity_like(&text[at + 1..]),
                '~' => syntax.strikethrough || syntax.subscript,
                '$' => syntax.math,
                '^' => syntax.superscript,
                '=' => syntax.highlight && (prev == Some('=') || next == Some('=')),
                '!' => last && context.before_bracket,
                '(' | ':' => first && context.after_bracket,
                _ => false,
            } || marker_at == Some(at)
                || hooks.is_some_and(|hooks| hooks.escapes(&spot, at));
            if escape && hooks.is_some_and(|hooks| !hooks.backslash_escapes(c)) {
                let _ = write!(out, "&#{};", u32::from(c));
                continue;
            }
            if escape {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }
}

impl Inlines<'_, '_> {
    /// Whether an unescaped `*` or `_` between `prev` and `next` (`None`: unknown, a
    /// line break: `'\n'`) could be read as emphasis. One that can only close is safe
    /// outside emphasis of the same character: every other run that could open is
    /// escaped, so there is nothing for it to close.
    fn may_delimit(&self, c: char, prev: Option<char>, next: Option<char>) -> bool {
        let (Some(prev), Some(next)) = (prev, next) else {
            return true;
        };
        let left = !is_whitespace(next)
            && (!is_punctuation(next) || is_whitespace(prev) || is_punctuation(prev));
        let right = !is_whitespace(prev)
            && (!is_punctuation(prev) || is_whitespace(next) || is_punctuation(next));
        let (opens, closes) = if c == '_' {
            (
                left && (!right || is_punctuation(prev)),
                right && (!left || is_punctuation(next)),
            )
        } else {
            (left, right)
        };
        opens || (closes && self.delimiters.contains(&c))
    }
}

/// Unicode whitespace as `CommonMark` defines it; a line break counts.
fn is_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0c' | '\r')
        || get_general_category(c) == GeneralCategory::SpaceSeparator
}

/// Unicode punctuation as `CommonMark` defines it: general categories P and S.
fn is_punctuation(c: char) -> bool {
    use GeneralCategory as G;
    matches!(
        get_general_category(c),
        G::ConnectorPunctuation
            | G::DashPunctuation
            | G::OpenPunctuation
            | G::ClosePunctuation
            | G::InitialPunctuation
            | G::FinalPunctuation
            | G::OtherPunctuation
            | G::MathSymbol
            | G::CurrencySymbol
            | G::ModifierSymbol
            | G::OtherSymbol
    )
}

/// The byte offset of the character to escape when `text` starts a line (of a paragraph
/// that has begun already): the first one of an ATX heading, block quote, bullet item,
/// setext underline, thematic break, code fence, HTML block, table delimiter row or
/// definition (with `syntax.description_lists`), the delimiter of an ordered item.
/// Flavor syntax is left to
/// [`PrinterHooks::line_start_marker`](crate::flavor::PrinterHooks::line_start_marker).
/// `text` is a text node or a whole printed line, so markup it may start with
/// (emphasis, a `==highlight==`, an autolink) is never escaped.
pub(super) fn line_start_marker(text: &str, syntax: ParseOptions) -> Option<usize> {
    let bytes = text.as_bytes();
    let blank_at = |i: usize| bytes.get(i).is_none_or(|b| matches!(b, b' ' | b'\t'));
    let only = |chars: &[u8]| bytes.iter().all(|b| chars.contains(b));
    match *bytes.first()? {
        b'#' => {
            let run = bytes.iter().take_while(|&&b| b == b'#').count();
            (run <= 6 && blank_at(run)).then_some(0)
        }
        b'>' => Some(0),
        b'=' => only(b"= \t").then_some(0),
        b'-' | b'|' | b':' if syntax.table && is_delimiter_row(text) => Some(0),
        b'-' => (blank_at(1) || bytes.get(1) == Some(&b'-')).then_some(0),
        b'+' | b'*' => blank_at(1).then_some(0),
        b'_' => (only(b"_ \t") && text.matches('_').count() >= 3).then_some(0),
        b'~' => text.starts_with("~~~").then_some(0),
        b'<' => starts_html_block(text).then_some(0),
        b':' => (syntax.description_lists && blank_at(1)).then_some(0),
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            (digits <= 9 && matches!(bytes.get(digits), Some(b'.' | b')')) && blank_at(digits + 1))
                .then_some(digits)
        }
        _ => None,
    }
}

/// Whether `line` could be the delimiter row of a table (`-|-`, `:--`): only pipes,
/// colons, dashes and blanks, with a dash.
fn is_delimiter_row(line: &str) -> bool {
    line.bytes()
        .all(|b| matches!(b, b'|' | b':' | b'-' | b' ' | b'\t'))
        && line.contains('-')
}

/// The block-level tag names of HTML blocks of type 6.
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// Whether a line starting with `line` starts an HTML block that may interrupt a
/// paragraph (types 1 to 6 of `CommonMark`).
fn starts_html_block(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('<') else {
        return false;
    };
    if rest.starts_with(['!', '?']) {
        return true;
    }
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let name_len = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let name = rest[..name_len].to_ascii_lowercase();
    let after = &rest[name_len..];
    let ends = after.is_empty() || after.starts_with([' ', '\t', '>']) || after.starts_with("/>");
    ends && (BLOCK_TAGS.contains(&name.as_str())
        || ["script", "pre", "style", "textarea"].contains(&name.as_str()))
}

/// Whether `rest` (the text after a `&`) would complete a character reference.
fn entity_like(rest: &str) -> bool {
    let Some(end) = rest.find(';') else {
        return false;
    };
    let name = &rest[..end];
    if let Some(number) = name.strip_prefix('#') {
        let (digits, radix) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16),
            None => (number, 10),
        };
        !digits.is_empty() && digits.len() <= 7 && digits.chars().all(|c| c.is_digit(radix))
    } else {
        named_references().contains(name)
    }
}

/// The names of the HTML5 named character references (`amp` for `&amp;`).
fn named_references() -> &'static HashSet<&'static str> {
    static NAMES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        entities::ENTITIES
            .iter()
            .filter_map(|entity| entity.entity.strip_prefix('&')?.strip_suffix(';'))
            .collect()
    })
}

/// A code span around `code`: a backtick run that does not occur in it, and a space on
/// each side where the content would otherwise lose or merge with one.
fn code_span(code: &str) -> String {
    // A CR left over from a CRLF line ending would end the line.
    let code = &code.replace('\r', "");
    let mut runs = Vec::new();
    let mut run = 0;
    for c in code.chars().chain(std::iter::once(' ')) {
        if c == '`' {
            run += 1;
        } else if run > 0 {
            runs.push(run);
            run = 0;
        }
    }
    let ticks = (1..=runs.len() + 1)
        .find(|n| !runs.contains(n))
        .unwrap_or(1);
    let fence = "`".repeat(ticks);
    let pad = code.starts_with('`')
        || code.ends_with('`')
        || (code.starts_with(' ') && code.ends_with(' ') && !code.chars().all(|c| c == ' '));
    if pad {
        format!("{fence} {code} {fence}")
    } else {
        format!("{fence}{code}{fence}")
    }
}

/// `[[target#anchor|alias]]`.
fn wikilink(link: &WikiLink) -> String {
    let mut out = format!("[[{}", link.target);
    if let Some(anchor) = &link.anchor {
        out.push('#');
        out.push_str(anchor);
    }
    if let Some(alias) = &link.alias {
        out.push('|');
        out.push_str(alias);
    }
    out.push_str("]]");
    out
}

/// A link destination: bare when it can be, else in `<…>`.
pub(super) fn destination(url: &str) -> String {
    let mut depth = 0i32;
    let mut balanced = true;
    for c in url.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                balanced &= depth >= 0;
            }
            _ => {}
        }
    }
    balanced &= depth == 0;
    let pointy = url.is_empty()
        || url.starts_with('<')
        || url
            .chars()
            .any(|c| c.is_ascii_whitespace() || c.is_ascii_control());
    let mut out = String::with_capacity(url.len() + 2);
    if pointy {
        if url.is_empty() {
            return String::new();
        }
        out.push('<');
    }
    for (at, c) in url.char_indices() {
        let escape = match c {
            '\\' => true,
            '<' | '>' => pointy,
            '(' | ')' => !pointy && !balanced,
            '&' => entity_like(&url[at + 1..]),
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        if c == '\n' {
            out.push_str("%0A");
            continue;
        }
        out.push(c);
    }
    if pointy {
        out.push('>');
    }
    out
}

/// A link title in double quotes.
pub(super) fn title_string(title: &str) -> String {
    let title = &title.replace("\r\n", "\n");
    let mut out = String::with_capacity(title.len() + 2);
    out.push('"');
    for (at, c) in title.char_indices() {
        if matches!(c, '"' | '\\') || (c == '&' && entity_like(&title[at + 1..])) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The info string of a code fence, escaped.
pub(super) fn info(info: &str) -> String {
    let mut out = String::with_capacity(info.len());
    for (at, c) in info.char_indices() {
        if c == '\\' || (c == '&' && entity_like(&info[at + 1..])) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
