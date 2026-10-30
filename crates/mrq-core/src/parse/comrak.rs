//! [`ComrakParser`]: comrak options from [`ParseOptions`] and conversion of comrak's arena
//! into an [`Ast`].

use std::rc::Rc;

use ::comrak::nodes::{
    AlertType, LineColumn, ListDelimType, ListType, NodeCodeBlock, NodeValue, TableAlignment,
};
use ::comrak::{Arena, Node as ComrakNode, Options};

use super::span::{LineIndex, repair, text_matches};
use super::{ParseOptions, Parser};
use crate::ast::{
    Alignment, Ast, Callout, CodeBlock, Comment, CommentSyntax, Fence, FootnoteDefinition,
    FootnoteReference, Heading, Link, LinkForm, List, ListItem, ListKind, Math, NodeId, NodeKind,
    OrderedDelimiter, ReferenceStyle, Span, Table, TableRow, TaskState,
};

/// A [`Parser`] backed by comrak.
#[derive(Debug, Clone, Copy, Default)]
pub struct ComrakParser {
    options: ParseOptions,
}

impl ComrakParser {
    /// A parser recognizing the extensions enabled in `options`.
    pub const fn new(options: ParseOptions) -> Self {
        Self { options }
    }

    /// The options this parser was created with.
    pub const fn options(&self) -> ParseOptions {
        self.options
    }
}

impl Parser for ComrakParser {
    fn parse(&self, src: &str) -> Ast {
        let arena = Arena::new();
        let root = ::comrak::parse_document(&arena, src, &comrak_options(self.options));
        let lines = LineIndex::new(src);
        fix_multiline_links(root, &lines, src);
        let mut converter = Converter {
            src,
            lines,
            ast: Ast::new(Span::new(0, src.len())),
            spans: vec![None],
        };
        let mut pending = vec![(root, converter.ast.root(), Fixup::default())];
        while let Some((node, parent, fixup)) = pending.pop() {
            // Push in reverse so children are converted, and get ids, in document order.
            let start = pending.len();
            converter.convert_children(node, parent, &fixup, &mut pending);
            pending[start..].reverse();
        }
        let Converter { mut ast, spans, .. } = converter;
        repair(&mut ast, src, &spans);
        ast
    }
}

/// comrak options for `options`. Rendering options are irrelevant; footnote definitions
/// stay where they were written instead of moving to the end of the document.
fn comrak_options(options: ParseOptions) -> Options<'static> {
    let mut comrak = Options::default();
    let ext = &mut comrak.extension;
    ext.strikethrough = options.strikethrough;
    ext.table = options.table;
    ext.autolink = options.autolink;
    ext.tasklist = options.tasklist;
    ext.footnotes = options.footnotes;
    ext.alerts = options.alerts;
    ext.math_dollars = options.math;
    ext.highlight = options.highlight;
    ext.superscript = options.superscript;
    ext.subscript = options.subscript;
    ext.description_lists = options.description_lists;
    comrak.parse.leave_footnote_definitions = true;
    comrak
}

/// An inline link whose destination or title contains line endings, in comrak's
/// coordinates (see [`fix_multiline_links`]).
#[derive(Debug, Clone, Copy)]
struct Swallow {
    /// comrak's line of the link.
    line: usize,
    /// The column at which comrak believes the link ends.
    column: usize,
    /// Offset just past the link's `)` in the source.
    end: usize,
    /// Line endings swallowed by this and earlier links of the block.
    lines: usize,
}

/// Rewrites comrak's positions around inline links whose destination or title spans
/// lines.
///
/// comrak doesn't count the line endings inside a link destination or title: it ends the
/// link on its first line, keeps counting columns on that line past the link, and numbers
/// every later line of the block too low by the swallowed line endings. This pass scans
/// such links in the source and moves the positions of the link and everything after it
/// back onto the source.
fn fix_multiline_links(root: ComrakNode<'_>, lines: &LineIndex<'_>, src: &str) {
    let blocks = root.descendants().filter(|node| {
        matches!(
            node.data().value,
            NodeValue::Paragraph | NodeValue::Heading(_) | NodeValue::TableCell
        )
    });
    for block in blocks {
        let layout = Layout::of(block);
        let mut swallowed: Vec<Swallow> = Vec::new();
        for node in block.descendants().skip(1) {
            let pos = node.data().sourcepos;
            let start = unswallow(&swallowed, lines, &layout, pos.start);
            let mut end = unswallow(&swallowed, lines, &layout, pos.end);
            if matches!(node.data().value, NodeValue::Link(_) | NodeValue::Image(_))
                && let Some(from) = lines.offset((start.line, start.column))
                && let Some((paren, to)) = inline_link_end(src, from)
            {
                // Only the destination and title are affected; line endings in the link
                // text are counted.
                let tail = &src[paren..to];
                let line_endings = tail.matches('\n').count() + tail.matches('\r').count()
                    - tail.matches("\r\n").count();
                if line_endings > 0 {
                    // comrak's content lines lack the indentation and quote markers.
                    let prefixes: usize = tail
                        .split_inclusive(['\n', '\r'])
                        .skip(1)
                        .map(|line| line.len() - line.trim_start_matches([' ', '\t', '>']).len())
                        .sum();
                    let earlier = swallowed.last().map_or(0, |s| s.lines);
                    let (paren_line, paren_column) = lines.position(paren);
                    swallowed.push(Swallow {
                        line: paren_line - earlier,
                        column: paren_column + tail.len() - prefixes - 1,
                        end: to,
                        lines: earlier + line_endings,
                    });
                    end = lines.position(to - 1).into();
                }
            }
            if (start, end) != (pos.start, pos.end) {
                node.data_mut().sourcepos = (start, end).into();
            }
        }
    }
}

