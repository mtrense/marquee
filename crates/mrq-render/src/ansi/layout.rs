//! The tree walk: one [`Layout`] per rendered document, laying blocks out as lines at a
//! width, containers prefixing the lines of their content, then the footnotes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use mrq_core::Document;
use mrq_core::ast::{
    Admonition, Ast, Callout, CodeBlock, Fold, Heading, List, ListKind, NodeId, NodeKind,
    OrderedDelimiter, TaskState, WikiLink,
};
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::DocumentOptions;
use mrq_core::validate::Target;

use super::table::{Row, table};
use super::wrap::{Piece, break_line, char_width, clean, push_char, push_span, wrap};
use super::{AnsiRenderer, BlockInfo, Graphic, HeadingInfo, Image, Line, StyledSpan, TextStyle};
use crate::diagram::{DiagramFormat, DiagramOutput, codes};
use crate::html::render::{IMAGE_EXTENSIONS, default_title, footnote_key};

/// Bullets by list depth.
const BULLETS: [&str; 3] = ["•", "◦", "▪"];

/// Columns per tab stop in code.
const TAB: usize = 4;

/// The lines of `doc`, and the warnings of its diagrams.
pub(super) fn document(renderer: &AnsiRenderer, doc: &Document) -> (Vec<Line>, Vec<Diagnostic>) {
    let mut layout = Layout::new(renderer, doc);
    let ast = doc.ast();
    for id in ast.descendants(ast.root()) {
        if let NodeKind::FootnoteDefinition(def) = ast.kind(id) {
            layout
                .footnotes
                .definitions
                .entry(footnote_key(&def.label))
                .or_insert(id);
        }
    }
    let width = layout.width();
    let mut lines = layout.blocks(ast.root(), width, TextStyle::default(), false);
    layout.footnote_section(&mut lines, width);
    (lines, layout.diagnostics)
}

/// The footnotes referenced so far, numbered in order of first reference.
#[derive(Default)]
struct Footnotes {
    /// Normalized label → definition (the first one with that label).
    definitions: HashMap<String, NodeId>,
    /// Normalized label → number (1-based).
    numbers: HashMap<String, usize>,
    /// Per number − 1: the definition.
    order: Vec<NodeId>,
}

