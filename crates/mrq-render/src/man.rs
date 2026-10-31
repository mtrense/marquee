//! The man backend: documents as roff for the `man(7)` macros, built with the `roff`
//! crate (`mrq render -o man mrq.1.md > mrq.1`).
//!
//! # The page title
//!
//! `.TH NAME SECTION [DATE [SOURCE [MANUAL]]]` and the `NAME` section come from, in
//! order of precedence:
//!
//! 1. the frontmatter: `title` (the name), `section`, `description`, `date`, `source`
//!    and `manual`;
//! 2. an H1 that starts the document, in ronn's form `name(section) -- description`
//!    (also with `-`, `–` or `—`; section and description are optional). It is not
//!    rendered otherwise;
//! 3. the file name, `name.section.md` (`mrq.1.md`) or just `name.md`;
//! 4. [`ManOptions`] for date, source and manual, and section `1`.
//!
//! The name is uppercased in `.TH`. `.SH NAME` with `name \- description` is generated
//! unless the document has a `NAME` section of its own. A page without any name is
//! `UNTITLED`, with a [`codes::MAN_NAME`] warning.
//!
//! # Blocks
//!
//! - **Headings:** a later H1 and H2 are `.SH` (uppercased), H3 `.SS`, H4–H6 a bold
//!   paragraph. In the `SYNOPSIS` section, line breaks in paragraphs are kept (`.br`).
//! - **Paragraphs** are `.PP` (none right after a heading). **Code blocks** are
//!   `.nf`/`.fi`, indented with `.RS 4`/`.RE`, unhighlighted.
//! - **Lists:** items are `.IP` with `\(bu`, the number (`1.`, `1)`) or the task box
//!   (`[ ]`, `[x]`) as tag; an item's blocks after its first paragraph (nested lists
//!   too) are indented by `.RS`/`.RE`. **Definition lists:** `.TP` with the term, the
//!   details after it.
//! - **Block quotes** are indented (`.RS 4`); **alerts, callouts and admonitions** too,
//!   with a bold title paragraph (the kind capitalized by default; an Obsidian title is
//!   Markdown; an mkdocs `""` title has none).
//! - **Tables** go through `tbl`: `.TS`/`.TE` with the column alignments, a bold
//!   header row with a rule under it and every cell a text block (`T{`/`T}`), so long
//!   cells wrap; `.sp` before it. A page with a table starts with `'\" t` so that `man` runs `tbl`.
//! - **Footnotes** are numbered `[n]` by first reference and listed in a closing
//!   `.SH NOTES`; unreferenced definitions are left out. Thematic breaks are `* * *`.
//! - Raw HTML, comments, frontmatter, link and abbreviation definitions and `mrq:`
//!   directives render nothing.
//!
//! # Inlines
//!
//! Emphasis is italic (`\fI`), strong and code bold (`\fB`; bold wins when nested, roff
//! has no portable bold italic). Links show their text, followed by ` <url>` when the
//! URL has a scheme and differs from the text (relative and `#anchor` links lead
//! nowhere in a man page). Images show their alt text, wikilinks their alias,
//! `target > anchor` or target, embeds their alias or target, tags `#tag`, math its
//! TeX. Strikethrough, highlight, superscript and subscript show their text.
//!
//! # Escaping
//!
//! Text is escaped by the `roff` crate: `\` and `-` (as `\-`), `\&` before a `.` or
//! `'` that would start a line, and apostrophes as `\*(Aq` (defined in a preamble the
//! crate writes first). Macro arguments escape `\`, `-` and `"` (`\(dq`) here, except
//! that the `.TH` date keeps its hyphens.
//! Non-ASCII text is written as UTF-8, which mandoc and man-db (through `preconv`)
//! read.

use std::collections::HashMap;
use std::path::Path;

use mrq_core::Document;
use mrq_core::ast::{
    Alignment, Ast, ListKind, NodeId, NodeKind, OrderedDelimiter, TaskState, WikiLink,
};
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::DocumentOptions;
use mrq_core::source::ByteRange;
use roff::{Inline, Roff};
use serde_json::Value;

use crate::html::render::{default_title, footnote_key};

