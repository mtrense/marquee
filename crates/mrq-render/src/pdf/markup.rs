//! The mrq AST as Typst markup: [`document`] writes the preamble and the body, and
//! collects the files the body names.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::ast::{
    Alignment, Ast, CodeBlock, ListKind, NodeId, NodeKind, OrderedDelimiter, TaskState, WikiLink,
};
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::DocumentOptions;
use mrq_core::source::ByteRange;
use mrq_core::validate::Target;
use typst::foundations::Bytes;

use super::escape::{push_text, string, text};
use super::preamble::{self, MONO, Page, SERIF, color};
use super::world::{Asset, bundled_font_families};
use super::{PdfRenderer, TypstDocument, codes, math};
use crate::diagram::{self, DiagramFormat, DiagramOutput};
use crate::highlight::Highlighted;
use crate::html::css::ALERT_KINDS;
use crate::html::render::{default_title, footnote_key};
use crate::theme::Theme;

/// Image formats Typst reads, by extension.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];

/// The whole Typst document for `doc`.
pub(super) fn document(renderer: &PdfRenderer, doc: &Document) -> TypstDocument {
    let theme = &renderer.theme;
    let mut diagnostics = Vec::new();
    let available = bundled_font_families();
    let mut fonts = |list: &str, fallback: &str| {
        let resolved = preamble::font_list(list, &available, fallback);
        if let Some(missing) = resolved.missing {
            diagnostics.push(Diagnostic::new(
                codes::FONT,
                Severity::Warning,
                doc.name(),
                ByteRange::new(0, 0),
                format!(
                    "none of the fonts `{missing}` is bundled with mrq; using `{fallback}` \
                     (bundled: {})",
                    available.join(", ")
                ),
            ));
        }
        resolved.families
    };
    let body_fonts = fonts(&theme.fonts.body, SERIF);
    let heading_fonts = match &theme.fonts.heading {
        Some(heading) => fonts(heading, SERIF),
        None => body_fonts.clone(),
    };
    let mono_fonts = fonts(&theme.fonts.mono, MONO);

    let title = crate::html::title(doc);
    let lang = doc.frontmatter().and_then(|fm| fm.get_str(&["lang"]));
    let page = Page {
        paper: &renderer.options.paper,
        title: Some(&title),
        lang,
        body_fonts: &body_fonts,
        heading_fonts: &heading_fonts,
        mono_fonts: &mono_fonts,
        template: renderer.template.as_ref().map(|t| t.vpath.as_str()),
    };
    let mut source = preamble::preamble(theme, &page);

    let mut writer = Writer::new(renderer, doc);
    writer.diagnostics = diagnostics;
    writer.children(writer.ast.root());
    source.push_str(&writer.out);

    let mut inputs = vec![("title", title.clone())];
    if let Some(path) = doc.path() {
        inputs.push(("source", path.display().to_string()));
    }
    TypstDocument {
        source,
        diagnostics: writer.diagnostics,
        file: doc.name().to_path_buf(),
        assets: writer.assets,
        template_dir: renderer.template.as_ref().map(|t| t.dir.clone()),
        inputs,
        today: renderer.options.today,
    }
}

/// The alert color of an alert, callout or admonition kind.
fn alert_color(theme: &Theme, kind: &str) -> String {
    let kind = kind.to_lowercase();
    let alert = ALERT_KINDS
        .iter()
        .find(|(_, kinds)| kinds.contains(&kind.as_str()))
        .map_or("note", |(alert, _)| alert);
    let a = &theme.colors.alerts;
    color(match alert {
        "tip" => a.tip,
        "important" => a.important,
        "warning" => a.warning,
        "caution" => a.caution,
        _ => a.note,
    })
}

/// Footnote definitions by key, and the keys whose footnote was written.
#[derive(Default)]
struct Footnotes {
    definitions: HashMap<String, NodeId>,
    written: HashSet<String>,
}