/// The source position of comrak's position `pos` in a block with `swallowed` links.
///
/// Later lines are numbered too low, and comrak takes their content columns from the line
/// with the number it believes they have.
fn unswallow(
    swallowed: &[Swallow],
    lines: &LineIndex<'_>,
    layout: &Layout,
    pos: LineColumn,
) -> LineColumn {
    let Some(last) = swallowed
        .iter()
        .rev()
        .find(|s| (s.line, s.column) < (pos.line, pos.column))
    else {
        return pos;
    };
    if pos.line == last.line {
        lines
            .position(last.end + (pos.column - last.column - 1))
            .into()
    } else {
        let line = pos.line + last.lines;
        let column = (pos.column + layout.content_start(lines, line))
            .checked_sub(layout.content_start(lines, pos.line))
            .unwrap_or(pos.column);
        LineColumn { line, column }
    }
}

/// Where the content of an inline container's lines starts.
#[derive(Debug, Clone, Copy)]
struct Layout {
    /// comrak's start of the block.
    start: LineColumn,
    /// Number of enclosing block quotes (and alerts).
    quotes: usize,
}

impl Layout {
    fn of(block: ComrakNode<'_>) -> Self {
        let quotes = block
            .ancestors()
            .filter(|node| {
                matches!(
                    node.data().value,
                    NodeValue::BlockQuote | NodeValue::Alert(_)
                )
            })
            .count();
        Self {
            start: block.data().sourcepos.start,
            quotes,
        }
    }

    /// 1-based column where content starts on `line`: the block's start on its first
    /// line, after the quote markers and indentation on the others.
    fn content_start(&self, lines: &LineIndex<'_>, line: usize) -> usize {
        self.content_starts(lines, line)[0]
    }

    /// [`content_start`](Self::content_start), plus the column after the quote markers
    /// only, which is where a lazy continuation line's content starts.
    fn content_starts(&self, lines: &LineIndex<'_>, line: usize) -> [usize; 2] {
        if line == self.start.line {
            return [self.start.column; 2];
        }
        let text = lines.line_text(line);
        let after_markers = strip_quote_markers(text, self.quotes);
        let indented = after_markers.trim_start_matches([' ', '\t']);
        [
            1 + text.len() - indented.len(),
            1 + text.len() - after_markers.len(),
        ]
    }
}

/// Corrections to comrak's positions for the inlines of one block.
#[derive(Debug, Clone, Default)]
struct Fixup {
    /// Added to every line number (see [`Converter::inline_fixup`]).
    lines: usize,
    /// The first line of the block, in comrak's numbering.
    base_line: usize,
    /// Per line of the block (from `base_line`), the byte column where comrak believes
    /// the content starts and where it really starts; positions on that line are moved
    /// by the difference. Lines past the end are left alone.
    columns: Rc<[(usize, usize)]>,
    /// Offsets of the backslashes of `\|` escapes in a table cell, ascending. comrak
    /// parses the cell's inlines with these backslashes removed, so its columns are short
    /// by the number of escapes before them.
    pipe_escapes: Rc<[usize]>,
}

impl Fixup {
    /// Applies the line and column corrections to a 1-based comrak position.
    fn position(&self, line: usize, column: usize) -> Option<(usize, usize)> {
        let column = match line
            .checked_sub(self.base_line)
            .and_then(|index| self.columns.get(index))
        {
            Some(&(believed, actual)) => (column + actual).checked_sub(believed)?,
            None => column,
        };
        Some((line + self.lines, column))
    }

    /// The source offset of `offset`, a position comrak computed in unescaped cell text.
    fn unescape(&self, offset: usize) -> usize {
        let before = self
            .pipe_escapes
            .iter()
            .enumerate()
            .take_while(|&(i, &backslash)| backslash - i < offset)
            .count();
        offset + before
    }
}

/// State of one conversion: the tree being built and the spans comrak reported for it.
struct Converter<'s> {
    src: &'s str,
    lines: LineIndex<'s>,
    ast: Ast,
    /// Per arena index, the span from comrak's sourcepos (`None` where it is unusable).
    spans: Vec<Option<Span>>,
}