/// The codes of the man backend's diagnostics.
pub mod codes {
    /// The page has no name: no frontmatter `title`, no leading H1 and no file name.
    pub const MAN_NAME: &str = "man-name";
}

/// What the document does not say about its `.TH` line. Frontmatter wins over these.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManOptions {
    /// The date in the centre of the footer, e.g. `2026-10-31` or `October 2026`.
    pub date: Option<String>,
    /// Where the page comes from (left of the footer), e.g. `mrq 0.1.0`.
    pub source: Option<String>,
    /// The manual's title (centre of the header), e.g. `mrq Manual`.
    pub manual: Option<String>,
}

/// What [`ManRenderer::render_with_diagnostics`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManOutput {
    /// The roff source of the page.
    pub roff: String,
    /// Warnings ([`codes::MAN_NAME`]).
    pub diagnostics: Vec<Diagnostic>,
}

/// Renders documents to man pages. See the [module docs](self).
#[derive(Debug, Clone, Default)]
pub struct ManRenderer {
    options: ManOptions,
}

impl ManRenderer {
    /// A renderer with `options` as the fallback `.TH` fields.
    pub const fn new(options: ManOptions) -> Self {
        Self { options }
    }

    /// The options.
    pub const fn options(&self) -> &ManOptions {
        &self.options
    }

    /// `doc` as roff. Warnings are dropped;
    /// [`render_with_diagnostics`](Self::render_with_diagnostics) keeps them.
    pub fn render(&self, doc: &Document) -> String {
        self.render_with_diagnostics(doc).roff
    }

    /// `doc` as roff, like [`render`](Self::render), and the warnings.
    pub fn render_with_diagnostics(&self, doc: &Document) -> ManOutput {
        let mut writer = Writer::new(doc);
        let mut diagnostics = Vec::new();
        let title = Title::of(doc, writer.title_heading);
        writer.head(&title, &self.options);
        if title.name.is_none() {
            diagnostics.push(Diagnostic::new(
                codes::MAN_NAME,
                Severity::Warning,
                doc.name(),
                ByteRange::new(0, 0),
                "the man page has no name: start the document with an H1 \
                 `name(section) -- description` or set `title` in the frontmatter",
            ));
        }
        writer.node(writer.ast.root());
        writer.footnote_section();
        let mut roff = writer.roff.render();
        if writer.has_table {
            // The preprocessor line `man` reads to run `tbl`; it must come first.
            roff.insert_str(0, "'\\\" t\n");
        }
        ManOutput { roff, diagnostics }
    }
}

/// The parts of the page title the document gives.
#[derive(Debug, Default)]
struct Title {
    name: Option<String>,
    section: Option<String>,
    description: Option<String>,
    date: Option<String>,
    source: Option<String>,
    manual: Option<String>,
}