struct Layout<'a> {
    renderer: &'a AnsiRenderer,
    doc: &'a Document,
    ast: &'a Ast,
    /// Heading → anchor, from the flavor's slugger.
    anchors: HashMap<NodeId, &'a str>,
    footnotes: Footnotes,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Layout<'a> {
    fn new(renderer: &'a AnsiRenderer, doc: &'a Document) -> Self {
        Self {
            renderer,
            doc,
            ast: doc.ast(),
            anchors: doc.anchors().iter().collect(),
            footnotes: Footnotes::default(),
            diagnostics: Vec::new(),
        }
    }

    fn width(&self) -> usize {
        self.renderer.options.width.max(1)
    }

    fn info(&self, id: NodeId) -> BlockInfo {
        BlockInfo {
            node: id,
            span: self.ast.span(id),
            heading: None,
        }
    }

    /// `spans` as lines of block `id`.
    fn lines_of(&self, id: NodeId, lines: Vec<Vec<StyledSpan>>) -> Vec<Line> {
        lines
            .into_iter()
            .map(|spans| Line {
                spans,
                block: Some(self.info(id)),
                graphic: None,
            })
            .collect()
    }

    /// The child blocks of `parent` at `width`, separated by blank lines unless `tight`.
    fn blocks(&mut self, parent: NodeId, width: usize, base: TextStyle, tight: bool) -> Vec<Line> {
        let mut out: Vec<Line> = Vec::new();
        for &child in self.ast.children(parent) {
            let lines = self.block(child, width, base, tight);
            if lines.is_empty() {
                continue;
            }
            if !out.is_empty() && !tight {
                out.push(Line::default());
            }
            out.extend(lines);
        }
        out
    }

    #[allow(clippy::match_same_arms)] // one arm per node kind
    fn block(&mut self, id: NodeId, width: usize, base: TextStyle, tight: bool) -> Vec<Line> {
        let ast = self.ast;
        let styles = &self.renderer.styles;
        match ast.kind(id) {
            NodeKind::Document => self.blocks(id, width, base, false),
            NodeKind::Paragraph => self.paragraph(id, width, base),
            NodeKind::Heading(heading) => self.heading(id, heading, width, base),
            NodeKind::ThematicBreak => {
                let rule = StyledSpan::new("─".repeat(width), styles.rule);
                self.lines_of(id, vec![vec![rule]])
            }
            NodeKind::BlockQuote => {
                let bar = vec![StyledSpan::new("│ ", styles.quote_bar)];
                let content = self.blocks(id, inner(width, 2), base.or(styles.quote), false);
                self.prefixed(id, content, &bar, &bar)
            }
            NodeKind::List(list) => self.list(id, list, width, base),
            NodeKind::CodeBlock(block) => self.code_block(id, block, width),
            NodeKind::HtmlBlock(html) => self.literal(id, html, width, styles.html),
            NodeKind::Table(t) => self.table(id, &t.alignments, width, base),
            NodeKind::DefinitionList => self.definition_list(id, width, base),
            NodeKind::Admonition(admonition) => self.admonition(id, admonition, width, base),
            NodeKind::Callout(callout) => self.callout(id, callout, width, base),
            NodeKind::Math(math) => {
                let indent = vec![StyledSpan::new("  ", TextStyle::default())];
                let lines = self.literal(id, &math.literal, inner(width, 2), base.or(styles.math));
                self.prefixed(id, lines, &indent, &indent)
            }
            NodeKind::ListItem(_) => self.blocks(id, width, base, tight),
            NodeKind::FrontMatter(_)
            | NodeKind::FootnoteDefinition(_)
            | NodeKind::LinkDefinition(_)
            | NodeKind::AbbreviationDefinition(_)
            | NodeKind::IncludeDirective(_)
            | NodeKind::TocDirective(_)
            | NodeKind::DirectiveEnd
            | NodeKind::Comment(_)
            | NodeKind::TableRow(_)
            | NodeKind::TableCell => Vec::new(),
            kind if kind.is_inline() => {
                let mut pieces = Vec::new();
                self.inline(id, base, None, &mut pieces);
                self.lines_of(id, wrap(&pieces, width))
            }
            _ => self.blocks(id, width, base, false),
        }
    }

    /// `lines` behind `first` (the first line) and `rest` (the others); lines without a
    /// block are the container's.
    fn prefixed(
        &self,
        container: NodeId,
        lines: Vec<Line>,
        first: &[StyledSpan],
        rest: &[StyledSpan],
    ) -> Vec<Line> {
        lines
            .into_iter()
            .enumerate()
            .map(|(i, mut line)| {
                let mut spans = if i == 0 {
                    first.to_vec()
                } else {
                    rest.to_vec()
                };
                spans.append(&mut line.spans);
                line.spans = spans;
                line.block.get_or_insert_with(|| self.info(container));
                line
            })
            .collect()
    }

    /// Text lines, each broken at `width`.
    fn literal(&self, id: NodeId, text: &str, width: usize, style: TextStyle) -> Vec<Line> {
        let lines = text
            .lines()
            .flat_map(|line| break_line(&clean(line), width))
            .map(|line| vec![StyledSpan::new(line, style)])
            .collect();
        self.lines_of(id, lines)
    }

    fn paragraph(&mut self, id: NodeId, width: usize, base: TextStyle) -> Vec<Line> {
        if self.renderer.options.graphics
            && let Some(lines) = self.image_paragraph(id, width)
        {
            return lines;
        }
        let mut pieces = Vec::new();
        self.inlines(id, base, None, &mut pieces);
        self.lines_of(id, wrap(&pieces, width))
    }

    /// A paragraph of only images, one line each, when one of them loads.
    fn image_paragraph(&mut self, id: NodeId, width: usize) -> Option<Vec<Line>> {
        let ast = self.ast;
        let mut images = Vec::new();
        for &child in ast.children(id) {
            match ast.kind(child) {
                NodeKind::Image(link) => images.push((link.url.clone(), ast.text(child))),
                NodeKind::Embed(link) if is_image(&link.target) => {
                    images.push((link.target.clone(), embed_alt(link).to_owned()));
                }
                NodeKind::Text(text) if text.trim().is_empty() => {}
                NodeKind::SoftBreak | NodeKind::LineBreak => {}
                _ => return None,
            }
        }
        let loaded: Vec<Option<Image>> =
            images.iter().map(|(url, _)| self.load_image(url)).collect();
        if loaded.iter().all(Option::is_none) {
            return None;
        }
        let style = self.renderer.styles.image;
        let lines = images
            .iter()
            .zip(loaded)
            .flat_map(|((_, alt), image)| {
                let fallback = vec![StyledSpan::new(format!("[image: {}]", clean(alt)), style)];
                match image {
                    Some(image) => self.graphic_lines(id, image, width, fallback),
                    None => vec![Line {
                        spans: fallback,
                        block: Some(self.info(id)),
                        graphic: None,
                    }],
                }
            })
            .collect();
        Some(lines)
    }

    /// The image file `url` next to the document: a PNG, or an SVG rasterized (feature
    /// `png`). Never fetched from the network.
    fn load_image(&self, url: &str) -> Option<Image> {
        let Target::File { path, .. } = Target::parse(url) else {
            return None;
        };
        let path = if path.is_absolute() {
            path
        } else {
            self.doc
                .path()?
                .parent()
                .unwrap_or(Path::new(""))
                .join(path)
        };
        let bytes = std::fs::read(&path).ok()?;
        let svg = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
        if svg {
            return rasterized(&bytes);
        }
        Image::from_png(bytes)
    }

    /// The line of `image` in block `id`, followed by a line for each further row with
    /// [`AnsiOptions::reserve_rows`](super::AnsiOptions::reserve_rows).
    fn graphic_lines(
        &self,
        id: NodeId,
        image: Image,
        width: usize,
        fallback: Vec<StyledSpan>,
    ) -> Vec<Line> {
        let graphic = self.graphic(image, width, fallback);
        let reserved = if self.renderer.options.reserve_rows {
            graphic.rows - 1
        } else {
            0
        };
        let mut lines = vec![Line {
            spans: Vec::new(),
            block: Some(self.info(id)),
            graphic: Some(graphic),
        }];
        lines.extend((0..reserved).map(|_| Line {
            spans: Vec::new(),
            block: Some(self.info(id)),
            graphic: None,
        }));
        lines
    }

    /// `image` sized in cells: at its own pixel size, shrunk to `width` columns.
    fn graphic(&self, image: Image, width: usize, fallback: Vec<StyledSpan>) -> Graphic {
        let (cell_width, cell_height) = self.renderer.options.cell_size;
        let (cell_width, cell_height) =
            (u64::from(cell_width.max(1)), u64::from(cell_height.max(1)));
        let room = width as u64 * cell_width;
        let (mut w, mut h) = (u64::from(image.width), u64::from(image.height));
        if w > room {
            h = (h * room / w).max(1);
            w = room;
        }
        let to_usize = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
        let to_u32 = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
        Graphic {
            columns: to_usize(w.div_ceil(cell_width)).clamp(1, width),
            rows: to_usize(h.div_ceil(cell_height)).max(1),
            pixels: (to_u32(w), to_u32(h)),
            image: Arc::new(image),
            fallback,
        }
    }

    fn heading(
        &mut self,
        id: NodeId,
        heading: &Heading,
        width: usize,
        base: TextStyle,
    ) -> Vec<Line> {
        let styles = &self.renderer.styles;
        let level = heading.level.clamp(1, 6);
        let style = base.or(styles.heading(level));
        let marker = format!("{} ", "#".repeat(usize::from(level)));
        let mut pieces = vec![Piece::text(&marker, styles.heading_marker, None)];
        self.inlines(id, style, None, &mut pieces);
        let mut lines = wrap(&pieces, width);
        let theme_underline = self.renderer.heading_underline[usize::from(level - 1)];
        if theme_underline {
            lines.push(vec![StyledSpan::new(
                "─".repeat(width),
                self.renderer.styles.heading_rule,
            )]);
        }
        let info = BlockInfo {
            heading: Some(HeadingInfo {
                level,
                anchor: self.anchors.get(&id).map(|a| (*a).to_owned()),
            }),
            ..self.info(id)
        };
        lines
            .into_iter()
            .map(|spans| Line {
                spans,
                block: Some(info.clone()),
                graphic: None,
            })
            .collect()
    }

    fn list(&mut self, id: NodeId, list: &List, width: usize, base: TextStyle) -> Vec<Line> {
        let ast = self.ast;
        let styles = &self.renderer.styles;
        let depth = ast
            .ancestors(id)
            .filter(|&a| a != id && matches!(ast.kind(a), NodeKind::List(_)))
            .count();
        let items = ast.children(id);
        let numbers: Vec<String> = match list.kind {
            ListKind::Bullet { .. } => vec![String::new(); items.len()],
            ListKind::Ordered { start, delimiter } => {
                let delimiter = match delimiter {
                    OrderedDelimiter::Period => '.',
                    OrderedDelimiter::Paren => ')',
                };
                (0..items.len() as u64)
                    .map(|i| format!("{}{delimiter}", start.saturating_add(i)))
                    .collect()
            }
        };
        let number_width = numbers.iter().map(String::len).max().unwrap_or(0);
        let mut out = Vec::new();
        for (i, &item) in items.iter().enumerate() {
            let task = match ast.kind(item) {
                NodeKind::ListItem(item) => item.task,
                _ => None,
            };
            let checkbox = task.map(|state| match state {
                TaskState::Open => "☐".to_owned(),
                TaskState::Done => "☑".to_owned(),
                TaskState::Other(c) => format!("[{}]", clean(&c.to_string())),
            });
            let mut marker = Vec::new();
            match list.kind {
                ListKind::Bullet { .. } if checkbox.is_some() => {}
                ListKind::Bullet { .. } => marker.push(StyledSpan::new(
                    format!("{} ", BULLETS[depth % BULLETS.len()]),
                    styles.list_marker,
                )),
                ListKind::Ordered { .. } => marker.push(StyledSpan::new(
                    format!("{:>number_width$} ", numbers[i]),
                    styles.list_marker,
                )),
            }
            if let Some(checkbox) = checkbox {
                marker.push(StyledSpan::new(format!("{checkbox} "), styles.task));
            }
            let marker_width: usize = marker.iter().map(StyledSpan::width).sum();
            let indent = vec![StyledSpan::new(
                " ".repeat(marker_width),
                TextStyle::default(),
            )];
            let mut content = self.blocks(item, inner(width, marker_width), base, list.tight);
            if content.is_empty() {
                content.push(Line {
                    block: Some(self.info(item)),
                    ..Line::default()
                });
            }
            if i > 0 && !list.tight {
                out.push(Line::default());
            }
            out.extend(self.prefixed(item, content, &marker, &indent));
        }
        out
    }

    /// A code block: a diagram, or highlighted code indented by two and painted to the
    /// full width in the block's background.
    fn code_block(&mut self, id: NodeId, block: &CodeBlock, width: usize) -> Vec<Line> {
        if let Some(lines) = self.diagram(id, block, width) {
            return lines;
        }
        let renderer = self.renderer;
        let highlighted = if renderer.options.highlight {
            renderer.highlighter.highlight(&block.literal, &block.info)
        } else {
            renderer.highlighter.highlight(&block.literal, "")
        };
        let plain = renderer.styles.code_block;
        let base = TextStyle {
            fg: highlighted.foreground.or(plain.fg),
            bg: highlighted.background.or(plain.bg),
            ..plain
        };
        let room = inner(width, 2);
        let mut rows: Vec<Vec<StyledSpan>> = Vec::new();
        for line in &highlighted.lines {
            let mut row = Vec::new();
            let (mut column, mut used) = (0, 0);
            for span in line {
                let style = TextStyle {
                    fg: span.style.foreground.or(base.fg),
                    bg: span.style.background.or(base.bg),
                    bold: base.bold || span.style.bold,
                    italic: base.italic || span.style.italic,
                    underline: base.underline || span.style.underline,
                    ..base
                };
                for c in span.text.chars() {
                    let (c, count) = match c {
                        '\t' => (' ', TAB - column % TAB),
                        c if c.is_control() => ('\u{fffd}', 1),
                        c => (c, 1),
                    };
                    for _ in 0..count {
                        let w = char_width(c);
                        if used + w > room && used > 0 {
                            rows.push(std::mem::take(&mut row));
                            used = 0;
                        }
                        push_char(&mut row, c, style, None);
                        used += w;
                        column += w;
                    }
                }
            }
            rows.push(row);
        }
        if rows.is_empty() {
            rows.push(Vec::new());
        }
        let rows = rows
            .into_iter()
            .map(|row| {
                let used: usize = row.iter().map(StyledSpan::width).sum();
                let mut spans = vec![StyledSpan::new("  ", base)];
                for span in row {
                    push_span(&mut spans, span);
                }
                push_span(
                    &mut spans,
                    StyledSpan::new(" ".repeat(room.saturating_sub(used)), base),
                );
                spans
            })
            .collect();
        self.lines_of(id, rows)
    }

    /// A fenced block in a diagram language rendered through the registry: PNG (with
    /// graphics) or text. `None` leaves it to the code block, with a warning when it
    /// failed.
    fn diagram(&mut self, id: NodeId, block: &CodeBlock, width: usize) -> Option<Vec<Line>> {
        let registry = self.renderer.diagrams.as_ref()?;
        block.fence?;
        let lang = block.lang()?;
        let accept: &[DiagramFormat] = if self.renderer.options.graphics {
            &[DiagramFormat::Png, DiagramFormat::Text]
        } else {
            &[DiagramFormat::Text]
        };
        match registry.render(lang, &block.literal, accept)? {
            Ok(rendered) => match rendered.output {
                DiagramOutput::Png(png) => {
                    let image = Image::from_png(png)?;
                    let fallback = vec![StyledSpan::new(
                        format!("[{} diagram]", clean(lang)),
                        self.renderer.styles.image,
                    )];
                    Some(self.graphic_lines(id, image, width, fallback))
                }
                DiagramOutput::Text(text) => {
                    Some(self.literal(id, text.trim_end_matches('\n'), width, TextStyle::default()))
                }
                DiagramOutput::Svg(_) => None,
            },
            Err(failure) => {
                self.diagnostics.push(Diagnostic::new(
                    codes::DIAGRAM,
                    Severity::Warning,
                    self.doc.name(),
                    self.ast.span(id),
                    format!("the `{lang}` diagram is shown as code: {failure}"),
                ));
                None
            }
        }
    }

    fn table(
        &mut self,
        id: NodeId,
        alignments: &[mrq_core::ast::Alignment],
        width: usize,
        base: TextStyle,
    ) -> Vec<Line> {
        let ast = self.ast;
        let row_ids = ast.children(id);
        let mut rows = Vec::new();
        for &row in row_ids {
            let header = matches!(ast.kind(row), NodeKind::TableRow(r) if r.header);
            let cells = ast
                .children(row)
                .iter()
                .map(|&cell| {
                    let mut pieces = Vec::new();
                    self.inlines(cell, base, None, &mut pieces);
                    pieces
                })
                .collect();
            rows.push(Row { header, cells });
        }
        let styles = &self.renderer.styles;
        table(
            &rows,
            alignments,
            width,
            styles.table_border,
            styles.table_header,
        )
        .into_iter()
        .map(|(row, spans)| Line {
            spans,
            block: Some(self.info(row.map_or(id, |i| row_ids[i]))),
            graphic: None,
        })
        .collect()
    }

    fn definition_list(&mut self, id: NodeId, width: usize, base: TextStyle) -> Vec<Line> {
        let ast = self.ast;
        let term = base.or(self.renderer.styles.definition_term);
        let indent = vec![StyledSpan::new("    ", TextStyle::default())];
        let mut out = Vec::new();
        for &item in ast.children(id) {
            if !out.is_empty() {
                out.push(Line::default());
            }
            for &part in ast.children(item) {
                let lines = match ast.kind(part) {
                    NodeKind::DefinitionTerm => self.blocks(part, width, term, true),
                    NodeKind::DefinitionDetails => {
                        let lines = self.blocks(part, inner(width, 4), base, false);
                        self.prefixed(part, lines, &indent, &indent)
                    }
                    _ => self.block(part, width, base, false),
                };
                out.extend(lines);
            }
        }
        out
    }

    fn admonition(
        &mut self,
        id: NodeId,
        admonition: &Admonition,
        width: usize,
        base: TextStyle,
    ) -> Vec<Line> {
        let style = self.title_style(&admonition.kind);
        let title = match admonition.title.as_deref() {
            Some("") if admonition.fold.is_none() => None,
            Some(title) if !title.is_empty() => Some(vec![Piece::text(title, style, None)]),
            _ => Some(vec![Piece::text(
                &default_title(&admonition.kind),
                style,
                None,
            )]),
        };
        self.boxed(id, &admonition.kind, title, admonition.fold, width, base)
    }

    /// A GFM alert or Obsidian callout; an Obsidian title is Markdown.
    fn callout(
        &mut self,
        id: NodeId,
        callout: &Callout,
        width: usize,
        base: TextStyle,
    ) -> Vec<Line> {
        let style = self.title_style(&callout.kind);
        let title = callout
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map_or_else(
                || vec![Piece::text(&default_title(&callout.kind), style, None)],
                |t| self.inline_markdown(t, style),
            );
        self.boxed(id, &callout.kind, Some(title), callout.fold, width, base)
    }

    fn title_style(&self, kind: &str) -> TextStyle {
        TextStyle {
            bold: true,
            ..self.renderer.styles.alert(kind)
        }
    }

    /// `text` parsed as Markdown with the document's flavor, as the pieces of its one
    /// paragraph; plain text when it is not a single paragraph.
    fn inline_markdown(&self, text: &str, style: TextStyle) -> Vec<Piece> {
        let parsed = DocumentOptions::new()
            .flavor(self.doc.flavor().name())
            .parse(text);
        if let Ok(doc) = &parsed {
            let ast = doc.ast();
            if let [only] = ast.children(ast.root())
                && matches!(ast.kind(*only), NodeKind::Paragraph)
            {
                let mut pieces = Vec::new();
                Layout::new(self.renderer, doc).inlines(*only, style, None, &mut pieces);
                return pieces;
            }
        }
        vec![Piece::text(text, style, None)]
    }

    /// An alert, callout or admonition: a bar in the kind's color before the title and
    /// the body.
    fn boxed(
        &mut self,
        id: NodeId,
        kind: &str,
        title: Option<Vec<Piece>>,
        fold: Option<Fold>,
        width: usize,
        base: TextStyle,
    ) -> Vec<Line> {
        let color = self.renderer.styles.alert(kind);
        let bar = vec![StyledSpan::new("┃ ", color)];
        let room = inner(width, 2);
        let mut lines = Vec::new();
        if let Some(title) = title {
            let mark = match fold {
                Some(Fold::Open) => "▾ ",
                Some(Fold::Closed) => "▸ ",
                None => "",
            };
            let mut pieces = vec![Piece::text(mark, self.title_style(kind), None)];
            pieces.extend(title);
            lines = self.lines_of(id, wrap(&pieces, room));
        }
        lines.extend(self.blocks(id, room, base, false));
        self.prefixed(id, lines, &bar, &bar)
    }

    // --- Inlines ------------------------------------------------------------------------

    fn inlines(&mut self, id: NodeId, style: TextStyle, link: Option<&str>, out: &mut Vec<Piece>) {
        for &child in self.ast.children(id) {
            self.inline(child, style, link, out);
        }
    }

    #[allow(clippy::too_many_lines)] // one arm per node kind
    fn inline(&mut self, id: NodeId, style: TextStyle, link: Option<&str>, out: &mut Vec<Piece>) {
        let ast = self.ast;
        let styles = &self.renderer.styles;
        let text = |text: &str, style: TextStyle| Piece::text(text, style, link);
        match ast.kind(id) {
            NodeKind::Text(t) => out.push(text(t, style)),
            NodeKind::SoftBreak => out.push(text(" ", style)),
            NodeKind::LineBreak => out.push(Piece::Break),
            NodeKind::Code(code) => out.push(text(code, style.or(styles.code))),
            NodeKind::Emphasis => self.inlines(id, style.or(styles.emphasis), link, out),
            NodeKind::Strong => self.inlines(id, style.or(styles.strong), link, out),
            NodeKind::Strikethrough => self.inlines(id, style.or(styles.strikethrough), link, out),
            NodeKind::Highlight => self.inlines(id, style.or(styles.highlight), link, out),
            NodeKind::Superscript | NodeKind::Subscript => {
                let mark = if matches!(ast.kind(id), NodeKind::Superscript) {
                    "^("
                } else {
                    "_("
                };
                out.push(text(mark, style));
                self.inlines(id, style, link, out);
                out.push(text(")", style));
            }
            NodeKind::Link(l) => {
                let link_style = style.or(styles.link);
                self.inlines(id, link_style, Some(&l.url), out);
                let shown = ast.text(id);
                let same = shown == l.url || l.url.strip_prefix("mailto:") == Some(shown.as_str());
                if self.renderer.options.link_urls && !same && !l.url.is_empty() {
                    out.push(Piece::text(
                        &format!(" ({})", l.url),
                        style.or(styles.link_url),
                        link,
                    ));
                }
            }
            NodeKind::Image(_) => {
                let alt = ast.text(id);
                out.push(text(&format!("[image: {alt}]"), style.or(styles.image)));
            }
            NodeKind::HtmlInline(html) => out.push(text(html, style.or(styles.html))),
            NodeKind::FootnoteReference(reference) => {
                let label = self.footnote_reference(&reference.label);
                out.push(text(&label, style.or(styles.footnote_ref)));
            }
            NodeKind::WikiLink(w) => {
                let shown = match (&w.alias, w.target.is_empty(), &w.anchor) {
                    (Some(alias), ..) => alias.clone(),
                    (None, true, Some(anchor)) => anchor.clone(),
                    (None, false, Some(anchor)) => format!("{} > {anchor}", w.target),
                    (None, _, None) => w.target.clone(),
                };
                let target = wiki_target(w);
                out.push(Piece::text(&shown, style.or(styles.link), Some(&target)));
            }
            NodeKind::Embed(w) => {
                if is_image(&w.target) {
                    let alt = embed_alt(w);
                    out.push(text(&format!("[image: {alt}]"), style.or(styles.image)));
                } else {
                    let shown = w.alias.as_deref().unwrap_or(&w.target);
                    let target = wiki_target(w);
                    out.push(Piece::text(shown, style.or(styles.link), Some(&target)));
                }
            }
            NodeKind::Tag(tag) => out.push(text(&format!("#{}", tag.name), style.or(styles.tag))),
            NodeKind::Math(math) => out.push(text(&math.literal, style.or(styles.math))),
            NodeKind::Comment(_) => {}
            _ => self.inlines(id, style, link, out),
        }
    }

    /// `[n]` for a defined footnote, numbering it on first reference; the reference as
    /// written otherwise.
    fn footnote_reference(&mut self, label: &str) -> String {
        let key = footnote_key(label);
        let Some(&definition) = self.footnotes.definitions.get(&key) else {
            return format!("[^{label}]");
        };
        let footnotes = &mut self.footnotes;
        let number = *footnotes.numbers.entry(key).or_insert_with(|| {
            footnotes.order.push(definition);
            footnotes.order.len()
        });
        format!("[{number}]")
    }

    /// The referenced footnotes after a short rule, in order of first reference; one
    /// referenced only from another footnote is added while writing.
    fn footnote_section(&mut self, lines: &mut Vec<Line>, width: usize) {
        if self.footnotes.order.is_empty() {
            return;
        }
        let styles = &self.renderer.styles;
        let (rule, footnote, marker_style) = (styles.rule, styles.footnote, styles.footnote_ref);
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::new(vec![StyledSpan::new(
            "─".repeat(width.min(20)),
            rule,
        )]));
        let mut index = 0;
        while index < self.footnotes.order.len() {
            let definition = self.footnotes.order[index];
            let marker = vec![StyledSpan::new(format!("[{}] ", index + 1), marker_style)];
            let marker_width: usize = marker.iter().map(StyledSpan::width).sum();
            let indent = vec![StyledSpan::new(
                " ".repeat(marker_width),
                TextStyle::default(),
            )];
            let mut content = self.blocks(definition, inner(width, marker_width), footnote, false);
            if content.is_empty() {
                content.push(Line::default());
            }
            lines.extend(self.prefixed(definition, content, &marker, &indent));
            index += 1;
        }
    }
}

