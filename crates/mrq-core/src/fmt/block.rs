//! The block printer: every block node becomes a list of lines, and containers prefix
//! their children's lines (`> `, list markers and indentation).

use super::Printer;
use crate::ast::{
    Admonition, Callout, CodeBlock, CommentSyntax, Fold, FrontMatterFormat, Heading, List,
    ListKind, NodeId, NodeKind, OrderedDelimiter, TaskState,
};
use crate::config::OrderedList;

/// The largest ordered list number `CommonMark` accepts (nine digits).
const MAX_LIST_NUMBER: u64 = 999_999_999;
const MAX_LIST_DIGITS: usize = 9;

/// Where a block sits among its siblings.
#[derive(Debug, Clone, Copy)]
struct Place {
    /// Its index in its parent.
    index: usize,
    /// Whether its siblings are printed without blank lines (tight list items).
    tight: bool,
    /// The marker of the list just before it, if it follows one.
    previous: Option<Marker>,
    /// The bullet of the list item it is in.
    bullet: Option<char>,
}

/// How a list was printed, so that a list right after it can use a different marker:
/// two adjacent lists with the same marker would merge into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Bullet(char),
    Ordered(OrderedDelimiter),
}

impl Printer<'_> {
    /// The lines of the whole document, reference definitions included.
    pub(super) fn document(&self) -> Vec<String> {
        let mut lines = self.blocks(self.ast.root(), false, None);
        push_block(&mut lines, self.definitions.rest(), false);
        lines
    }

    /// The children of the container `parent`, separated by a blank line unless `tight`.
    /// `bullet` is the marker of the list item `parent`, if it is one.
    fn blocks(&self, parent: NodeId, tight: bool, bullet: Option<char>) -> Vec<String> {
        let mut out = Vec::new();
        let mut previous = None;
        for (index, &child) in self.ast.children(parent).iter().enumerate() {
            // Definitions before a container, or before or inside (at the start of) a
            // leaf block, come first.
            let span = self.ast.span(child);
            let before = if is_container(self.ast.kind(child)) {
                span.start
            } else {
                span.end
            };
            push_block(&mut out, self.definitions.take_before(before), tight);
            let place = Place {
                index,
                tight,
                previous,
                bullet,
            };
            let (lines, marker) = self.block(child, place);
            previous = marker;
            push_block(&mut out, lines, tight);
        }
        // A list item ends with its last child; definitions after that, up to the next
        // item, are still inside it.
        let end = match self.ast.next_sibling(parent) {
            Some(next) if matches!(self.ast.kind(parent), NodeKind::ListItem(_)) => {
                self.ast.span(next).start
            }
            _ => self.ast.span(parent).end,
        };
        push_block(&mut out, self.definitions.take_before(end), tight);
        out
    }

    /// One block at `place`. Lists also return the marker they used.
    fn block(&self, id: NodeId, place: Place) -> (Vec<String>, Option<Marker>) {
        let lines = match self.ast.kind(id) {
            NodeKind::Document => self.blocks(id, false, None),
            NodeKind::FrontMatter(front) => self.verbatim(id).unwrap_or_else(|| {
                let fence = match front.format {
                    FrontMatterFormat::Toml => "+++",
                    _ => "---",
                };
                let mut lines = vec![fence.to_owned()];
                lines.extend(split_lines(&front.literal));
                lines.push(fence.to_owned());
                lines
            }),
            NodeKind::Paragraph | NodeKind::DefinitionTerm if self.is_inline_parent(id) => {
                split_lines(&self.inlines(id, false))
            }
            NodeKind::Heading(heading) => self.heading(id, heading),
            NodeKind::ThematicBreak => {
                let after_paragraph = place.tight
                    && self
                        .ast
                        .prev_sibling(id)
                        .is_some_and(|prev| matches!(self.ast.kind(prev), NodeKind::Paragraph));
                // `- ---` is a thematic break, not an item; `foo\n---` a setext heading.
                let rule = if after_paragraph || (place.index == 0 && place.bullet == Some('-')) {
                    "***"
                } else {
                    "---"
                };
                vec![rule.to_owned()]
            }
            NodeKind::BlockQuote => quote(self.blocks(id, false, None)),
            NodeKind::Callout(callout) => {
                let mut lines = vec![callout_marker(callout)];
                lines.extend(self.blocks(id, false, None));
                quote(lines)
            }
            NodeKind::List(list) => {
                let (lines, marker) = self.list(id, list, place.previous);
                return (lines, Some(marker));
            }
            NodeKind::ListItem(_) => self.item(id, "-", true, Some('-')),
            NodeKind::CodeBlock(code) => self.code_block(code),
            NodeKind::HtmlBlock(html) => split_lines(html),
            NodeKind::Table(table) => self.table(id, &table.alignments),
            NodeKind::FootnoteDefinition(def) => {
                let marker = format!("[^{}]:", def.label);
                indent(&marker, "    ", self.blocks(id, false, None))
            }
            NodeKind::LinkDefinition(def) => {
                vec![super::refs::definition(
                    &def.label,
                    &def.url,
                    def.title.as_deref(),
                )]
            }
            NodeKind::DefinitionList => {
                let mut lines = Vec::new();
                for &item in self.ast.children(id) {
                    push_block(&mut lines, self.blocks(item, true, None), false);
                }
                lines
            }
            NodeKind::DefinitionDetails => indent(":", "    ", self.blocks(id, false, None)),
            NodeKind::Admonition(admonition) => {
                let header = admonition_header(admonition);
                let body = self.blocks(id, false, None);
                let mut lines = vec![header];
                lines.extend(indent_all("    ", body));
                lines
            }
            NodeKind::AbbreviationDefinition(abbr) => {
                vec![format!("*[{}]: {}", abbr.abbr, abbr.expansion)]
            }
            NodeKind::IncludeDirective(_) | NodeKind::TocDirective(_) | NodeKind::DirectiveEnd => {
                self.verbatim(id).unwrap_or_default()
            }
            NodeKind::Comment(comment) => split_lines(&match comment.syntax {
                CommentSyntax::Percent => format!("%%{}%%", comment.literal),
                _ => format!("<!--{}-->", comment.literal),
            }),
            kind if kind.is_inline() => split_lines(&self.inline_block(id)),
            // Rows and cells outside a table, terms and items outside a definition list,
            // paragraphs holding blocks: print what they contain.
            _ => self.blocks(id, false, None),
        };
        (lines, None)
    }

    /// Whether the children of `id` are inlines (or it has none).
    fn is_inline_parent(&self, id: NodeId) -> bool {
        self.ast
            .children(id)
            .iter()
            .all(|&child| self.ast.kind(child).is_inline())
    }

    /// The source of `id`, line by line, if it has any.
    fn verbatim(&self, id: NodeId) -> Option<Vec<String>> {
        let span = self.ast.span(id);
        let text = self.src.get(span.start..span.end)?;
        (!text.is_empty()).then(|| split_lines(text))
    }

    /// An ATX heading, or a setext one when its text spans lines.
    fn heading(&self, id: NodeId, heading: &Heading) -> Vec<String> {
        let level = heading.level;
        let mut text = self.inlines(id, false);
        // mkdocs `{#id .class}` attributes, as written (M6.5 normalizes them).
        if let Some(attrs) = &heading.attrs
            && let Some(source) = self.src.get(attrs.span.start..attrs.span.end)
            && !source.is_empty()
        {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(source);
        }
        if text.contains('\n') && level <= 2 {
            let mut lines = split_lines(&text);
            lines.push(if level == 1 { "===" } else { "---" }.to_owned());
            return lines;
        }
        // A trailing `#` would be read as a closing sequence.
        if text.ends_with('#') && !text.ends_with("\\#") {
            text.insert(text.len() - 1, '\\');
        }
        let hashes = "#".repeat(usize::from(level.clamp(1, 6)));
        if text.is_empty() {
            vec![hashes]
        } else {
            vec![format!("{hashes} {text}")]
        }
    }

    /// A list, with a marker unlike that of an adjacent `previous` list.
    fn list(&self, id: NodeId, list: &List, previous: Option<Marker>) -> (Vec<String>, Marker) {
        let marker = match list.kind {
            ListKind::Bullet { .. } => {
                let preferred = first_char(self.config.list_marker.as_str());
                Marker::Bullet(if previous == Some(Marker::Bullet(preferred)) {
                    ['-', '*', '+']
                        .into_iter()
                        .find(|&c| c != preferred)
                        .unwrap_or(preferred)
                } else {
                    preferred
                })
            }
            ListKind::Ordered { delimiter, .. } => {
                Marker::Ordered(if previous == Some(Marker::Ordered(delimiter)) {
                    match delimiter {
                        OrderedDelimiter::Period => OrderedDelimiter::Paren,
                        OrderedDelimiter::Paren => OrderedDelimiter::Period,
                    }
                } else {
                    delimiter
                })
            }
        };
        let mut lines = Vec::new();
        for (index, &item) in self.ast.children(id).iter().enumerate() {
            let item_lines = match marker {
                Marker::Bullet(c) => self.item(item, &c.to_string(), list.tight, Some(c)),
                Marker::Ordered(delimiter) => {
                    let start = match list.kind {
                        ListKind::Ordered { start, .. } => start,
                        ListKind::Bullet { .. } => 1,
                    };
                    let number = self.item_number(item, start, index);
                    let delimiter = match delimiter {
                        OrderedDelimiter::Period => '.',
                        OrderedDelimiter::Paren => ')',
                    };
                    self.item(item, &format!("{number}{delimiter}"), list.tight, None)
                }
            };
            push_block(&mut lines, item_lines, list.tight);
        }
        (lines, marker)
    }

    /// The number of the `index`th item of an ordered list starting at `start`: the
    /// start number for every item (`one`), counting up from it (`ordered`), or the
    /// digits as written, leading zeros included (`preserve`). Only the start number
    /// changes the HTML.
    fn item_number(&self, item: NodeId, start: u64, index: usize) -> String {
        let consecutive = start.saturating_add(index as u64);
        let number = match self.config.ordered_list {
            OrderedList::One => start,
            OrderedList::Ordered => consecutive,
            OrderedList::Preserve => {
                let span = self.ast.span(item);
                let digits: String = self
                    .src
                    .get(span.start..span.end)
                    .unwrap_or_default()
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                if (1..=MAX_LIST_DIGITS).contains(&digits.len()) {
                    return digits;
                }
                consecutive
            }
        };
        // A tenth digit would end the list.
        if number > MAX_LIST_NUMBER {
            start.to_string()
        } else {
            number.to_string()
        }
    }

    /// A list item with `marker` (`-`, `1.`); content is indented by the marker width.
    fn item(&self, id: NodeId, marker: &str, tight: bool, bullet: Option<char>) -> Vec<String> {
        let mut body = self.blocks(id, tight, bullet);
        if let NodeKind::ListItem(item) = self.ast.kind(id)
            && let Some(task) = item.task
        {
            let state = match task {
                TaskState::Open => ' ',
                TaskState::Done => 'x',
                TaskState::Other(c) => c,
            };
            let task = format!("[{state}]");
            match body.first_mut() {
                Some(first) if !first.is_empty() => *first = format!("{task} {first}"),
                Some(first) => *first = task,
                None => body.push(task),
            }
        }
        let width = marker.chars().count() + 1;
        indent(marker, &" ".repeat(width), body)
    }

    /// A fenced code block: the configured fence, unless the info string has a backtick,
    /// longer than any fence-like line of the code.
    fn code_block(&self, code: &CodeBlock) -> Vec<String> {
        let mut fence_char = first_char(self.config.code_fence.as_str());
        if fence_char == '`' && code.info.contains('`') {
            fence_char = '~';
        }
        let longest = code
            .literal
            .lines()
            .map(|line| {
                line.trim_start()
                    .chars()
                    .take_while(|&c| c == fence_char)
                    .count()
            })
            .max()
            .unwrap_or(0);
        let fence = fence_char.to_string().repeat((longest + 1).max(3));
        let mut lines = vec![format!("{fence}{}", super::inline::info(&code.info))];
        lines.extend(split_lines(&code.literal));
        lines.push(fence);
        lines
    }
}