impl Converter<'_> {
    /// Converts the children of `node` and appends them to `parent`; nodes without an mrq
    /// counterpart are skipped and their children spliced into `parent`. Each converted
    /// child is pushed to `pending` for its own children. `fixup` corrects the positions
    /// comrak reports for the children.
    fn convert_children<'a>(
        &mut self,
        node: ComrakNode<'a>,
        parent: NodeId,
        fixup: &Fixup,
        pending: &mut Vec<(ComrakNode<'a>, NodeId, Fixup)>,
    ) {
        for child in node.children() {
            let value = &child.data().value;
            let mut span = self.sourcepos(child, fixup);
            if let (NodeValue::CodeBlock(code), Some(s)) = (value, span.as_mut())
                && !code.fenced
            {
                s.start = with_indent(self.src, s.start);
            }
            match self.kind(value, span) {
                Some(kind) => {
                    if let (NodeKind::FootnoteDefinition(def), Some(s)) = (&kind, span.as_mut()) {
                        // At least `[^label]:`.
                        s.end = s.end.max(s.start + def.label.len() + 4).min(self.src.len());
                    }
                    let inner = match value {
                        NodeValue::Paragraph | NodeValue::Heading(_) => self.inline_fixup(child),
                        NodeValue::TableCell => Fixup {
                            pipe_escapes: span
                                .map_or_else(Rc::default, |s| pipe_escapes(self.src, s)),
                            ..Fixup::default()
                        },
                        _ => fixup.clone(),
                    };
                    let id = self.ast.create(kind, Span::default());
                    self.ast
                        .append_child(parent, id)
                        .expect("fresh node appends");
                    self.spans.push(span);
                    pending.push((child, id, inner));
                }
                None => self.convert_children(child, parent, fixup, pending),
            }
        }
    }

    /// The span of `node`'s sourcepos, corrected by `fixup`.
    fn sourcepos(&self, node: ComrakNode<'_>, fixup: &Fixup) -> Option<Span> {
        let pos = node.data().sourcepos;
        let span = self.lines.span(
            fixup.position(pos.start.line, pos.start.column)?,
            fixup.position(pos.end.line, pos.end.column)?,
        )?;
        if fixup.pipe_escapes.is_empty() {
            return Some(span);
        }
        let end = fixup.unescape(span.end).min(self.src.len());
        Some(Span::new(fixup.unescape(span.start).min(end), end))
    }

    /// Corrections for the inline positions inside `block` (a paragraph or heading),
    /// for two cases where comrak's positions drift from the source:
    ///
    /// - When link reference definitions open a paragraph, comrak removes their lines but
    ///   keeps numbering the remaining lines from the paragraph's first line, with the
    ///   content columns of the removed lines: every inline is reported `k` lines too
    ///   early, and its column is off by the difference in content indentation. Where
    ///   content starts depends on whether a line is a lazy continuation, so both
    ///   candidates are tried.
    /// - After a tab that is only partly consumed as indentation, comrak counts the rest
    ///   of the tab as spaces, so columns on that line are up to three too large.
    ///
    /// Inlines whose source is known from their value anchor the correction: per line,
    /// the first candidate correction that puts the most anchors on matching source wins;
    /// across line shifts the smallest `k` with the most hits. The fixup is a no-op if
    /// the anchors already match.
    fn inline_fixup(&self, block: ComrakNode<'_>) -> Fixup {
        let pos = block.data().sourcepos;
        let Some(span) = self.sourcepos(block, &Fixup::default()) else {
            return Fixup::default();
        };
        let refdefs = self.src[span.start..]
            .trim_start_matches(' ')
            .starts_with('[');
        let base = pos.start.line;
        // comrak's end of some blocks (definition terms) is too early; the parent and the
        // inlines bound the lines the content can be on.
        let last = block
            .descendants()
            .chain(block.parent())
            .map(|node| node.data().sourcepos.end.line)
            .max()
            .unwrap_or(base)
            .saturating_sub(base);
        let tabs = (1..=last).any(|index| self.lines.line_text(base + index).contains('\t'));
        if !refdefs && !tabs {
            return Fixup::default();
        }
        // Anchors: inlines whose source is (roughly) known from their value.
        let anchors: Vec<_> = block
            .descendants()
            .filter(|node| anchor_matches(&node.data().value, None))
            .collect();
        // Anchors that land on matching source under `fixup`, among those starting or
        // ending on `line`.
        let hits = |fixup: &Fixup, line: Option<usize>| {
            anchors
                .iter()
                .filter(|anchor| {
                    line.is_none_or(|line| {
                        let pos = anchor.data().sourcepos;
                        pos.start.line == line || pos.end.line == line
                    }) && self.sourcepos(anchor, fixup).is_some_and(|s| {
                        anchor_matches(&anchor.data().value, Some(&self.src[s.start..s.end]))
                    })
                })
                .count()
        };
        if hits(&Fixup::default(), None) == anchors.len() {
            return Fixup::default();
        }
        // Candidate 1-based columns where comrak's content starts on each line of the
        // block; lazy continuation lines keep their indentation.
        let layout = Layout::of(block);
        let starts: Vec<[usize; 2]> = (0..=last)
            .map(|index| layout.content_starts(&self.lines, base + index))
            .collect();
        let shifts = if refdefs { last } else { 0 };
        let mut best = (0, Fixup::default());
        for lines in 0..=shifts {
            // Per line, the (believed, actual) column pair that puts the most anchors in
            // place.
            let columns: Rc<[(usize, usize)]> = (0..=last - lines)
                .map(|index| {
                    let mut pairs = Vec::new();
                    for &believed in &starts[index] {
                        for &actual in &starts[index + lines] {
                            pairs.push((believed, actual));
                        }
                    }
                    if self.lines.line_text(base + index + lines).contains('\t') {
                        pairs.extend((1..=3).map(|excess| (excess, 0)));
                    }
                    let mut best_pair = (0, pairs[0]);
                    for pair in pairs {
                        let fixup = Fixup {
                            lines,
                            base_line: base + index,
                            columns: Rc::from([pair]),
                            ..Fixup::default()
                        };
                        let count = hits(&fixup, Some(base + index));
                        if count > best_pair.0 {
                            best_pair = (count, pair);
                        }
                    }
                    best_pair.1
                })
                .collect();
            let fixup = Fixup {
                lines,
                base_line: base,
                columns,
                ..Fixup::default()
            };
            let count = hits(&fixup, None);
            if count > best.0 {
                best = (count, fixup);
            }
        }
        best.1
    }

    /// The mrq kind for a comrak value; `span` is its reported source, used to recover
    /// details comrak doesn't keep (link forms, labels as written). `None` for values
    /// whose content is spliced into the parent.
    fn kind(&self, value: &NodeValue, span: Option<Span>) -> Option<NodeKind> {
        let source = span.map_or("", |s| &self.src[s.start..s.end]);
        Some(match value {
            NodeValue::Document => NodeKind::Document,
            NodeValue::Paragraph => NodeKind::Paragraph,
            NodeValue::Heading(heading) => NodeKind::Heading(Heading {
                level: heading.level,
                setext: heading.setext,
                attrs: None,
            }),
            NodeValue::ThematicBreak => NodeKind::ThematicBreak,
            NodeValue::BlockQuote => NodeKind::BlockQuote,
            NodeValue::List(list) => NodeKind::List(List {
                kind: match list.list_type {
                    ListType::Bullet => ListKind::Bullet {
                        marker: char::from(list.bullet_char),
                    },
                    ListType::Ordered => ListKind::Ordered {
                        start: list.start as u64,
                        delimiter: match list.delimiter {
                            ListDelimType::Period => OrderedDelimiter::Period,
                            ListDelimType::Paren => OrderedDelimiter::Paren,
                        },
                    },
                },
                tight: list.tight,
            }),
            NodeValue::Item(_) => NodeKind::ListItem(ListItem { task: None }),
            NodeValue::TaskItem(task) => NodeKind::ListItem(ListItem {
                task: Some(task.symbol.map_or(TaskState::Open, TaskState::from_marker)),
            }),
            NodeValue::CodeBlock(code) => NodeKind::CodeBlock(code_block(code)),
            NodeValue::HtmlBlock(html) => html_comment(&html.literal).map_or_else(
                || NodeKind::HtmlBlock(html.literal.clone()),
                NodeKind::Comment,
            ),
            NodeValue::Table(table) => NodeKind::Table(Table {
                alignments: table.alignments.iter().map(|&a| alignment(a)).collect(),
            }),
            NodeValue::TableRow(header) => NodeKind::TableRow(TableRow { header: *header }),
            NodeValue::TableCell => NodeKind::TableCell,
            NodeValue::FootnoteDefinition(def) => {
                // comrak may end an empty definition before its label; read to the end.
                let rest = span.map_or("", |s| &self.src[s.start..]);
                NodeKind::FootnoteDefinition(FootnoteDefinition {
                    label: footnote_label(rest, "]:").unwrap_or(&def.name).to_owned(),
                })
            }
            NodeValue::DescriptionList => NodeKind::DefinitionList,
            NodeValue::DescriptionItem(_) => NodeKind::DefinitionItem,
            NodeValue::DescriptionTerm => NodeKind::DefinitionTerm,
            NodeValue::DescriptionDetails => NodeKind::DefinitionDetails,
            NodeValue::Alert(alert) => NodeKind::Callout(Callout {
                kind: alert_kind(source).map_or_else(
                    || alert_type_name(alert.alert_type).to_owned(),
                    str::to_owned,
                ),
                title: alert.title.clone(),
                fold: None,
            }),
            NodeValue::Text(text) => NodeKind::Text(text.to_string()),
            NodeValue::SoftBreak => NodeKind::SoftBreak,
            NodeValue::LineBreak => NodeKind::LineBreak,
            NodeValue::Code(code) => NodeKind::Code(code.literal.clone()),
            NodeValue::HtmlInline(html) => html_comment(html)
                .map_or_else(|| NodeKind::HtmlInline(html.clone()), NodeKind::Comment),
            NodeValue::Emph => NodeKind::Emphasis,
            NodeValue::Strong => NodeKind::Strong,
            NodeValue::Strikethrough => NodeKind::Strikethrough,
            NodeValue::Highlight => NodeKind::Highlight,
            NodeValue::Superscript => NodeKind::Superscript,
            NodeValue::Subscript => NodeKind::Subscript,
            NodeValue::Link(link) => NodeKind::Link(Link {
                url: link.url.clone(),
                title: (!link.title.is_empty()).then(|| link.title.clone()),
                form: link_form(source, false),
            }),
            NodeValue::Image(link) => NodeKind::Image(Link {
                url: link.url.clone(),
                title: (!link.title.is_empty()).then(|| link.title.clone()),
                form: link_form(source, true),
            }),
            NodeValue::FootnoteReference(reference) => {
                NodeKind::FootnoteReference(FootnoteReference {
                    label: footnote_label(source, "]")
                        .unwrap_or(&reference.name)
                        .to_owned(),
                })
            }
            NodeValue::Math(math) => NodeKind::Math(Math {
                display: math.display_math,
                literal: math.literal.clone(),
            }),
            // Only produced by comrak extensions that `comrak_options` never enables.
            _ => return None,
        })
    }
}

