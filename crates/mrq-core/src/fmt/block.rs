//! The block printer: every block node becomes a list of lines, and containers prefix
//! their children's lines (`> `, list markers and indentation).

use super::wrap::Room;
use super::{Piece, Printer, codes};
use crate::ast::{
    Admonition, Callout, CodeBlock, CommentSyntax, Fold, FrontMatter, FrontMatterFormat, Heading,
    HeadingAttrs, List, ListKind, NodeId, NodeKind, OrderedDelimiter, Span, TaskState,
};
use crate::config::{FrontmatterFormat, OrderedList};

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
    /// The columns it may fill.
    room: Room,
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
        let room = Room::new(usize::from(self.config.width));
        let mut lines = self.blocks(self.ast.root(), false, None, room);
        push_block(&mut lines, self.definitions.rest(), false);
        lines
    }

    /// The top-level blocks and the definitions between them, each printed alone; see
    /// [`pieces`](super::pieces). Definitions after the last block are kept, those for
    /// labels without one are not (links show their destinations).
    pub(super) fn pieces(&self) -> Vec<Piece> {
        let room = Room::new(usize::from(self.config.width));
        let mut out = Vec::new();
        for &child in self.ast.children(self.ast.root()) {
            let span = self.ast.span(child);
            let before = if is_container(self.ast.kind(child)) {
                span.start
            } else {
                span.end
            };
            self.push_definitions(&mut out, before);
            let place = Place {
                index: 1,
                tight: false,
                previous: None,
                bullet: None,
                room,
            };
            out.push(Piece::Block(child, self.block(child, place).0));
        }
        self.push_definitions(&mut out, usize::MAX);
        out
    }

    /// Appends the definitions not handed out yet that start before `offset` to `out`.
    fn push_definitions(&self, out: &mut Vec<Piece>, offset: usize) {
        let definitions = self.definitions.take_placed_before(offset);
        out.extend(
            definitions
                .into_iter()
                .map(|(span, line)| Piece::Definition(span, line)),
        );
    }

    /// The children of the container `parent`, separated by a blank line unless `tight`,
    /// in `room`. `bullet` is the marker of the list item `parent`, if it is one.
    fn blocks(&self, parent: NodeId, tight: bool, bullet: Option<char>, room: Room) -> Vec<String> {
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
            let definitions = self.definitions.take_before(before);
            // A blank line after definitions where the block after them could be read
            // as part of the last one; the source had one there too.
            let after_definitions = !definitions.is_empty() && self.continues_definition(child);
            let prev = self.ast.prev_sibling(child);
            push_block(&mut out, definitions, tight && !self.continues(prev));
            let place = Place {
                index,
                tight,
                previous,
                bullet,
                room: if out.is_empty() { room } else { room.rest() },
            };
            let (lines, marker) = self.block(child, place);
            previous = marker;
            // Without a blank line, a paragraph right after a quote would continue it.
            let after_quote = self.ast.prev_sibling(child).is_some_and(|prev| {
                matches!(
                    self.ast.kind(prev),
                    NodeKind::BlockQuote | NodeKind::Callout(_)
                )
            });
            if tight && after_quote && matches!(self.ast.kind(child), NodeKind::Paragraph) {
                out.push(">".to_owned());
            }
            push_block(
                &mut out,
                lines,
                self.joins_previous(child, tight) && !after_definitions,
            );
        }
        // A list item ends with its last child; definitions after that, up to the next
        // item, are still inside it.
        let item = matches!(self.ast.kind(parent), NodeKind::ListItem(_));
        let end = match self.ast.next_sibling(parent) {
            Some(next) if item => self.ast.span(next).start,
            _ if item => self.last_item_end(self.ast.span(parent)),
            _ => self.ast.span(parent).end,
        };
        let last = self.ast.children(parent).last().copied();
        push_block(
            &mut out,
            self.definitions.take_before(end),
            tight && !self.continues(last),
        );
        out
    }

    /// Whether `block`, printed right after a reference definition, could be read as
    /// part of it: anything but a block that interrupts a paragraph (a table as a row
    /// of it, a paragraph starting with a quote as its title, a list starting at 3 as
    /// its text, `---` as its setext underline).
    fn continues_definition(&self, block: NodeId) -> bool {
        !matches!(
            self.ast.kind(block),
            NodeKind::Heading(_)
                | NodeKind::CodeBlock(_)
                | NodeKind::BlockQuote
                | NodeKind::Callout(_)
        )
    }

    /// Whether the block `prev` would take in definitions right after it, which the
    /// source then had a blank line before: as lines of a paragraph (also lazily, at
    /// the end of a container), of a table or of an HTML block.
    fn continues(&self, prev: Option<NodeId>) -> bool {
        prev.is_some_and(|prev| {
            !matches!(
                self.ast.kind(prev),
                NodeKind::Heading(_)
                    | NodeKind::ThematicBreak
                    | NodeKind::CodeBlock(_)
                    | NodeKind::FrontMatter(_)
            )
        })
    }

    /// Where the last item of a list, at `span`, ends for the definitions in it. An item
    /// whose content ends in definitions ends at its last child, or at its marker when
    /// it holds nothing else; the definitions on the line of its marker and on the lines
    /// indented to its content that follow are still in it.
    fn last_item_end(&self, span: Span) -> usize {
        let src = self.src;
        let line_start = src[..span.start].rfind('\n').map_or(0, |nl| nl + 1);
        let first_end = src[span.start..]
            .find('\n')
            .map_or(src.len(), |nl| span.start + nl);
        let first = &src[span.start..first_end];
        let marker = first
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '*' | '+' | '.' | ')')))
            .unwrap_or(first.len());
        let spaces = first[marker..].len() - first[marker..].trim_start_matches(' ').len();
        let spaces = if (1..=4).contains(&spaces) { spaces } else { 1 };
        let content = span.start - line_start + marker + spaces;
        let mut end = first_end;
        let mut at = first_end;
        while at < src.len() {
            let line_end = src[at + 1..].find('\n').map_or(src.len(), |nl| at + 1 + nl);
            let line = src[at + 1..line_end].trim_end_matches('\r');
            if !line.trim().is_empty() {
                if line.len() - line.trim_start_matches(' ').len() < content {
                    break;
                }
                end = line_end;
            }
            at = line_end;
        }
        span.end.max(end)
    }

    /// Whether `id` follows its previous sibling without a blank line: in a tight
    /// container, and where the source had none before definition details (which a
    /// blank line makes loose) and between an abbreviation definition and the
    /// paragraph or definition it was cut from.
    fn joins_previous(&self, id: NodeId, tight: bool) -> bool {
        let Some(prev) = self.ast.prev_sibling(id) else {
            return tight;
        };
        match (self.ast.kind(prev), self.ast.kind(id)) {
            (_, NodeKind::DefinitionDetails) => tight && !self.blank_line_between(prev, id),
            // A `\` ending a definition would be a hard break before the next line.
            (
                NodeKind::AbbreviationDefinition(abbr),
                NodeKind::AbbreviationDefinition(_) | NodeKind::Paragraph,
            ) if abbr.expansion.ends_with('\\') => tight,
            (
                NodeKind::AbbreviationDefinition(_),
                NodeKind::AbbreviationDefinition(_) | NodeKind::Paragraph,
            )
            | (NodeKind::Paragraph, NodeKind::AbbreviationDefinition(_)) => {
                tight || !self.blank_line_between(prev, id)
            }
            _ => tight,
        }
    }

    /// Whether the source has a blank line (or one of only quote markers) between the
    /// end of `prev` and the start of `next`.
    fn blank_line_between(&self, prev: NodeId, next: NodeId) -> bool {
        let gap = self
            .src
            .get(self.ast.span(prev).end..self.ast.span(next).start)
            .unwrap_or_default();
        // The first piece ends `prev`'s line, the last one is the prefix of `next`'s.
        let mut lines = gap.split('\n');
        lines.next();
        lines.rev().skip(1).any(|line| {
            line.trim_matches(|c: char| c == '>' || c.is_whitespace())
                .is_empty()
        })
    }

    /// One block at `place`. Lists also return the marker they used.
    fn block(&self, id: NodeId, place: Place) -> (Vec<String>, Option<Marker>) {
        if self.is_verbatim(id) {
            let span = self.ast.span(id);
            let source = self.src.get(span.start..span.end).unwrap_or_default();
            return (split_lines(&self.unprefixed(source, span.start)), None);
        }
        let lines = match self.ast.kind(id) {
            NodeKind::Document => self.blocks(id, false, None, place.room),
            NodeKind::FrontMatter(front) => self.frontmatter(id, front),
            NodeKind::Paragraph if self.is_inline_parent(id) => self.paragraph(id, place.room),
            NodeKind::DefinitionTerm if self.is_inline_parent(id) => {
                split_lines(&self.inlines(id, false))
            }
            NodeKind::DefinitionTerm => self.definition_term(id, place),
            NodeKind::Heading(heading) => self.heading(id, heading),
            NodeKind::ThematicBreak => vec![self.thematic_break(id, place).to_owned()],
            NodeKind::BlockQuote => quote(self.blocks(id, false, None, place.room.inside(2, 2))),
            NodeKind::Callout(callout) => self.callout(id, callout, place.room),
            NodeKind::List(list) => {
                let (lines, marker) = self.list(id, list, place);
                return (lines, Some(marker));
            }
            NodeKind::ListItem(_) => self.item(id, "-", true, Some('-'), place.room),
            NodeKind::CodeBlock(code) => self.code_block(id, code),
            NodeKind::HtmlBlock(html) => {
                // An unclosed block in a container ends with the blank lines before the
                // container's end; the blank line after the block stands for them.
                let mut lines = split_lines(html);
                while lines.last().is_some_and(|line| line.trim().is_empty()) {
                    lines.pop();
                }
                lines
            }
            NodeKind::Table(table) => self.table(id, &table.alignments),
            NodeKind::FootnoteDefinition(def) => {
                let marker = format!("[^{}]:", def.label);
                let room = place.room.inside(marker.chars().count() + 1, 4);
                indent(&marker, "    ", self.blocks(id, false, None, room))
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
                    push_block(&mut lines, self.blocks(item, true, None, place.room), false);
                }
                lines
            }
            NodeKind::DefinitionDetails => self.definition_details(id, place.room),
            NodeKind::Admonition(admonition) => {
                let header = admonition_header(admonition);
                let body = self.blocks(id, false, None, place.room.inside(4, 4).rest());
                let mut lines = vec![header];
                lines.extend(indent_all("    ", body));
                lines
            }
            NodeKind::AbbreviationDefinition(abbr) => {
                let line = format!("*[{}]: {}", abbr.abbr, abbr.expansion);
                vec![line.trim_end().to_owned()]
            }
            NodeKind::IncludeDirective(_) | NodeKind::TocDirective(_) | NodeKind::DirectiveEnd => {
                self.verbatim(id).unwrap_or_default()
            }
            NodeKind::Comment(comment) => split_lines(&match comment.syntax {
                // The literal is the source between the delimiters, prefixes included.
                CommentSyntax::Percent => {
                    let literal = self.unprefixed(&comment.literal, self.ast.span(id).start);
                    format!("%%{literal}%%")
                }
                _ => format!("<!--{}-->", comment.literal),
            }),
            kind if kind.is_inline() => split_lines(&self.inline_block(id)),
            // Rows and cells outside a table, terms and items outside a definition list,
            // paragraphs holding blocks: print what they contain.
            _ => self.blocks(id, false, None, place.room),
        };
        (lines, None)
    }

    /// `---`, or `***` where that would be read as something else: `- ---` is a
    /// thematic break, not an item; `foo\n---` a setext heading (an admonition header
    /// is a paragraph line to `CommonMark`); `---` first in the document opens
    /// frontmatter.
    fn thematic_break(&self, id: NodeId, place: Place) -> &'static str {
        let after_paragraph = place.tight
            && self.ast.prev_sibling(id).is_some_and(|prev| {
                matches!(
                    self.ast.kind(prev),
                    NodeKind::Paragraph | NodeKind::Admonition(_)
                )
            });
        let first_in_document = place.index == 0 && self.ast.parent(id) == Some(self.ast.root());
        if after_paragraph || first_in_document || (place.index == 0 && place.bullet == Some('-')) {
            "***"
        } else {
            "---"
        }
    }

    /// `> [!kind]± title` and the body. The marker line is the start of a paragraph:
    /// what follows it must continue it or interrupt it, else an empty line comes first.
    fn callout(&self, id: NodeId, callout: &Callout, room: Room) -> Vec<String> {
        let mut lines = vec![callout_marker(callout)];
        if !self
            .ast
            .children(id)
            .first()
            .is_none_or(|&first| self.interrupts_paragraph(first))
        {
            lines.push(String::new());
        }
        lines.extend(self.blocks(id, false, None, room.inside(2, 2).rest()));
        quote(lines)
    }

    /// A definition term that holds blocks. Each line of a term is a term of its own
    /// in Python-Markdown, so its paragraphs are never wrapped.
    fn definition_term(&self, id: NodeId, place: Place) -> Vec<String> {
        let mut lines = Vec::new();
        for &child in self.ast.children(id) {
            let block =
                if self.ast.kind(child) == &NodeKind::Paragraph && self.is_inline_parent(child) {
                    split_lines(&self.inlines(child, false))
                } else {
                    self.block(child, Place { index: 0, ..place }).0
                };
            push_block(&mut lines, block, false);
        }
        lines
    }

    /// `:` and three spaces, so that the content lines up with the 4-space indentation
    /// of its continuation, as Python-Markdown wants it. Empty details keep one space:
    /// a bare `:` is text.
    fn definition_details(&self, id: NodeId, room: Room) -> Vec<String> {
        let room = room.inside(4, 4);
        let mut lines = indent(":  ", "    ", self.blocks(id, false, None, room));
        if lines[0] == ":" {
            lines[0].push(' ');
        }
        lines
    }

    /// Whether the block `id`, printed right below a paragraph line, continues the
    /// paragraph (another paragraph) or interrupts it as itself.
    fn interrupts_paragraph(&self, id: NodeId) -> bool {
        match self.ast.kind(id) {
            NodeKind::Paragraph
            | NodeKind::BlockQuote
            | NodeKind::Callout(_)
            | NodeKind::Heading(_)
            | NodeKind::CodeBlock(_) => true,
            // Only lists starting at 1 whose first item is not empty.
            NodeKind::List(list) => {
                matches!(
                    list.kind,
                    ListKind::Bullet { .. } | ListKind::Ordered { start: 1, .. }
                ) && self
                    .ast
                    .children(id)
                    .first()
                    .is_some_and(|&item| !self.ast.children(item).is_empty())
            }
            _ => false,
        }
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
        // mkdocs `{#id .class}` attributes; as written if they cannot be normalized.
        if let Some(attrs) = &heading.attrs {
            let list = attribute_list(attrs).or_else(|| {
                self.src
                    .get(attrs.span.start..attrs.span.end)
                    .filter(|source| !source.is_empty())
                    .map(str::to_owned)
            });
            if let Some(list) = list {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&list);
            }
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

    /// A list at `place`, with a marker unlike that of an adjacent previous list, and
    /// one that does not make an item's first line a thematic break (`- - -` for nested
    /// empty lists).
    fn list(&self, id: NodeId, list: &List, place: Place) -> (Vec<String>, Marker) {
        let previous = place.previous;
        let candidates: Vec<Marker> = match list.kind {
            ListKind::Bullet { .. } => {
                let preferred = first_char(self.config.list_marker.as_str());
                std::iter::once(preferred)
                    .chain(['-', '*', '+'].into_iter().filter(|&c| c != preferred))
                    .map(Marker::Bullet)
                    .filter(|&marker| previous != Some(marker))
                    .collect()
            }
            ListKind::Ordered { delimiter, .. } => {
                vec![Marker::Ordered(
                    if previous == Some(Marker::Ordered(delimiter)) {
                        match delimiter {
                            OrderedDelimiter::Period => OrderedDelimiter::Paren,
                            OrderedDelimiter::Paren => OrderedDelimiter::Period,
                        }
                    } else if self.for_diff {
                        OrderedDelimiter::Period
                    } else {
                        delimiter
                    },
                )]
            }
        };
        let mut fallback = None;
        for &marker in &candidates {
            let (lines, thematic) = self.items(id, list, marker, place.room);
            if !thematic {
                return (lines, marker);
            }
            fallback.get_or_insert((lines, marker));
        }
        fallback.unwrap_or((Vec::new(), Marker::Bullet('-')))
    }

    /// The items of a list with `marker`, and whether the first line of one of them is a
    /// thematic break.
    fn items(&self, id: NodeId, list: &List, marker: Marker, room: Room) -> (Vec<String>, bool) {
        let mut lines = Vec::new();
        let mut thematic = false;
        for (index, &item) in self.ast.children(id).iter().enumerate() {
            let room = if index == 0 { room } else { room.rest() };
            let item_lines = match marker {
                Marker::Bullet(c) => self.item(item, &c.to_string(), list.tight, Some(c), room),
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
                    self.item(
                        item,
                        &format!("{number}{delimiter}"),
                        list.tight,
                        None,
                        room,
                    )
                }
            };
            thematic |= item_lines
                .first()
                .is_some_and(|line| is_thematic_break(line));
            push_block(&mut lines, item_lines, list.tight);
        }
        (lines, thematic)
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

    /// A list item with `marker` (`-`, `1.`) in `room`; content is indented by the
    /// marker width.
    fn item(
        &self,
        id: NodeId,
        marker: &str,
        tight: bool,
        bullet: Option<char>,
        room: Room,
    ) -> Vec<String> {
        let width = marker.chars().count() + 1;
        let task = match self.ast.kind(id) {
            NodeKind::ListItem(item) => item.task,
            _ => None,
        };
        // `[ ] ` takes room on the first line.
        let lead = if task.is_some() { 4 } else { 0 };
        let mut body = self.blocks(id, tight, bullet, room.inside(width + lead, width));
        if let Some(task) = task {
            let state = match task {
                TaskState::Open => ' ',
                TaskState::Done => 'x',
                TaskState::Other(c) => c,
            };
            let task = format!("[{state}]");
            // Only a paragraph can start on the line of the task marker.
            let paragraph = self
                .ast
                .children(id)
                .first()
                .is_some_and(|&first| matches!(self.ast.kind(first), NodeKind::Paragraph));
            match body.first_mut() {
                Some(first) if first.is_empty() => *first = task,
                Some(first) if paragraph => *first = format!("{task} {first}"),
                Some(_) => {
                    let blocks = std::mem::take(&mut body);
                    body.push(task);
                    push_block(&mut body, blocks, tight);
                }
                None => body.push(task),
            }
        }
        indent(marker, &" ".repeat(width), body)
    }

    /// Frontmatter: verbatim, or normalized with `fmt.frontmatter: normalize` where that
    /// keeps its data (verbatim with a warning where not).
    fn frontmatter(&self, id: NodeId, front: &FrontMatter) -> Vec<String> {
        let normalized = (self.config.frontmatter == FrontmatterFormat::Normalize)
            .then(|| super::frontmatter::normalize(front))
            .and_then(|result| {
                result
                    .map_err(|reason| {
                        let message = format!("frontmatter left as written: {reason}");
                        self.warn(codes::FRONTMATTER_NORMALIZE, self.ast.span(id), message);
                    })
                    .ok()
            });
        if normalized.is_none()
            && let Some(lines) = self.verbatim(id)
        {
            return lines;
        }
        let fence = match front.format {
            FrontMatterFormat::Toml => "+++",
            _ => "---",
        };
        let mut lines = vec![fence.to_owned()];
        lines.extend(split_lines(normalized.as_deref().unwrap_or(&front.literal)));
        lines.push(fence.to_owned());
        lines
    }

    /// A fenced code block, its code formatted if there is a formatter for its language:
    /// the configured fence, unless the info string has a backtick, longer than any
    /// fence-like line of the code.
    fn code_block(&self, id: NodeId, code: &CodeBlock) -> Vec<String> {
        let formatted =
            code.lang()
                .and_then(|lang| match self.code.format(lang, &code.literal)? {
                    Ok(mut formatted) => {
                        if !formatted.is_empty() && !formatted.ends_with('\n') {
                            formatted.push('\n');
                        }
                        Some(formatted)
                    }
                    Err(err) => {
                        let message = format!("`{lang}` code left as written: {err}");
                        self.warn(codes::CODE_FORMATTER, self.ast.span(id), message);
                        None
                    }
                });
        let literal = formatted.as_deref().unwrap_or(&code.literal);
        let mut fence_char = first_char(self.config.code_fence.as_str());
        if fence_char == '`' && code.info.contains('`') {
            fence_char = '~';
        }
        let longest = literal
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
        // Every line of the code counts, a single empty one too.
        if !literal.is_empty() {
            let literal = literal.strip_suffix('\n').unwrap_or(literal);
            let literal = literal.strip_suffix('\r').unwrap_or(literal);
            lines.extend(
                literal
                    .split('\n')
                    .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned()),
            );
        }
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

/// Whether `line` is a thematic break: three or more `-`, `*` or `_`, all the same,
/// and blanks.
fn is_thematic_break(line: &str) -> bool {
    let line = line.trim_start();
    line.chars().next().is_some_and(|c| {
        matches!(c, '-' | '*' | '_')
            && line.chars().all(|d| d == c || d == ' ' || d == '\t')
            && line.chars().filter(|&d| d == c).count() >= 3
    })
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
pub(super) fn split_lines(text: &str) -> Vec<String> {
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
        _ => vec![marker.trim_end().to_owned()],
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

/// `{#id .class key=value}`, the attribute list of a heading: the id, the classes,
/// then the other attributes, each in the shortest form that reads back the same (a bare
/// word for `w=w`, values quoted only where needed). `None` if a value has both kinds of
/// quotes, which no form can hold.
fn attribute_list(attrs: &HeadingAttrs) -> Option<String> {
    // A token that reads back as itself: no blanks, `=`, quotes or braces.
    let plain = |s: &str| {
        !s.is_empty()
            && !s.contains(|c: char| c.is_whitespace() || matches!(c, '=' | '"' | '\'' | '{' | '}'))
    };
    let value = |v: &str| {
        if plain(v) {
            Some(v.to_owned())
        } else if !v.contains('"') {
            Some(format!("\"{v}\""))
        } else if !v.contains('\'') {
            Some(format!("'{v}'"))
        } else {
            None
        }
    };
    let mut tokens = Vec::new();
    if let Some(id) = &attrs.id {
        tokens.push(if plain(id) {
            format!("#{id}")
        } else {
            format!("id={}", value(id)?)
        });
    }
    for class in &attrs.classes {
        tokens.push(if plain(class) {
            format!(".{class}")
        } else {
            format!("class={}", value(class)?)
        });
    }
    for (key, v) in &attrs.attributes {
        if key.is_empty() || key.contains(|c: char| c.is_whitespace() || c == '}') {
            return None;
        }
        tokens.push(if key == v && plain(key) && !key.starts_with(['#', '.']) {
            key.clone()
        } else {
            format!("{key}={}", value(v)?)
        });
    }
    Some(format!("{{{}}}", tokens.join(" ")))
}

fn first_char(s: &str) -> char {
    s.chars().next().unwrap_or('-')
}