/// Whether the block `kind` holds blocks of its own; reference definitions inside it are
/// placed by its own children.
const fn is_container(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::BlockQuote
            | NodeKind::Callout(_)
            | NodeKind::List(_)
            | NodeKind::ListItem(_)
            | NodeKind::FootnoteDefinition(_)
            | NodeKind::Admonition(_)
            | NodeKind::DefinitionList
            | NodeKind::DefinitionItem
            | NodeKind::DefinitionDetails
    )
}

/// Appends a block's `lines` to `out`, after a blank line unless `tight` or first.
fn push_block(out: &mut Vec<String>, lines: Vec<String>, tight: bool) {
    if lines.is_empty() {
        return;
    }
    if !out.is_empty() && !tight {
        out.push(String::new());
    }
    out.extend(lines);
}

/// `text` split into lines without their line endings; nothing for empty text, and a
/// final line ending doesn't start another line.
fn split_lines(text: &str) -> Vec<String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect()
}

/// `lines` inside a block quote.
fn quote(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        return vec![">".to_owned()];
    }
    lines
        .into_iter()
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect()
}

/// `lines` after `marker` on the first line and indented by `rest` on the others; empty
/// lines stay empty.
fn indent(marker: &str, rest: &str, lines: Vec<String>) -> Vec<String> {
    let mut lines = lines.into_iter();
    let mut out = match lines.next() {
        Some(first) if !first.is_empty() => vec![format!("{marker} {first}")],
        _ => vec![marker.to_owned()],
    };
    out.extend(indent_all(rest, lines));
    out
}