/// Whether `source` matches an inline's value: text through [`text_matches`], HTML
/// exactly, code spans by their backticks, other inlines by their opening delimiter.
/// With `source` `None`, whether the value can be checked at all. Links can't: bare
/// autolinks have no delimiter (their text anchors them).
fn anchor_matches(value: &NodeValue, source: Option<&str>) -> bool {
    let opens =
        |delimiters: &[&str]| source.is_none_or(|s| delimiters.iter().any(|d| s.starts_with(d)));
    match value {
        NodeValue::Text(literal) => {
            !literal.is_empty() && source.is_none_or(|s| text_matches(s, literal))
        }
        NodeValue::HtmlInline(html) => source.is_none_or(|s| s == html),
        NodeValue::Code(code) => source.is_none_or(|s| {
            let fence = "`".repeat(code.num_backticks);
            s.len() >= 2 * fence.len() && s.starts_with(&fence) && s.ends_with(&fence)
        }),
        NodeValue::FootnoteReference(_) => opens(&["[^"]),
        NodeValue::Emph | NodeValue::Strong => opens(&["*", "_"]),
        NodeValue::Strikethrough | NodeValue::Subscript => opens(&["~"]),
        NodeValue::Superscript => opens(&["^"]),
        NodeValue::Highlight => opens(&["=="]),
        NodeValue::Math(_) => opens(&["$"]),
        _ => false,
    }
}