impl Title {
    /// The title from the frontmatter, then the leading H1, then the file name.
    fn of(doc: &Document, heading: Option<NodeId>) -> Self {
        let mut title = Self::default();
        if let Some(fm) = doc.frontmatter() {
            let field = |key: &str| match fm.get(&[key])? {
                Value::String(s) => Some(s.trim().to_owned()).filter(|s| !s.is_empty()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            };
            title.name = field("title");
            title.section = field("section");
            title.description = field("description");
            title.date = field("date");
            title.source = field("source");
            title.manual = field("manual");
        }
        if let Some(heading) = heading {
            let text = doc.ast().text(heading);
            let (lhs, description) = split_description(&text);
            let (name, section) = split_section(lhs);
            title.name = title.name.or(name);
            title.section = title.section.or(section);
            title.description = title.description.or(description);
        }
        if title.name.is_none()
            && let Some(stem) = doc.path().and_then(Path::file_stem)
        {
            let stem = stem.to_string_lossy();
            let (name, section) = match stem.rsplit_once('.') {
                Some((name, section))
                    if !name.is_empty() && section.starts_with(|c: char| c.is_ascii_digit()) =>
                {
                    (name, Some(section))
                }
                _ => (&*stem, None),
            };
            title.name = Some(name.to_owned());
            title.section = title.section.or_else(|| section.map(str::to_owned));
        }
        title
    }
}

/// `name -- description` split at the first ` -- `, ` - `, ` – ` or ` — `.
fn split_description(text: &str) -> (&str, Option<String>) {
    let separator = [" -- ", " - ", " – ", " — "]
        .iter()
        .filter_map(|sep| text.find(sep).map(|at| (at, sep.len())))
        .min();
    match separator {
        Some((at, len)) => {
            let description = text[at + len..].trim();
            (
                text[..at].trim(),
                (!description.is_empty()).then(|| description.to_owned()),
            )
        }
        None => (text.trim(), None),
    }
}

/// `name(section)` split into its parts; the section is optional.
fn split_section(text: &str) -> (Option<String>, Option<String>) {
    let some = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
    if let Some(inner) = text.strip_suffix(')')
        && let Some((name, section)) = inner.rsplit_once('(')
        && !name.trim().is_empty()
    {
        return (some(name), some(section));
    }
    (some(text), None)
}

/// A macro argument: `\`, `-` and `"` escaped, and `""` when empty (the `roff` crate
/// quotes arguments with spaces).
fn arg(text: &str) -> String {
    if text.is_empty() {
        return "\"\"".to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '-' => out.push_str("\\-"),
            '"' => out.push_str("\\(dq"),
            '\n' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Whether `url` starts with a scheme (`https:`, `mailto:`, …).
fn has_scheme(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Kinds before a leading H1 that do not keep it from being the title.
const fn is_invisible(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::FrontMatter(_)
            | NodeKind::LinkDefinition(_)
            | NodeKind::AbbreviationDefinition(_)
            | NodeKind::HtmlBlock(_)
            | NodeKind::Comment(_)
            | NodeKind::IncludeDirective(_)
            | NodeKind::TocDirective(_)
            | NodeKind::DirectiveEnd
    )
}

/// Inline text being collected, in lines split at hard breaks, with the current font.
#[derive(Default)]
struct Inlines {
    lines: Vec<Vec<Inline>>,
    bold: usize,
    italic: usize,
}

impl Inlines {
    /// Adds `text` in the current font, merged with the previous part when the font is
    /// the same (so that the `roff` crate sees a `.` after a newline and escapes it).
    fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        let line = self.lines.last_mut().expect("a line");
        let merged = match (line.last_mut(), self.bold > 0, self.italic > 0) {
            (Some(Inline::Bold(prev)), true, _)
            | (Some(Inline::Italic(prev)), false, true)
            | (Some(Inline::Roman(prev)), false, false) => {
                prev.push_str(text);
                true
            }
            _ => false,
        };
        if !merged {
            line.push(if self.bold > 0 {
                Inline::Bold(text.to_owned())
            } else if self.italic > 0 {
                Inline::Italic(text.to_owned())
            } else {
                Inline::Roman(text.to_owned())
            });
        }
    }

    /// Ends the line: what follows starts a new text line after `.br`.
    fn hard_break(&mut self) {
        self.lines.push(Vec::new());
    }

    /// The non-empty lines, with whitespace at their ends trimmed (a text line starting
    /// with a space would break the filled line in roff).
    fn into_lines(self) -> Vec<Vec<Inline>> {
        let mut lines: Vec<Vec<Inline>> = Vec::new();
        for mut line in self.lines {
            if let Some(Inline::Roman(text) | Inline::Bold(text) | Inline::Italic(text)) =
                line.first_mut()
            {
                *text = text.trim_start().to_owned();
            }
            if let Some(Inline::Roman(text) | Inline::Bold(text) | Inline::Italic(text)) =
                line.last_mut()
            {
                *text = text.trim_end().to_owned();
            }
            line.retain(|inline| !matches!(inline, Inline::Roman(t) | Inline::Bold(t) | Inline::Italic(t) if t.is_empty()));
            if !line.is_empty() {
                lines.push(line);
            }
        }
        lines
    }
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

struct Writer<'a> {
    doc: &'a Document,
    ast: &'a Ast,
    roff: Roff,
    /// The leading H1 that gives the page title.
    title_heading: Option<NodeId>,
    /// The last line was `.SH` or `.SS`, so a paragraph needs no `.PP`.
    after_heading: bool,
    /// Inside the `SYNOPSIS` section.
    synopsis: bool,
    has_table: bool,
    footnotes: Footnotes,
}

impl<'a> Writer<'a> {
    fn new(doc: &'a Document) -> Self {
        let ast = doc.ast();
        let title_heading = ast
            .children(ast.root())
            .iter()
            .copied()
            .find(|&id| !is_invisible(ast.kind(id)))
            .filter(|&id| matches!(ast.kind(id), NodeKind::Heading(h) if h.level == 1));
        let mut footnotes = Footnotes::default();
        for id in ast.descendants(ast.root()) {
            if let NodeKind::FootnoteDefinition(def) = ast.kind(id) {
                footnotes
                    .definitions
                    .entry(footnote_key(&def.label))
                    .or_insert(id);
            }
        }
        Self {
            doc,
            ast,
            roff: Roff::new(),
            title_heading,
            after_heading: false,
            synopsis: false,
            has_table: false,
            footnotes,
        }
    }

    fn control(&mut self, name: &str, args: &[&str]) {
        self.after_heading = matches!(name, "SH" | "SS");
        self.roff.control(name, args.iter().copied());
    }

    fn text(&mut self, line: Vec<Inline>) {
        self.after_heading = false;
        self.roff.text(line);
    }

    fn text_lines(&mut self, inlines: Inlines) {
        for (i, line) in inlines.into_lines().into_iter().enumerate() {
            if i > 0 {
                self.control("br", &[]);
            }
            self.text(line);
        }
    }

    /// Starts a paragraph, unless right after a heading.
    fn pp(&mut self) {
        if !self.after_heading {
            self.control("PP", &[]);
        }
        self.after_heading = false;
    }

    /// `.TH` and the generated `NAME` section.
    fn head(&mut self, title: &Title, options: &ManOptions) {
        let name = title.name.as_deref().unwrap_or("untitled");
        let fields = [
            Some(name.to_uppercase()),
            Some(title.section.clone().unwrap_or_else(|| "1".to_owned())),
            title.date.clone().or_else(|| options.date.clone()),
            title.source.clone().or_else(|| options.source.clone()),
            title.manual.clone().or_else(|| options.manual.clone()),
        ];
        let last = fields.iter().rposition(Option::is_some).unwrap_or(0);
        let args: Vec<String> = fields[..=last]
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let field = field.as_deref().unwrap_or("");
                // The date keeps its hyphens, so that mandoc can parse `2026-10-31`.
                if i == 2 && !field.is_empty() {
                    field.replace('\\', "\\\\").replace('"', "\\(dq")
                } else {
                    arg(field)
                }
            })
            .collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.control("TH", &args);

        let ast = self.ast;
        let has_name_section = ast.descendants(ast.root()).any(|id| {
            matches!(ast.kind(id), NodeKind::Heading(h) if h.level <= 2)
                && Some(id) != self.title_heading
                && ast.text(id).trim().eq_ignore_ascii_case("name")
        });
        if title.name.is_none() || has_name_section {
            return;
        }
        self.control("SH", &["NAME"]);
        let line = match &title.description {
            Some(description) => format!("{name} - {description}"),
            None => name.to_owned(),
        };
        self.text(vec![Inline::Roman(line)]);
    }