struct Writer<'a> {
    renderer: &'a PdfRenderer,
    doc: &'a Document,
    ast: &'a Ast,
    out: String,
    /// Heading → anchor, from the flavor's slugger.
    anchors: HashMap<NodeId, &'a str>,
    footnotes: Footnotes,
    assets: HashMap<String, Asset>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Writer<'a> {
    fn new(renderer: &'a PdfRenderer, doc: &'a Document) -> Self {
        let ast = doc.ast();
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
            renderer,
            doc,
            ast,
            out: String::new(),
            anchors: doc.anchors().iter().collect(),
            footnotes,
            assets: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn push(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn text(&mut self, text: &str) {
        push_text(&mut self.out, text);
    }

    fn warn(&mut self, id: NodeId, code: &str, message: String) {
        self.diagnostics.push(Diagnostic::new(
            code,
            Severity::Warning,
            self.doc.name(),
            self.ast.span(id),
            message,
        ));
    }

    /// What `write` writes, captured instead of appended.
    fn capture(&mut self, write: impl FnOnce(&mut Self)) -> String {
        let saved = std::mem::take(&mut self.out);
        write(self);
        std::mem::replace(&mut self.out, saved)
    }

    /// The blocks of `id` as the inside of a content block `[…]`, without the trailing
    /// blank line.
    fn content(&mut self, id: NodeId) -> String {
        let inner = self.capture(|w| w.children(id));
        inner.trim_end().to_owned()
    }

    /// The inlines of `id` as the inside of a content block.
    fn inline_content(&mut self, id: NodeId) -> String {
        self.capture(|w| w.inlines(id))
    }

    fn children(&mut self, id: NodeId) {
        for &child in self.ast.children(id) {
            self.block(child);
        }
    }

    #[allow(clippy::match_same_arms)] // one arm per node kind
    fn block(&mut self, id: NodeId) {
        let ast = self.ast;
        match ast.kind(id) {
            NodeKind::Document => self.children(id),
            NodeKind::Paragraph => {
                self.inlines(id);
                self.push("\n\n");
            }
            NodeKind::Heading(heading) => {
                let content = self.inline_content(id);
                self.push(&format!("#heading(level: {})[{content}]", heading.level));
                if let Some(anchor) = self.anchors.get(&id).copied() {
                    self.push(&format!(" #label({})", string(anchor)));
                }
                self.push("\n\n");
            }
            NodeKind::ThematicBreak => self.push("#mrq-rule()\n\n"),
            NodeKind::BlockQuote => {
                let content = self.content(id);
                self.push(&format!("#mrq-quote[\n{content}\n]\n\n"));
            }
            NodeKind::List(list) => self.list(id, list.kind, list.tight),
            NodeKind::CodeBlock(block) => self.code_block(id, block),
            NodeKind::Table(table) => self.table(id, &table.alignments),
            NodeKind::DefinitionList => self.definition_list(id),
            NodeKind::Admonition(admonition) => {
                let title = match admonition.title.as_deref() {
                    Some("") if admonition.fold.is_none() => None,
                    Some(title) if !title.is_empty() => Some(text(title)),
                    _ => Some(text(&default_title(&admonition.kind))),
                };
                self.alert(id, &admonition.kind, title);
            }
            NodeKind::Callout(callout) => {
                let title = callout
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map_or_else(
                        || text(&default_title(&callout.kind)),
                        |t| self.inline_markdown(t),
                    );
                self.alert(id, &callout.kind, Some(title));
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
                self.inline(id);
                self.push("\n\n");
            }
        }
    }

    fn alert(&mut self, id: NodeId, kind: &str, title: Option<String>) {
        let color = alert_color(&self.renderer.theme, kind);
        let title = title.map_or_else(|| "none".to_owned(), |t| format!("[{t}]"));
        let content = self.content(id);
        self.push(&format!("#mrq-alert({color}, {title})[\n{content}\n]\n\n"));
    }

    /// `text` parsed as Markdown with the document's flavor, as inline markup; the text
    /// as it is when it is not a single paragraph.
    fn inline_markdown(&self, source: &str) -> String {
        let parsed = DocumentOptions::new()
            .flavor(self.doc.flavor().name())
            .parse(source)
            .ok();
        let Some(doc) = parsed else {
            return text(source);
        };
        let ast = doc.ast();
        let [paragraph] = ast.children(ast.root()) else {
            return text(source);
        };
        if !matches!(ast.kind(*paragraph), NodeKind::Paragraph) {
            return text(source);
        }
        let mut writer = Writer::new(self.renderer, &doc);
        writer.inlines(*paragraph);
        writer.out
    }

    fn list(&mut self, id: NodeId, kind: ListKind, tight: bool) {
        let ast = self.ast;
        let items = ast.children(id);
        let task = |item: NodeId| match ast.kind(item) {
            NodeKind::ListItem(item) => item.task,
            _ => None,
        };
        let all_tasks = !items.is_empty() && items.iter().all(|&item| task(item).is_some());
        let mut args = vec![format!("tight: {tight}")];
        let function = match kind {
            ListKind::Bullet { .. } => {
                if all_tasks {
                    args.push("marker: []".to_owned());
                }
                "list"
            }
            ListKind::Ordered { start, delimiter } => {
                if start != 1 {
                    args.push(format!("start: {start}"));
                }
                if delimiter == OrderedDelimiter::Paren {
                    args.push("numbering: \"1)\"".to_owned());
                }
                "enum"
            }
        };
        self.push(&format!("#{function}({},\n", args.join(", ")));
        for &item in items {
            let mut content = String::new();
            match task(item) {
                Some(TaskState::Open) => content.push_str("#mrq-task(false) "),
                Some(TaskState::Done) => content.push_str("#mrq-task(true) "),
                Some(TaskState::Other(c)) => {
                    content.push_str(&text(&format!("[{c}]")));
                    content.push(' ');
                }
                None => {}
            }
            content.push_str(&self.content(item));
            self.push(&format!("  [{content}],\n"));
        }
        self.push(")\n\n");
    }

    fn definition_list(&mut self, id: NodeId) {
        let ast = self.ast;
        self.push("#terms(\n");
        for &item in ast.children(id) {
            let mut term = String::new();
            let mut details = Vec::new();
            for &part in ast.children(item) {
                if matches!(ast.kind(part), NodeKind::DefinitionTerm) {
                    let content = self.content(part);
                    if !term.is_empty() {
                        term.push_str(" #linebreak() ");
                    }
                    term.push_str(&content);
                } else {
                    details.push(self.content(part));
                }
            }
            self.push(&format!(
                "  terms.item([{term}], [{}]),\n",
                details.join("\n\n")
            ));
        }
        self.push(")\n\n");
    }

    fn table(&mut self, id: NodeId, alignments: &[Alignment]) {
        let ast = self.ast;
        let rows = ast.children(id);
        let columns = rows
            .iter()
            .map(|&row| ast.children(row).len())
            .chain(std::iter::once(alignments.len()))
            .max()
            .unwrap_or(0)
            .max(1);
        let align: Vec<&str> = (0..columns)
            .map(|i| match alignments.get(i) {
                Some(Alignment::Center) => "center",
                Some(Alignment::Right) => "right",
                _ => "left",
            })
            .collect();
        self.push(&format!(
            "#table(\n  columns: {columns},\n  align: ({},),\n",
            align.join(", ")
        ));
        for &row in rows {
            let header = matches!(ast.kind(row), NodeKind::TableRow(r) if r.header);
            let mut cells: Vec<String> = ast
                .children(row)
                .iter()
                .map(|&cell| {
                    let content = self.inline_content(cell);
                    if header && !content.is_empty() {
                        format!("[#strong[{content}]]")
                    } else {
                        format!("[{content}]")
                    }
                })
                .collect();
            cells.resize(columns, "[]".to_owned());
            if header {
                self.push(&format!("  table.header({}),\n", cells.join(", ")));
            } else {
                self.push(&format!("  {},\n", cells.join(", ")));
            }
        }
        self.push(")\n\n");
    }

    fn code_block(&mut self, id: NodeId, block: &CodeBlock) {
        if self.diagram(id, block) {
            return;
        }
        let renderer = self.renderer;
        let colors = &renderer.theme.colors;
        let highlighted = if renderer.options.highlight {
            renderer.highlighter.highlight(&block.literal, &block.info)
        } else {
            renderer.highlighter.highlight(&block.literal, "")
        };
        self.code_lines(
            &highlighted,
            highlighted.background.unwrap_or(colors.code_background),
            highlighted.foreground.unwrap_or(colors.code_foreground),
        );
    }

    /// `#mrq-code` with one array of `(text, fill, bold, italic, underline)` spans per
    /// line.
    fn code_lines(
        &mut self,
        code: &Highlighted,
        background: crate::theme::Color,
        foreground: crate::theme::Color,
    ) {
        self.push(&format!(
            "#mrq-code(fill: {}, color: {}, (\n",
            color(background),
            color(foreground)
        ));
        for line in &code.lines {
            let spans: Vec<String> = line
                .iter()
                .map(|span| {
                    let style = span.style;
                    format!(
                        "({}, {}, {}, {}, {})",
                        string(&span.text),
                        style.foreground.map_or_else(|| "none".to_owned(), color),
                        style.bold,
                        style.italic,
                        style.underline
                    )
                })
                .collect();
            match spans.len() {
                0 => self.push("  (),\n"),
                _ => self.push(&format!("  ({},),\n", spans.join(", "))),
            }
        }
        self.push("))\n\n");
    }

    /// A fenced block in a diagram language rendered through the registry, as SVG or
    /// PNG. `false` leaves it to the code block, with a warning when it failed.
    fn diagram(&mut self, id: NodeId, block: &CodeBlock) -> bool {
        let Some(registry) = self.renderer.diagrams.clone() else {
            return false;
        };
        let Some(lang) = block.fence.and(block.lang()) else {
            return false;
        };
        match registry.render(
            lang,
            &block.literal,
            &[DiagramFormat::Svg, DiagramFormat::Png],
        ) {
            None => false,
            Some(Ok(rendered)) => {
                let (extension, bytes) = match rendered.output {
                    DiagramOutput::Svg(svg) => ("svg", svg.into_bytes()),
                    DiagramOutput::Png(png) => ("png", png),
                    DiagramOutput::Text(_) => return false,
                };
                let path = self.asset(extension, Bytes::new(bytes));
                self.push(&format!("#mrq-diagram({})\n\n", string(&path)));
                true
            }
            Some(Err(failure)) => {
                self.warn(
                    id,
                    diagram::codes::DIAGRAM,
                    format!("the `{lang}` diagram is shown as code: {failure}"),
                );
                false
            }
        }
    }

    /// Mounts `bytes` as the next asset; its virtual path.
    fn asset(&mut self, extension: &str, bytes: Bytes) -> String {
        let path = format!("/assets/{}.{extension}", self.assets.len() + 1);
        self.assets.insert(path.clone(), Asset::Bytes(bytes));
        path
    }

    fn inlines(&mut self, id: NodeId) {
        for &child in self.ast.children(id) {
            self.inline(child);
        }
    }

    /// `#function[inlines]`.
    fn wrapped(&mut self, function: &str, id: NodeId) {
        let content = self.inline_content(id);
        self.push(&format!("#{function}[{content}]"));
    }

    fn inline(&mut self, id: NodeId) {
        let ast = self.ast;
        match ast.kind(id) {
            NodeKind::Text(text) => self.text(text),
            NodeKind::SoftBreak => self.push("\n"),
            NodeKind::LineBreak => self.push("#linebreak()\n"),
            NodeKind::Code(code) => self.push(&format!("#raw({})", string(code))),
            NodeKind::Emphasis => self.wrapped("emph", id),
            NodeKind::Strong => self.wrapped("strong", id),
            NodeKind::Strikethrough => self.wrapped("strike", id),
            NodeKind::Highlight => self.wrapped("highlight", id),
            NodeKind::Superscript => self.wrapped("super", id),
            NodeKind::Subscript => self.wrapped("sub", id),
            NodeKind::Link(link) => self.link(id, &link.url),
            NodeKind::Image(link) => {
                let alt = ast.text(id);
                self.image(id, &link.url, &alt);
            }
            NodeKind::FootnoteReference(reference) => self.footnote_reference(&reference.label),
            NodeKind::WikiLink(link) => self.wikilink(link),
            NodeKind::Embed(link) => {
                if is_image(&link.target) {
                    let alt = link.alias.clone().unwrap_or_else(|| link.target.clone());
                    self.image(id, &link.target, &alt);
                } else {
                    self.text(link.alias.as_deref().unwrap_or(&link.target));
                }
            }
            NodeKind::Tag(tag) => {
                let name = text(&format!("#{}", tag.name));
                self.push(&format!("#mrq-tag[{name}]"));
            }
            NodeKind::Math(math) => self.math(id, &math.literal, math.display),
            NodeKind::HtmlInline(_) | NodeKind::Comment(_) => {}
            _ => self.inlines(id),
        }
    }

    fn link(&mut self, id: NodeId, url: &str) {
        let content = self.inline_content(id);
        if url.is_empty() {
            self.push(&content);
            return;
        }
        if let Some(fragment) = url.strip_prefix('#') {
            match self.label_of(fragment) {
                Some(label) => self.push(&format!("#link(label({}))[{content}]", string(&label))),
                None => self.push(&content),
            }
            return;
        }
        self.push(&format!("#link({})[{content}]", string(url)));
    }

    /// The label of the heading `anchor` names in this document.
    fn label_of(&self, anchor: &str) -> Option<String> {
        let heading = self.doc.anchors().get(anchor)?;
        self.anchors.get(&heading).map(|a| (*a).to_owned())
    }

    fn wikilink(&mut self, link: &WikiLink) {
        let shown = match (&link.alias, link.target.is_empty(), &link.anchor) {
            (Some(alias), ..) => alias.clone(),
            (None, true, Some(anchor)) => anchor.clone(),
            (None, false, Some(anchor)) => format!("{} > {anchor}", link.target),
            (None, _, None) => link.target.clone(),
        };
        let local = link
            .target
            .is_empty()
            .then(|| link.anchor.as_deref().and_then(|a| self.label_of(a)))
            .flatten();
        match local {
            Some(label) => self.push(&format!(
                "#link(label({}))[{}]",
                string(&label),
                text(&shown)
            )),
            None => self.text(&shown),
        }
    }

    /// A local image file, mounted; `[alt]` with a warning otherwise.
    fn image(&mut self, id: NodeId, url: &str, alt: &str) {
        match self.load_image(url) {
            Ok((extension, bytes)) => {
                let path = self.asset(&extension, bytes);
                self.push(&format!(
                    "#mrq-image({}, alt: {})",
                    string(&path),
                    string(alt)
                ));
            }
            Err(reason) => {
                self.warn(
                    id,
                    codes::IMAGE,
                    format!("the image `{url}` is shown as its alt text: {reason}"),
                );
                self.text(&format!("[{alt}]"));
            }
        }
    }

    /// The extension and bytes of the local image `url`, relative to the document.
    fn load_image(&self, url: &str) -> Result<(String, Bytes), String> {
        let path = match Target::parse(url) {
            Target::File { path, .. } => path,
            Target::External(_) => return Err("remote images are not fetched".to_owned()),
            _ => return Err("not an image file".to_owned()),
        };
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
            .ok_or_else(|| format!("only {} images are supported", IMAGE_EXTENSIONS.join(", ")))?;
        let base: PathBuf = self
            .doc
            .path()
            .and_then(Path::parent)
            .map_or_else(PathBuf::new, Path::to_path_buf);
        let file = base.join(path);
        let data = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok((extension, Bytes::new(data)))
    }

    /// A footnote the first time its label is referenced, a reference to it later;
    /// `[^label]` for an undefined label.
    fn footnote_reference(&mut self, label: &str) {
        let key = footnote_key(label);
        let Some(&definition) = self.footnotes.definitions.get(&key) else {
            self.text(&format!("[^{label}]"));
            return;
        };
        let name = string(&format!("mrq-footnote:{key}"));
        if self.footnotes.written.insert(key) {
            let content = self.content(definition);
            self.push(&format!("#footnote[{content}]#label({name})"));
        } else {
            self.push(&format!("#footnote(label({name}))"));
        }
    }

    /// Math as Typst math, or its TeX as code with a warning.
    fn math(&mut self, id: NodeId, tex: &str, display: bool) {
        let tex = tex.trim();
        if tex.is_empty() {
            return;
        }
        match math::tex_to_typst(tex) {
            Some(converted) if converted.is_empty() => {}
            Some(converted) if display => self.push(&format!("$ {converted} $")),
            Some(converted) => self.push(&format!("${converted}$")),
            None => {
                self.warn(
                    id,
                    codes::MATH,
                    format!("the math `{tex}` cannot be converted to Typst and is shown as TeX"),
                );
                self.push(&format!("#raw(block: {display}, {})", string(tex)));
            }
        }
    }
}

/// Whether an embed target is an image file.
fn is_image(target: &str) -> bool {
    Path::new(target)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}