/// `line` without up to `quotes` block quote markers and the indentation before them
/// (lazy lines have fewer markers).
fn strip_quote_markers(mut line: &str, mut quotes: usize) -> &str {
    while quotes > 0
        && let Some(rest) = line.trim_start_matches([' ', '\t']).strip_prefix('>')
    {
        line = rest;
        quotes -= 1;
    }
    line
}

/// Moves `start`, the first character of an indented code block, back over the
/// indentation that makes it one: up to four columns of spaces, or a tab.
fn with_indent(src: &str, start: usize) -> usize {
    let mut columns = 0;
    let mut at = start;
    for byte in src[..start].bytes().rev() {
        columns += match byte {
            b' ' => 1,
            b'\t' => 4,
            _ => break,
        };
        if columns > 4 {
            break;
        }
        at -= 1;
    }
    at
}

/// Offsets of the backslashes of the `\|` escapes in `cell`.
fn pipe_escapes(src: &str, cell: Span) -> Rc<[usize]> {
    src[cell.start..cell.end]
        .match_indices("\\|")
        .map(|(i, _)| cell.start + i)
        .collect()
}

fn code_block(code: &NodeCodeBlock) -> CodeBlock {
    CodeBlock {
        fence: code.fenced.then(|| Fence {
            char: char::from(code.fence_char),
            len: code.fence_length,
        }),
        info: code.info.clone(),
        literal: code.literal.clone(),
    }
}

const fn alignment(alignment: TableAlignment) -> Alignment {
    match alignment {
        TableAlignment::None => Alignment::None,
        TableAlignment::Left => Alignment::Left,
        TableAlignment::Center => Alignment::Center,
        TableAlignment::Right => Alignment::Right,
    }
}

const fn alert_type_name(alert: AlertType) -> &'static str {
    match alert {
        AlertType::Note => "NOTE",
        AlertType::Tip => "TIP",
        AlertType::Important => "IMPORTANT",
        AlertType::Warning => "WARNING",
        AlertType::Caution => "CAUTION",
    }
}

/// The comment when `html` (an HTML block or inline literal) is exactly one
/// `<!-- … -->` comment, possibly followed by whitespace.
fn html_comment(html: &str) -> Option<Comment> {
    let inner = html.trim_end().strip_prefix("<!--")?.strip_suffix("-->")?;
    (!inner.contains("-->")).then(|| Comment {
        syntax: CommentSyntax::Html,
        literal: inner.to_owned(),
    })
}

/// The label of a footnote reference `[^label]` or definition `[^label]: …` as written,
/// with `close` the text after the label.
fn footnote_label<'a>(source: &'a str, close: &str) -> Option<&'a str> {
    let rest = source.strip_prefix("[^")?;
    let end = rest.find(']')?;
    rest[end..].starts_with(close).then(|| &rest[..end])
}

/// The type keyword of a GFM alert as written: `NOTE` in `> [!NOTE]`.
fn alert_kind(source: &str) -> Option<&str> {
    let rest = &source[source.find("[!")? + 2..];
    Some(&rest[..rest.find(']')?])
}

/// How the destination of a link (or image, `![…]`) with source `source` was written.
///
/// Inline links end with `)`, references with `]`; the last bracket pair of a reference
/// is its label, and when that pair starts the link it is a shortcut reference. Autolinks
/// are wrapped in `<…>`, and anything else is a GFM bare autolink.
fn link_form(source: &str, image: bool) -> LinkForm {
    let text = if image {
        source.strip_prefix('!').unwrap_or(source)
    } else {
        source
    };
    if text.starts_with('<') {
        return LinkForm::Autolink;
    }
    if !text.starts_with('[') {
        return LinkForm::Bare;
    }
    if text.ends_with(')') {
        return LinkForm::Inline;
    }
    let Some(open) = last_bracket_pair(text) else {
        return LinkForm::Inline;
    };
    let label = &text[open + 1..text.len() - 1];
    let style = if open == 0 {
        ReferenceStyle::Shortcut
    } else if label.is_empty() {
        ReferenceStyle::Collapsed
    } else {
        ReferenceStyle::Full
    };
    let label = if style == ReferenceStyle::Collapsed {
        &text[1..open - 1]
    } else {
        label
    };
    LinkForm::Reference {
        label: label.to_owned(),
        style,
    }
}