    fn children(&mut self, id: NodeId) {
        for &child in self.ast.children(id) {
            self.node(child);
        }
    }

    #[allow(clippy::match_same_arms)] // one arm per node kind
    fn node(&mut self, id: NodeId) {
        let ast = self.ast;
        match ast.kind(id) {
            NodeKind::Document | NodeKind::DefinitionList => self.children(id),
            NodeKind::Paragraph => {
                self.pp();
                let mut inlines = Inlines::default();
                self.inlines(id, &mut inlines);
                self.text_lines(inlines);
            }
            NodeKind::Heading(heading) => {
                if Some(id) == self.title_heading {
                    return;
                }
                let text = ast.text(id).trim().to_owned();
                match heading.level {
                    1 | 2 => {
                        let upper = text.to_uppercase();
                        self.synopsis = upper == "SYNOPSIS";
                        self.control("SH", &[&arg(&upper)]);
                    }
                    3 => self.control("SS", &[&arg(&text)]),
                    _ => {
                        self.pp();
                        let mut inlines = Inlines {
                            bold: 1,
                            ..Inlines::default()
                        };
                        self.inlines(id, &mut inlines);
                        self.text_lines(inlines);
                    }
                }
            }
            NodeKind::ThematicBreak => {
                self.pp();
                self.text(vec![Inline::Roman("* * *".to_owned())]);
            }
            NodeKind::BlockQuote => {
                self.control("RS", &["4"]);
                self.children(id);
                self.control("RE", &[]);
            }
            NodeKind::List(list) => self.list(id, list.kind),
            NodeKind::CodeBlock(block) => self.code_block(&block.literal),
            NodeKind::Table(table) => self.table(id, &table.alignments),
            NodeKind::DefinitionItem => self.definition_item(id),
            NodeKind::Admonition(admonition) => {
                let title = match admonition.title.as_deref() {
                    Some("") if admonition.fold.is_none() => None,
                    Some(title) if !title.is_empty() => Some(Inlines::bold_text(title)),
                    _ => Some(Inlines::bold_text(&default_title(&admonition.kind))),
                };
                self.boxed(id, title);
            }
            NodeKind::Callout(callout) => {
                let title = callout
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map_or_else(
                        || Inlines::bold_text(&default_title(&callout.kind)),
                        |t| self.inline_markdown(t),
                    );
                self.boxed(id, Some(title));
            }
            NodeKind::FrontMatter(_)
            | NodeKind::FootnoteDefinition(_)
            | NodeKind::LinkDefinition(_)
            | NodeKind::AbbreviationDefinition(_)
            | NodeKind::IncludeDirective(_)
            | NodeKind::TocDirective(_)
            | NodeKind::DirectiveEnd
            | NodeKind::HtmlBlock(_)
            | NodeKind::Comment(_)
            | NodeKind::HtmlInline(_) => {}
            kind if kind.is_block() => self.children(id),
            // An inline standing as a block of its own (display math).
            _ => {
                self.pp();
                let mut inlines = Inlines::default();
                self.inline(id, &mut inlines);
                self.text_lines(inlines);
            }
        }
    }