/// Every line of `lines` indented by `prefix`; empty lines stay empty.
fn indent_all(prefix: &str, lines: impl IntoIterator<Item = String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| {
            if line.is_empty() {
                line
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect()
}

/// `[!kind]`, its fold sign and title, the first line of a callout.
fn callout_marker(callout: &Callout) -> String {
    let fold = match callout.fold {
        Some(Fold::Open) => "+",
        Some(Fold::Closed) => "-",
        None => "",
    };
    match &callout.title {
        Some(title) if !title.is_empty() => format!("[!{}]{fold} {title}", callout.kind),
        _ => format!("[!{}]{fold}", callout.kind),
    }
}

/// `!!! kind classes "title"`, the first line of an admonition.
fn admonition_header(admonition: &Admonition) -> String {
    let mut header = match admonition.fold {
        None => "!!!",
        Some(Fold::Closed) => "???",
        Some(Fold::Open) => "???+",
    }
    .to_owned();
    for word in std::iter::once(&admonition.kind).chain(&admonition.classes) {
        header.push(' ');
        header.push_str(word);
    }
    if let Some(title) = &admonition.title {
        header.push_str(" \"");
        header.push_str(title);
        header.push('"');
    }
    header
}

fn first_char(s: &str) -> char {
    s.chars().next().unwrap_or('-')
}