/// The offsets of the `(` and of the end of the inline link or image starting at `start`
/// (`[text](dest "title")`), scanned from the source; `None` if it isn't one (a reference
/// or autolink).
fn inline_link_end(src: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = src.as_bytes();
    let mut at = start + usize::from(bytes.get(start) == Some(&b'!'));
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    // The link text: balanced brackets, skipping escapes and code spans.
    let mut depth = 0usize;
    loop {
        match *bytes.get(at)? {
            b'\\' => at += 1,
            b'`' => at = skip_code_span(bytes, at),
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        at += 1;
    }
    at += 1;
    if bytes.get(at) != Some(&b'(') {
        return None;
    }
    let paren = at;
    at = skip_link_space(bytes, at + 1);
    // The destination: `<…>`, or a run without spaces and with balanced parentheses.
    if bytes.get(at) == Some(&b'<') {
        at = find_unescaped(bytes, at + 1, b'>')? + 1;
    } else {
        let mut parens = 0usize;
        while let Some(&byte) = bytes.get(at) {
            match byte {
                b'\\' => at += 1,
                b'(' => parens += 1,
                b')' if parens == 0 => break,
                b')' => parens -= 1,
                _ if byte.is_ascii_whitespace() => break,
                _ => {}
            }
            at += 1;
        }
    }
    at = skip_link_space(bytes, at);
    let close = match bytes.get(at)? {
        b'"' => Some(b'"'),
        b'\'' => Some(b'\''),
        b'(' => Some(b')'),
        _ => None,
    };
    if let Some(close) = close {
        at = skip_link_space(bytes, find_unescaped(bytes, at + 1, close)? + 1);
    }
    (bytes.get(at) == Some(&b')')).then_some((paren, at + 1))
}

/// The offset of the last backtick of the code span opened by the backtick run at `at`,
/// or of the run itself if it is never closed.
fn skip_code_span(bytes: &[u8], at: usize) -> usize {
    let run = bytes[at..].iter().take_while(|&&b| b == b'`').count();
    let mut i = at + run;
    while i < bytes.len() {
        let len = bytes[i..].iter().take_while(|&&b| b == b'`').count();
        if len == run {
            return i + len - 1;
        }
        i += len.max(1);
    }
    at + run - 1
}

/// Skips whitespace in a link destination or title, including line endings and the
/// block quote markers that continue a quoted line.
fn skip_link_space(bytes: &[u8], mut at: usize) -> usize {
    let mut line_start = false;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'\n' | b'\r' => line_start = true,
            b'>' if line_start => {}
            b' ' | b'\t' => {}
            _ => break,
        }
        at += 1;
    }
    at
}

/// The offset of the first `target` byte at or after `at` that isn't backslash-escaped.
fn find_unescaped(bytes: &[u8], mut at: usize, target: u8) -> Option<usize> {
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'\\' => at += 1,
            _ if byte == target => return Some(at),
            _ => {}
        }
        at += 1;
    }
    None
}