    /// Literal lines between `.nf` and `.fi`, indented.
    fn code_block(&mut self, literal: &str) {
        self.pp();
        self.control("RS", &["4"]);
        self.control("nf", &[]);
        for line in literal.trim_end_matches('\n').split('\n') {
            self.text(vec![Inline::Roman(line.to_owned())]);
        }
        self.control("fi", &[]);
        self.control("RE", &[]);
    }

    /// `.TP`, the term (its blocks' inlines), then the blocks of all its details.
    fn definition_item(&mut self, id: NodeId) {
        let ast = self.ast;
        self.control("TP", &[]);
        let mut blocks = Vec::new();
        for &part in ast.children(id) {
            if matches!(ast.kind(part), NodeKind::DefinitionTerm) {
                let mut inlines = Inlines::default();
                self.inlines(part, &mut inlines);
                self.text_lines(inlines);
            } else {
                blocks.extend_from_slice(ast.children(part));
            }
        }
        self.tagged_body(&blocks, None);
    }

    /// A list: one `.IP` per item.
    fn list(&mut self, id: NodeId, kind: ListKind) {
        let ast = self.ast;
        let items = ast.children(id);
        let task = items
            .iter()
            .any(|&item| matches!(ast.kind(item), NodeKind::ListItem(i) if i.task.is_some()));
        let width = match kind {
            _ if task => 4,
            ListKind::Bullet { .. } => 2,
            ListKind::Ordered { start, .. } => {
                let last = start + items.len().saturating_sub(1) as u64;
                last.to_string().len() + 2
            }
        };
        let width = width.to_string();
        for (i, &item) in items.iter().enumerate() {
            let state = match ast.kind(item) {
                NodeKind::ListItem(item) => item.task,
                _ => None,
            };
            let tag = match (state, kind) {
                (Some(TaskState::Open), _) => "[ ]".to_owned(),
                (Some(TaskState::Done), _) => "[x]".to_owned(),
                (Some(TaskState::Other(c)), _) => arg(&format!("[{c}]")),
                (None, ListKind::Bullet { .. }) => "\\(bu".to_owned(),
                (None, ListKind::Ordered { start, delimiter }) => {
                    let mark = match delimiter {
                        OrderedDelimiter::Period => '.',
                        OrderedDelimiter::Paren => ')',
                    };
                    format!("{}{mark}", start + i as u64)
                }
            };
            self.control("IP", &[&tag, &width]);
            self.tagged_body(ast.children(item), Some(&width));
        }
    }