/// The width left inside a prefix `used` columns wide; at least one.
fn inner(width: usize, used: usize) -> usize {
    width.saturating_sub(used).max(1)
}

/// Whether an embed target is an image file.
fn is_image(target: &str) -> bool {
    Path::new(target)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The alt text of an image embed: the alias unless it is a size (`W` or `WxH`).
fn embed_alt(link: &WikiLink) -> &str {
    let size = |alias: &str| {
        let (width, height) = alias.split_once('x').unwrap_or((alias, ""));
        let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        number(width) && (height.is_empty() || number(height))
    };
    match link.alias.as_deref() {
        Some(alias) if !size(alias) => alias,
        _ => &link.target,
    }
}

/// The link of a wikilink or embed: `[[target#anchor]]`.
fn wiki_target(link: &WikiLink) -> String {
    match &link.anchor {
        Some(anchor) => format!("[[{}#{anchor}]]", link.target),
        None => format!("[[{}]]", link.target),
    }
}

#[cfg(feature = "png")]
fn rasterized(svg: &[u8]) -> Option<Image> {
    let svg = std::str::from_utf8(svg).ok()?;
    Image::from_png(crate::diagram::rasterize(svg, 1.0).ok()?)
}

#[cfg(not(feature = "png"))]
fn rasterized(_svg: &[u8]) -> Option<Image> {
    None
}