/// Offset of the `[` matching the final `]` of `text`, skipping escaped brackets.
fn last_bracket_pair(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let escaped = |i: usize| bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
    let mut depth = 0usize;
    for i in (0..bytes.len()).rev() {
        match bytes[i] {
            b']' if !escaped(i) => depth += 1,
            b'[' if !escaped(i) => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Callout, CommentSyntax, LinkForm, NodeKind, ReferenceStyle};

    fn gfm(src: &str) -> Ast {
        ComrakParser::new(ParseOptions::gfm()).parse(src)
    }

    /// Kinds of the attached nodes in document order, excluding the root.
    fn kinds(ast: &Ast) -> Vec<NodeKind> {
        ast.descendants(ast.root())
            .skip(1)
            .map(|id| ast.kind(id).clone())
            .collect()
    }

    /// The first attached node of type `name` and its source.
    fn find<'a>(ast: &Ast, src: &'a str, name: &str) -> (NodeKind, &'a str) {
        let id = ast
            .descendants(ast.root())
            .find(|&id| ast.kind(id).node_type().as_str() == name)
            .unwrap_or_else(|| panic!("no {name} in\n{}", ast.dump(ast.root())));
        (ast.kind(id).clone(), ast.node(id).source(src))
    }

    #[test]
    fn blocks_convert_with_payloads_and_spans() {
        let src =
            "Title\n===\n\n> - [x] a\n>   b\n\n3) c\n\n```rust x\nfn\n```\n\n    ind\n\n***\n";
        let ast = gfm(src);
        let expected = "\
document 0..65
  heading 0..9 { level: 1, setext: true, attrs: None }
    text 0..5 \"Title\"
  block_quote 11..26
    list 13..26 { kind: Bullet { marker: '-' }, tight: true }
      list_item 13..26 { task: Some(Done) }
        paragraph 19..26
          text 19..20 \"a\"
          soft_break 20..21
          text 25..26 \"b\"
  list 28..32 { kind: Ordered { start: 3, delimiter: Paren }, tight: true }
    list_item 28..32 { task: None }
      paragraph 31..32
        text 31..32 \"c\"
  code_block 34..50 { fence: Some(Fence { char: '`', len: 3 }), info: \"rust x\", literal: \"fn\\n\" }
  code_block 52..59 { fence: None, info: \"\", literal: \"ind\\n\" }
  thematic_break 61..64
";
        assert_eq!(ast.dump(ast.root()), expected);
    }

    #[test]
    fn task_states_follow_the_marker() {
        let ast = gfm("- [ ] a\n- [x] b\n- [X] c\n");
        let tasks: Vec<_> = kinds(&ast)
            .into_iter()
            .filter_map(|kind| match kind {
                NodeKind::ListItem(item) => Some(item.task),
                _ => None,
            })
            .collect();
        assert_eq!(
            tasks,
            [
                Some(TaskState::Open),
                Some(TaskState::Done),
                Some(TaskState::Done)
            ]
        );
    }

    #[test]
    fn tables_carry_alignments_and_header_rows() {
        let src = "| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n";
        let ast = gfm(src);
        let dump = ast.dump(ast.root());
        assert!(
            dump.contains("{ alignments: [Left, Center, Right] }"),
            "{dump}"
        );
        assert!(dump.contains("table_row 0..13 { header: true }"), "{dump}");
        assert_eq!(find(&ast, src, "table_cell").1, " a ");
    }

    #[test]
    fn escaped_pipes_in_table_cells_keep_inline_spans_aligned() {
        let src = "| a |\n| - |\n| x `\\|` **\\|** y\\|z |\n";
        let ast = gfm(src);
        let row = ast.children(ast.children(ast.root())[0])[1];
        let cell = ast.node(ast.children(row)[0]);
        let sources: Vec<_> = cell.children().map(|n| n.source(src)).collect();
        assert_eq!(sources, ["x ", "`\\|`", " ", "**\\|**", " y\\|z"]);
    }

    #[test]
    fn link_forms_are_recovered_from_the_source() {
        let src = "[a](/u \"t\") [b][r] [r][] [r] [x \\[y\\] z] ![i](/p) <http://h.x> www.w.org\n\n\
                   [r]: /ref\n[x \\[y\\] z]: /xyz\n";
        let ast = gfm(src);
        let links: Vec<_> = kinds(&ast)
            .into_iter()
            .filter_map(|kind| match kind {
                NodeKind::Link(link) | NodeKind::Image(link) => Some(link),
                _ => None,
            })
            .collect();
        let reference = |label: &str, style| LinkForm::Reference {
            label: label.into(),
            style,
        };
        let forms: Vec<_> = links.iter().map(|link| link.form.clone()).collect();
        assert_eq!(
            forms,
            [
                LinkForm::Inline,
                reference("r", ReferenceStyle::Full),
                reference("r", ReferenceStyle::Collapsed),
                reference("r", ReferenceStyle::Shortcut),
                reference("x \\[y\\] z", ReferenceStyle::Shortcut),
                LinkForm::Inline,
                LinkForm::Autolink,
                LinkForm::Bare,
            ]
        );
        assert_eq!(links[0].title.as_deref(), Some("t"));
        assert_eq!(links[1].url, "/ref");
        assert_eq!(links[1].title, None);
        assert_eq!(links[7].url, "http://www.w.org");
    }

    #[test]
    fn inline_links_spanning_lines_end_at_their_parenthesis() {
        let src = "x [a `]`](/u\n\"ti\ntle\") b\nc ![i](\n<d e>\n) *f*\n\n> [q](/v\n> 'w') z\n";
        let ast = gfm(src);
        let links: Vec<_> = ast
            .descendants(ast.root())
            .filter(|&id| matches!(ast.kind(id), NodeKind::Link(_) | NodeKind::Image(_)))
            .map(|id| (ast.node(id).source(src), ast.kind(id).clone()))
            .collect();
        assert_eq!(links[0].0, "[a `]`](/u\n\"ti\ntle\")");
        assert_eq!(links[1].0, "![i](\n<d e>\n)");
        assert_eq!(links[2].0, "[q](/v\n> 'w')");
        assert!(links.iter().all(|(_, kind)| matches!(
            kind,
            NodeKind::Link(link) | NodeKind::Image(link) if link.form == LinkForm::Inline
        )));
        let texts: Vec<_> = ast
            .descendants(ast.root())
            .filter(|&id| matches!(ast.kind(id), NodeKind::Text(_)))
            .map(|id| ast.node(id).source(src))
            .collect();
        assert_eq!(texts, ["x ", "a ", " b", "c ", "i", " ", "f", "q", " z"]);
    }

    #[test]
    fn inline_link_ends_are_scanned() {
        fn end(src: &str) -> Option<&str> {
            inline_link_end(src, 0).map(|(_, end)| &src[..end])
        }
        assert_eq!(end("[a](u) x"), Some("[a](u)"));
        assert_eq!(end("![i](\n<d e>\n) *f*"), Some("![i](\n<d e>\n)"));
        assert_eq!(
            end("[a [b] `]` \\]](<u>  'x\\'y' ) z"),
            Some("[a [b] `]` \\]](<u>  'x\\'y' )")
        );
        assert_eq!(end("[a](u(v)w (t)) z"), Some("[a](u(v)w (t))"));
        assert_eq!(end("[a][b] (c)"), None);
        assert_eq!(end("[a](b c)"), None);
        assert_eq!(end("<http://x>"), None);
    }

    #[test]
    fn lone_html_comments_become_comment_nodes() {
        let src = "<!-- block -->\n\n<!-- a --> <b>\n\nx <!-- inline --> <i>\n";
        let ast = gfm(src);
        let kinds = kinds(&ast);
        let comment = |literal: &str| {
            NodeKind::Comment(crate::ast::Comment {
                syntax: CommentSyntax::Html,
                literal: literal.into(),
            })
        };
        assert_eq!(kinds[0], comment(" block "));
        assert_eq!(kinds[1], NodeKind::HtmlBlock("<!-- a --> <b>\n".into()));
        assert!(kinds.contains(&comment(" inline ")), "{kinds:?}");
        assert!(kinds.contains(&NodeKind::HtmlInline("<i>".into())));
        assert_eq!(find(&ast, src, "comment").1, "<!-- block -->");
    }

    #[test]
    fn alerts_become_callouts() {
        let src = "> [!Tip]\n> Body\n";
        let ast = gfm(src);
        let (kind, source) = find(&ast, src, "callout");
        assert_eq!(
            kind,
            NodeKind::Callout(Callout {
                kind: "Tip".into(),
                title: None,
                fold: None,
            })
        );
        assert_eq!(source, src.trim_end());
        assert_eq!(find(&ast, src, "paragraph").1, "Body");
    }

    #[test]
    fn footnotes_stay_in_place_with_labels_as_written() {
        let src = "a[^Note]\n\n[^Note]: Foot.\n\nafter\n";
        let ast = gfm(src);
        let types: Vec<_> = ast
            .children(ast.root())
            .iter()
            .map(|&id| ast.kind(id).node_type().as_str())
            .collect();
        assert_eq!(types, ["paragraph", "footnote_definition", "paragraph"]);
        let (kind, source) = find(&ast, src, "footnote_reference");
        assert_eq!(source, "[^Note]");
        assert!(matches!(kind, NodeKind::FootnoteReference(r) if r.label == "Note"));
        let (kind, source) = find(&ast, src, "footnote_definition");
        assert_eq!(source, "[^Note]: Foot.");
        assert!(matches!(kind, NodeKind::FootnoteDefinition(d) if d.label == "Note"));

        let src = "- [^Empty]:\n\n";
        let ast = gfm(src);
        let (kind, source) = find(&ast, src, "footnote_definition");
        assert_eq!(source, "[^Empty]:");
        assert!(matches!(kind, NodeKind::FootnoteDefinition(d) if d.label == "Empty"));
        assert_eq!(find(&ast, src, "list_item").1, "- [^Empty]:");
    }

    #[test]
    fn inline_extensions_convert() {
        let mut options = ParseOptions::gfm();
        options.highlight = true;
        options.superscript = true;
        options.subscript = true;
        let src = "~~s~~ ==h== ^u^ ~d~ $m$ $$D$$ `c` *e* **s**\\\nx\n";
        let ast = ComrakParser::new(options).parse(src);
        for (name, source) in [
            ("strikethrough", "~~s~~"),
            ("highlight", "==h=="),
            ("superscript", "^u^"),
            ("subscript", "~d~"),
            ("code", "`c`"),
            ("emphasis", "*e*"),
            ("strong", "**s**"),
            ("line_break", "\\\n"),
        ] {
            assert_eq!(find(&ast, src, name).1, source, "{name}");
        }
        let maths: Vec<_> = kinds(&ast)
            .into_iter()
            .filter(|kind| matches!(kind, NodeKind::Math(_)))
            .collect();
        assert_eq!(
            format!("{maths:?}"),
            "[Math(Math { display: false, literal: \"m\" }), \
             Math(Math { display: true, literal: \"D\" })]"
        );
    }

    #[test]
    fn description_lists_convert() {
        let mut options = ParseOptions::commonmark();
        options.description_lists = true;
        let src = "Term\n: Details\n";
        let ast = ComrakParser::new(options).parse(src);
        let types: Vec<_> = ast
            .descendants(ast.root())
            .map(|id| ast.kind(id).node_type().as_str())
            .collect();
        assert_eq!(
            types,
            [
                "document",
                "definition_list",
                "definition_item",
                "definition_term",
                "paragraph",
                "text",
                "definition_details",
                "paragraph",
                "text"
            ]
        );
    }

    #[test]
    fn disabled_extensions_stay_plain_text() {
        let ast = ComrakParser::new(ParseOptions::commonmark()).parse("~~a~~ | b |\n> [!NOTE]\n");
        assert!(
            ast.descendants(ast.root())
                .all(|id| !matches!(ast.kind(id), NodeKind::Strikethrough | NodeKind::Callout(_)))
        );
    }

    #[test]
    fn crlf_spans_exclude_the_line_ending() {
        let src = "# A\r\n\r\nb\r\nc\r\n";
        let ast = gfm(src);
        assert_eq!(find(&ast, src, "heading").1, "# A");
        assert_eq!(find(&ast, src, "paragraph").1, "b\r\nc");
        assert_eq!(find(&ast, src, "soft_break").1, "\r\n");
    }

    #[test]
    fn inlines_after_link_reference_definitions_point_at_their_source() {
        let src = "[foo]: /url\n\"title\" ok\n\n[a]: /a\n[b]: /b\n===\n[a]\n\n> [c]: /c\n> *x*\nlazy é\n\n [d]: /d\n  w\n\n> [e]: /e\n lazy\nmore\n";
        let ast = gfm(src);
        let texts: Vec<_> = ast
            .descendants(ast.root())
            .filter(|&id| matches!(ast.kind(id), NodeKind::Text(_)))
            .map(|id| ast.node(id).source(src))
            .collect();
        assert_eq!(
            texts,
            [
                "\"title\" ok",
                "===",
                "a",
                "x",
                "lazy é",
                "w",
                " lazy",
                "more"
            ]
        );
        assert_eq!(find(&ast, src, "link").1, "[a]");
        assert_eq!(find(&ast, src, "emphasis").1, "*x*");
    }

    #[test]
    fn inlines_after_partly_consumed_tabs_point_at_their_source() {
        let src = "- >`a\n\t`b\n";
        let ast = gfm(src);
        assert_eq!(find(&ast, src, "code").1, "`a\n\t`");
        assert_eq!(find(&ast, src, "text").1, "b");
    }

    #[test]
    fn empty_input_is_an_empty_document() {
        let ast = gfm("");
        assert_eq!(ast.dump(ast.root()), "document 0..0\n");
    }
}