    /// The body of an `.IP` or `.TP`: the first paragraph on the tag's line, the other
    /// blocks indented by `indent` (the prevailing indent when `None`).
    fn tagged_body(&mut self, blocks: &[NodeId], indent: Option<&str>) {
        let ast = self.ast;
        let rest = match blocks.split_first() {
            Some((&first, rest)) if matches!(ast.kind(first), NodeKind::Paragraph) => {
                let mut inlines = Inlines::default();
                self.inlines(first, &mut inlines);
                self.text_lines(inlines);
                rest
            }
            _ => blocks,
        };
        if rest.is_empty() {
            return;
        }
        match indent {
            Some(indent) => self.control("RS", &[indent]),
            None => self.control("RS", &[]),
        }
        for &block in rest {
            self.node(block);
        }
        self.control("RE", &[]);
    }

    /// An alert, callout or admonition: indented, with a bold title paragraph.
    fn boxed(&mut self, id: NodeId, title: Option<Inlines>) {
        self.control("RS", &["4"]);
        if let Some(title) = title {
            self.control("PP", &[]);
            self.text_lines(title);
        }
        self.children(id);
        self.control("RE", &[]);
    }

    /// `text` parsed as Markdown with the document's flavor, bold; the text as it is
    /// when it is not a single paragraph.
    fn inline_markdown(&self, text: &str) -> Inlines {
        let parsed = DocumentOptions::new()
            .flavor(self.doc.flavor().name())
            .parse(text)
            .ok();
        let Some(doc) = parsed else {
            return Inlines::bold_text(text);
        };
        let ast = doc.ast();
        let [paragraph] = ast.children(ast.root()) else {
            return Inlines::bold_text(text);
        };
        if !matches!(ast.kind(*paragraph), NodeKind::Paragraph) {
            return Inlines::bold_text(text);
        }
        let mut inlines = Inlines {
            bold: 1,
            ..Inlines::default()
        };
        Writer::new(&doc).inlines(*paragraph, &mut inlines);
        inlines
    }

    fn table(&mut self, id: NodeId, alignments: &[Alignment]) {
        let ast = self.ast;
        self.has_table = true;
        // Not `.PP`: mandoc drops an empty paragraph before a table, with its space.
        if !self.after_heading {
            self.control("sp", &[]);
        }
        self.control("TS", &[]);
        let columns = ast
            .children(id)
            .iter()
            .map(|&row| ast.children(row).len())
            .max()
            .unwrap_or(0)
            .max(1);
        let format = |bold: bool| -> String {
            (0..columns)
                .map(|i| {
                    let align = match alignments.get(i) {
                        Some(Alignment::Center) => "c",
                        Some(Alignment::Right) => "r",
                        _ => "l",
                    };
                    if bold {
                        format!("{align}b")
                    } else {
                        align.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let rows = ast.children(id);
        let header = rows
            .first()
            .is_some_and(|&row| matches!(ast.kind(row), NodeKind::TableRow(r) if r.header));
        let body = rows.len() > usize::from(header);
        if header {
            let line = if body {
                format(true)
            } else {
                format(true) + "."
            };
            self.text(vec![Inline::Roman(line)]);
        }
        if body {
            self.text(vec![Inline::Roman(format(false) + ".")]);
        }
        for (i, &row) in rows.iter().enumerate() {
            self.table_row(row);
            if i == 0 && header && body {
                self.text(vec![Inline::Roman("_".to_owned())]);
            }
        }
        self.control("TE", &[]);
    }

    /// A row of tab-separated text blocks (`T{`, the cell's text, `T}`); empty cells
    /// are empty.
    fn table_row(&mut self, row: NodeId) {
        let mut line = String::new();
        for (i, &cell) in self.ast.children(row).iter().enumerate() {
            if i > 0 {
                line.push('\t');
            }
            let mut inlines = Inlines::default();
            self.inlines(cell, &mut inlines);
            let lines = inlines.into_lines();
            if lines.is_empty() {
                continue;
            }
            line.push_str("T{");
            self.text(vec![Inline::Roman(std::mem::take(&mut line))]);
            for (j, text) in lines.into_iter().enumerate() {
                if j > 0 {
                    self.control("br", &[]);
                }
                self.text(text);
            }
            line.push_str("T}");
        }
        if !line.is_empty() {
            self.text(vec![Inline::Roman(line)]);
        }
    }

    fn inlines(&mut self, id: NodeId, out: &mut Inlines) {
        for &child in self.ast.children(id) {
            self.inline(child, out);
        }
    }

    fn inline(&mut self, id: NodeId, out: &mut Inlines) {
        let ast = self.ast;
        match ast.kind(id) {
            NodeKind::Text(text) => out.push(text),
            NodeKind::SoftBreak if self.synopsis => out.hard_break(),
            NodeKind::SoftBreak => out.push("\n"),
            NodeKind::LineBreak => out.hard_break(),
            NodeKind::Code(code) => {
                out.bold += 1;
                out.push(code);
                out.bold -= 1;
            }
            NodeKind::Emphasis => {
                out.italic += 1;
                self.inlines(id, out);
                out.italic -= 1;
            }
            NodeKind::Strong => {
                out.bold += 1;
                self.inlines(id, out);
                out.bold -= 1;
            }
            NodeKind::Link(link) => {
                self.inlines(id, out);
                let text = ast.text(id);
                let url = link.url.as_str();
                if has_scheme(url) && text != url && url.strip_prefix("mailto:") != Some(&text) {
                    out.push(&format!(" <{url}>"));
                }
            }
            NodeKind::Image(_) => out.push(&ast.text(id)),
            NodeKind::FootnoteReference(reference) => {
                let mark = self.footnote_reference(&reference.label);
                out.push(&mark);
            }
            NodeKind::WikiLink(link) => out.push(&wikilink_text(link)),
            NodeKind::Embed(link) => out.push(link.alias.as_deref().unwrap_or(&link.target)),
            NodeKind::Tag(tag) => out.push(&format!("#{}", tag.name)),
            NodeKind::Math(math) => out.push(&math.literal),
            NodeKind::HtmlInline(_) | NodeKind::Comment(_) => {}
            _ => self.inlines(id, out),
        }
    }

    /// `[n]` for a defined footnote, numbered by first reference; `[^label]` otherwise.
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

    /// `.SH NOTES` with the referenced footnotes. A definition referenced only from
    /// another footnote is added while writing.
    fn footnote_section(&mut self) {
        if self.footnotes.order.is_empty() {
            return;
        }
        self.synopsis = false;
        self.control("SH", &["NOTES"]);
        let mut index = 0;
        while index < self.footnotes.order.len() {
            let definition = self.footnotes.order[index];
            let mark = format!("[{}]", index + 1);
            let width = (mark.len() + 1).to_string();
            self.control("IP", &[&mark, &width]);
            self.tagged_body(self.ast.children(definition), Some(&width));
            index += 1;
        }
    }
}

impl Inlines {
    /// Plain `text` in bold.
    fn bold_text(text: &str) -> Self {
        let mut inlines = Self {
            bold: 1,
            ..Self::default()
        };
        inlines.push(text);
        inlines
    }
}

/// What a wikilink shows: its alias, `target > anchor`, the anchor or the target.
fn wikilink_text(link: &WikiLink) -> String {
    match (&link.alias, link.target.is_empty(), &link.anchor) {
        (Some(alias), ..) => alias.clone(),
        (None, true, Some(anchor)) => anchor.clone(),
        (None, false, Some(anchor)) => format!("{} > {anchor}", link.target),
        (None, _, None) => link.target.clone(),
    }
}
