//! The tree walk: one [`Writer`] per rendered document, writing the body HTML node by
//! node, then the footnotes section.

use std::collections::HashMap;
use std::path::Path;

use mrq_core::Document;
use mrq_core::ast::{
    Admonition, Alignment, Ast, Callout, CodeBlock, CommentSyntax, Fold, Heading, ListKind, NodeId,
    NodeKind, TaskState, WikiLink,
};
use mrq_core::diag::Diagnostic;
use mrq_core::document::DocumentOptions;
use mrq_core::flavor::obsidian_anchor;
use pulldown_latex::config::{DisplayMode, MathStyle};
use pulldown_latex::{Parser, RenderConfig, Storage, push_mathml};

use super::escape::{escaped, push_escaped, push_href, push_raw_html};
use super::{CodeBlockContext, HtmlRenderer};
use crate::highlight::CLASS_PREFIX;

/// File extensions an Obsidian embed shows as an image.
pub(crate) const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "avif", "ico",
];

/// The body HTML of `doc`, and the warnings of the code block hooks.
pub(super) fn body(renderer: &HtmlRenderer, doc: &Document) -> (String, Vec<Diagnostic>) {
    let ast = doc.ast();
    let mut writer = Writer {
        renderer,
        doc,
        ast,
        out: String::new(),
        anchors: doc.anchors().iter().collect(),
        footnotes: Footnotes::default(),
        abbreviations: Vec::new(),
        checkbox: None,
        diagnostics: Vec::new(),
    };
    for id in ast.descendants(ast.root()) {
        match ast.kind(id) {
            NodeKind::FootnoteDefinition(def) => {
                writer
                    .footnotes
                    .definitions
                    .entry(footnote_key(&def.label))
                    .or_insert(id);
            }
            NodeKind::AbbreviationDefinition(abbr) if !abbr.abbr.is_empty() => {
                writer
                    .abbreviations
                    .push((abbr.abbr.as_str(), abbr.expansion.as_str()));
            }
            _ => {}
        }
    }
    // Longest first, so `HTML5` wins over `HTML` at the same position.
    writer
        .abbreviations
        .sort_by_key(|(abbr, _)| std::cmp::Reverse(abbr.len()));
    writer.node(ast.root(), false);
    writer.footnote_section();
    (writer.out, writer.diagnostics)
}

/// The footnotes referenced so far, numbered in order of first reference.
#[derive(Default)]
struct Footnotes {
    /// Normalized label → definition (the first one with that label).
    definitions: HashMap<String, NodeId>,
    /// Normalized label → number (1-based).
    numbers: HashMap<String, usize>,
    /// Per number − 1: the definition and how often it was referenced.
    order: Vec<(NodeId, usize)>,
}

/// Footnote labels match case-insensitively with whitespace runs collapsed, as link
/// labels do.
pub(crate) fn footnote_key(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

struct Writer<'a> {
    renderer: &'a HtmlRenderer,
    doc: &'a Document,
    ast: &'a Ast,
    out: String,
    /// Heading → anchor, from the flavor's slugger.
    anchors: HashMap<NodeId, &'a str>,
    footnotes: Footnotes,
    /// (abbreviation, expansion), longest first.
    abbreviations: Vec<(&'a str, &'a str)>,
    /// The checkbox of a task item, written by the item's first paragraph.
    checkbox: Option<TaskState>,
    /// Warnings of the code block hooks.
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Writer<'a> {
    /// Starts a new line unless at the start of one.
    fn cr(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn push(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn children(&mut self, id: NodeId, tight: bool) {
        for &child in self.ast.children(id) {
            self.node(child, tight);
        }
    }

    /// Writes `id`; `tight` says that it is a block directly inside an item of a tight
    /// list, where paragraphs have no `<p>`.
    #[allow(clippy::too_many_lines, clippy::match_same_arms)] // one arm per node kind
    fn node(&mut self, id: NodeId, tight: bool) {
        let ast = self.ast;
        match ast.kind(id) {
            NodeKind::Document => self.children(id, false),
            NodeKind::Paragraph if tight => {
                self.flush_checkbox();
                self.children(id, false);
            }
            NodeKind::Paragraph => {
                self.cr();
                self.push("<p>");
                self.flush_checkbox();
                self.children(id, false);
                self.push("</p>\n");
            }
            NodeKind::Heading(heading) => self.heading(id, heading),
            NodeKind::ThematicBreak => {
                self.cr();
                self.push("<hr />\n");
            }
            NodeKind::BlockQuote => self.container(id, "<blockquote>\n", "</blockquote>\n"),
            NodeKind::List(list) => {
                self.cr();
                let (open, close) = match list.kind {
                    ListKind::Bullet { .. } => ("<ul>\n".to_owned(), "</ul>\n"),
                    ListKind::Ordered { start: 1, .. } => ("<ol>\n".to_owned(), "</ol>\n"),
                    ListKind::Ordered { start, .. } => {
                        (format!("<ol start=\"{start}\">\n"), "</ol>\n")
                    }
                };
                self.push(&open);
                self.children(id, list.tight);
                self.cr();
                self.push(close);
            }
            NodeKind::ListItem(item) => {
                self.cr();
                self.push("<li>");
                self.checkbox = item.task;
                let first_is_paragraph = ast
                    .children(id)
                    .first()
                    .is_some_and(|&c| matches!(ast.kind(c), NodeKind::Paragraph));
                if !first_is_paragraph {
                    self.flush_checkbox();
                }
                self.children(id, tight);
                self.checkbox = None;
                self.push("</li>\n");
            }
            NodeKind::CodeBlock(block) => self.code_block(id, block),
            NodeKind::HtmlBlock(html) => {
                self.cr();
                push_raw_html(&mut self.out, html, self.renderer.options.tagfilter);
            }
            NodeKind::Table(table) => self.table(id, &table.alignments),
            NodeKind::DefinitionList => self.container(id, "<dl>\n", "</dl>\n"),
            NodeKind::DefinitionItem => self.children(id, false),
            NodeKind::DefinitionTerm => self.definition_part(id, "dt"),
            NodeKind::DefinitionDetails => self.definition_part(id, "dd"),
            NodeKind::Admonition(admonition) => self.admonition(id, admonition),
            NodeKind::Callout(callout) => self.callout(id, callout),
            NodeKind::FrontMatter(_)
            | NodeKind::FootnoteDefinition(_)
            | NodeKind::LinkDefinition(_)
            | NodeKind::AbbreviationDefinition(_)
            | NodeKind::IncludeDirective(_)
            | NodeKind::TocDirective(_)
            | NodeKind::DirectiveEnd
            | NodeKind::TableRow(_)
            | NodeKind::TableCell => {}

            NodeKind::Text(text) => self.text(text),
            NodeKind::SoftBreak => self.push("\n"),
            NodeKind::LineBreak => self.push("<br />\n"),
            NodeKind::Code(code) => {
                self.push("<code>");
                push_escaped(&mut self.out, code);
                self.push("</code>");
            }
            NodeKind::Emphasis => self.wrap(id, "<em>", "</em>"),
            NodeKind::Strong => self.wrap(id, "<strong>", "</strong>"),
            NodeKind::Strikethrough => self.wrap(id, "<del>", "</del>"),
            NodeKind::Highlight => self.wrap(id, "<mark>", "</mark>"),
            NodeKind::Superscript => self.wrap(id, "<sup>", "</sup>"),
            NodeKind::Subscript => self.wrap(id, "<sub>", "</sub>"),
            NodeKind::Link(link) => {
                self.push("<a href=\"");
                push_href(&mut self.out, &link.url);
                self.push("\"");
                self.title(link.title.as_deref());
                self.push(">");
                self.children(id, false);
                self.push("</a>");
            }
            NodeKind::Image(link) => {
                self.push("<img src=\"");
                push_href(&mut self.out, &link.url);
                self.push("\" alt=\"");
                push_escaped(&mut self.out, &ast.text(id));
                self.push("\"");
                self.title(link.title.as_deref());
                self.push(" />");
            }
            NodeKind::HtmlInline(html) => {
                push_raw_html(&mut self.out, html, self.renderer.options.tagfilter);
            }
            NodeKind::FootnoteReference(reference) => self.footnote_reference(&reference.label),
            NodeKind::WikiLink(link) => self.wikilink(link),
            NodeKind::Embed(link) => self.embed(link),
            NodeKind::Tag(tag) => {
                self.push("<span class=\"tag\">#");
                push_escaped(&mut self.out, &tag.name);
                self.push("</span>");
            }
            NodeKind::Math(math) => {
                let block = self.at_block_level(id);
                if block {
                    self.cr();
                }
                self.math(&math.literal, math.display);
                if block {
                    self.push("\n");
                }
            }
            NodeKind::Comment(comment) if comment.syntax == CommentSyntax::Html => {
                let block = self.at_block_level(id);
                if block {
                    self.cr();
                }
                self.push("<!--");
                self.push(&comment.literal);
                self.push("-->");
                if block {
                    self.push("\n");
                }
            }
            NodeKind::Comment(_) => {}
            _ => self.children(id, false),
        }
    }

    /// Whether an inline kind (math, comment) stands as a block of its own, directly in
    /// a container of blocks.
    fn at_block_level(&self, id: NodeId) -> bool {
        self.ast.parent(id).is_some_and(|parent| {
            let kind = self.ast.kind(parent);
            kind.is_block()
                && !matches!(
                    kind,
                    NodeKind::Paragraph | NodeKind::Heading(_) | NodeKind::TableCell
                )
        })
    }

    fn container(&mut self, id: NodeId, open: &str, close: &str) {
        self.cr();
        self.push(open);
        self.children(id, false);
        self.cr();
        self.push(close);
    }

    fn wrap(&mut self, id: NodeId, open: &str, close: &str) {
        self.push(open);
        self.children(id, false);
        self.push(close);
    }

    fn title(&mut self, title: Option<&str>) {
        if let Some(title) = title {
            self.push(" title=\"");
            push_escaped(&mut self.out, title);
            self.push("\"");
        }
    }

    fn flush_checkbox(&mut self) {
        let Some(state) = self.checkbox.take() else {
            return;
        };
        let input = match state {
            TaskState::Open => "<input type=\"checkbox\" disabled=\"\" /> ".to_owned(),
            TaskState::Done => "<input type=\"checkbox\" checked=\"\" disabled=\"\" /> ".to_owned(),
            TaskState::Other(c) => format!(
                "<input type=\"checkbox\" disabled=\"\" data-task=\"{}\" /> ",
                escaped(&c.to_string())
            ),
        };
        self.push(&input);
    }

    /// Text, with known abbreviations as `<abbr>`.
    fn text(&mut self, text: &str) {
        let mut rest = text;
        while let Some((at, abbr, expansion)) = self.next_abbreviation(text, rest) {
            push_escaped(&mut self.out, &rest[..at]);
            self.push("<abbr title=\"");
            push_escaped(&mut self.out, expansion);
            self.push("\">");
            push_escaped(&mut self.out, abbr);
            self.push("</abbr>");
            rest = &rest[at + abbr.len()..];
        }
        push_escaped(&mut self.out, rest);
    }

    /// The first abbreviation in `rest` (a suffix of `text`) that stands as a whole word.
    fn next_abbreviation(&self, text: &str, rest: &str) -> Option<(usize, &'a str, &'a str)> {
        let offset = text.len() - rest.len();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut best: Option<(usize, &'a str, &'a str)> = None;
        for &(abbr, expansion) in &self.abbreviations {
            let found = rest.match_indices(abbr).map(|(at, _)| at).find(|&at| {
                let before = text[..offset + at].chars().next_back();
                let after = rest[at + abbr.len()..].chars().next();
                !before.is_some_and(word) && !after.is_some_and(word)
            });
            if let Some(at) = found
                && best.is_none_or(|(b, ..)| at < b)
            {
                best = Some((at, abbr, expansion));
            }
        }
        best
    }

    fn heading(&mut self, id: NodeId, heading: &Heading) {
        let level = heading.level.clamp(1, 6);
        self.cr();
        self.push(&format!("<h{level}"));
        if let Some(anchor) = self.anchors.get(&id).copied() {
            self.push(" id=\"");
            push_escaped(&mut self.out, anchor);
            self.push("\"");
        }
        if let Some(attrs) = &heading.attrs {
            if !attrs.classes.is_empty() {
                self.push(" class=\"");
                push_escaped(&mut self.out, &attrs.classes.join(" "));
                self.push("\"");
            }
            for (name, value) in &attrs.attributes {
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
                if valid && name != "id" && name != "class" {
                    self.push(&format!(" {name}=\""));
                    push_escaped(&mut self.out, value);
                    self.push("\"");
                }
            }
        }
        self.push(">");
        self.children(id, false);
        self.push(&format!("</h{level}>\n"));
    }

    fn code_block(&mut self, id: NodeId, block: &CodeBlock) {
        let mut cx = CodeBlockContext {
            span: self.ast.span(id),
            file: self.doc.name(),
            diagnostics: &mut self.diagnostics,
        };
        if let Some(html) = self
            .renderer
            .code_hooks
            .iter()
            .find_map(|hook| hook.render(block, &mut cx))
        {
            self.cr();
            self.push(&html);
            self.cr();
            return;
        }
        self.cr();
        let highlighted = if self.renderer.options.highlight {
            self.renderer
                .highlighter
                .highlight_html(&block.literal, &block.info)
        } else {
            None
        };
        if highlighted.is_some() {
            self.push(&format!("<pre class=\"{CLASS_PREFIX}code\">"));
        } else {
            self.push("<pre>");
        }
        match block.lang() {
            Some(lang) => {
                self.push("<code class=\"language-");
                push_escaped(&mut self.out, lang);
                self.push("\">");
            }
            None => self.push("<code>"),
        }
        match highlighted {
            Some(html) => self.push(&html),
            None => push_escaped(&mut self.out, &block.literal),
        }
        self.push("</code></pre>\n");
    }

    fn table(&mut self, id: NodeId, alignments: &[Alignment]) {
        let ast = self.ast;
        self.cr();
        self.push("<table>\n");
        let mut body = false;
        for &row in ast.children(id) {
            let header = matches!(ast.kind(row), NodeKind::TableRow(r) if r.header);
            if header {
                self.push("<thead>\n");
            } else if !body {
                self.push("<tbody>\n");
                body = true;
            }
            self.push("<tr>\n");
            let tag = if header { "th" } else { "td" };
            for (i, &cell) in ast.children(row).iter().enumerate() {
                let align = match alignments.get(i) {
                    Some(Alignment::Left) => " align=\"left\"",
                    Some(Alignment::Center) => " align=\"center\"",
                    Some(Alignment::Right) => " align=\"right\"",
                    _ => "",
                };
                self.push(&format!("<{tag}{align}>"));
                self.children(cell, false);
                self.push(&format!("</{tag}>\n"));
            }
            self.push("</tr>\n");
            if header {
                self.push("</thead>\n");
            }
        }
        if body {
            self.push("</tbody>\n");
        }
        self.push("</table>\n");
    }

    /// A `<dt>` or `<dd>`; a lone paragraph inside is written without `<p>`.
    fn definition_part(&mut self, id: NodeId, tag: &str) {
        let ast = self.ast;
        self.cr();
        self.push(&format!("<{tag}>"));
        let lone_paragraph =
            matches!(ast.children(id), [only] if matches!(ast.kind(*only), NodeKind::Paragraph));
        self.children(id, lone_paragraph);
        self.push(&format!("</{tag}>\n"));
    }

    fn admonition(&mut self, id: NodeId, admonition: &Admonition) {
        let title = match admonition.title.as_deref() {
            Some("") if admonition.fold.is_none() => None,
            Some(title) if !title.is_empty() => Some(escaped(title)),
            _ => Some(escaped(&default_title(&admonition.kind))),
        };
        self.boxed(
            id,
            &admonition.kind,
            &admonition.classes,
            title.as_deref(),
            admonition.fold,
        );
    }

    /// A GFM alert or Obsidian callout; an Obsidian title is Markdown.
    fn callout(&mut self, id: NodeId, callout: &Callout) {
        let title = callout
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map_or_else(
                || escaped(&default_title(&callout.kind)),
                |t| self.inline_markdown(t),
            );
        self.boxed(id, &callout.kind, &[], Some(&title), callout.fold);
    }

    /// `text` parsed as Markdown with the document's flavor and rendered without its
    /// paragraph; escaped text when it is not a single paragraph.
    fn inline_markdown(&self, text: &str) -> String {
        let parsed = DocumentOptions::new()
            .flavor(self.doc.flavor().name())
            .parse(text);
        parsed
            .ok()
            .filter(|doc| {
                let ast = doc.ast();
                matches!(ast.children(ast.root()), [only] if matches!(ast.kind(*only), NodeKind::Paragraph))
            })
            .map(|doc| body(self.renderer, &doc).0)
            .and_then(|html| {
                let inner = html.strip_prefix("<p>")?.strip_suffix("</p>\n")?;
                Some(inner.to_owned())
            })
            .unwrap_or_else(|| escaped(text))
    }

    /// An alert, callout or admonition: a `<div>`, or a `<details>` when foldable, with
    /// the classes `admonition` and the lowercased kind; `title` is HTML.
    fn boxed(
        &mut self,
        id: NodeId,
        kind: &str,
        classes: &[String],
        title: Option<&str>,
        fold: Option<Fold>,
    ) {
        let mut class = format!("admonition {}", kind_class(kind));
        for extra in classes {
            class.push(' ');
            class.push_str(extra);
        }
        let class = escaped(&class);
        self.cr();
        match fold {
            None => self.push(&format!("<div class=\"{class}\">\n")),
            Some(Fold::Open) => self.push(&format!("<details class=\"{class}\" open=\"\">\n")),
            Some(Fold::Closed) => self.push(&format!("<details class=\"{class}\">\n")),
        }
        if let Some(title) = title {
            let tag = if fold.is_some() { "summary" } else { "p" };
            self.push(&format!("<{tag} class=\"admonition-title\">"));
            self.push(title);
            self.push(&format!("</{tag}>\n"));
        }
        self.children(id, false);
        self.cr();
        self.push(if fold.is_some() {
            "</details>\n"
        } else {
            "</div>\n"
        });
    }

    fn footnote_reference(&mut self, label: &str) {
        let key = footnote_key(label);
        let Some(&definition) = self.footnotes.definitions.get(&key) else {
            self.push("[^");
            push_escaped(&mut self.out, label);
            self.push("]");
            return;
        };
        let footnotes = &mut self.footnotes;
        let number = *footnotes.numbers.entry(key).or_insert_with(|| {
            footnotes.order.push((definition, 0));
            footnotes.order.len()
        });
        let refs = &mut footnotes.order[number - 1].1;
        *refs += 1;
        let id = backref_id(number, *refs);
        self.push(&format!(
            "<sup class=\"footnote-ref\"><a href=\"#fn-{number}\" id=\"{id}\" data-footnote-ref=\"\">{number}</a></sup>"
        ));
    }

    /// The `<section>` with the referenced footnotes in order of first reference. A
    /// definition referenced only from another footnote is added while writing.
    fn footnote_section(&mut self) {
        if self.footnotes.order.is_empty() {
            return;
        }
        self.cr();
        self.push("<section class=\"footnotes\" data-footnotes=\"\">\n<ol>\n");
        let mut index = 0;
        while index < self.footnotes.order.len() {
            let (definition, _) = self.footnotes.order[index];
            let number = index + 1;
            self.push(&format!("<li id=\"fn-{number}\">\n"));
            let children = self.ast.children(definition);
            let last_paragraph = children
                .last()
                .filter(|&&last| matches!(self.ast.kind(last), NodeKind::Paragraph))
                .copied();
            for &child in children {
                if Some(child) != last_paragraph {
                    self.node(child, false);
                }
            }
            self.cr();
            self.push("<p>");
            if let Some(paragraph) = last_paragraph {
                self.children(paragraph, false);
                self.push(" ");
            }
            // Read the count only now: the footnote's own text may reference it again.
            let refs = self.footnotes.order[index].1;
            let backrefs: Vec<String> = (1..=refs)
                .map(|r| {
                    let id = backref_id(number, r);
                    let (label, mark) = if r == 1 {
                        (number.to_string(), "↩".to_owned())
                    } else {
                        (format!("{number}-{r}"), format!("↩<sup>{r}</sup>"))
                    };
                    format!(
                        "<a href=\"#{id}\" class=\"footnote-backref\" data-footnote-backref=\"\" aria-label=\"Back to reference {label}\">{mark}</a>"
                    )
                })
                .collect();
            self.push(&backrefs.join(" "));
            self.push("</p>\n</li>\n");
            index += 1;
        }
        self.push("</ol>\n</section>\n");
    }

    /// The `href` of a wikilink or embed: the target with `.html` added when it has no
    /// extension, then the anchor. A same-page anchor is the heading id the slugger gave
    /// it; others are made with the Obsidian anchor rules, `^block` ids as written.
    fn wiki_href(&self, link: &WikiLink) -> String {
        let mut href = link.target.clone();
        if !href.is_empty() && Path::new(&href).extension().is_none() {
            href.push_str(".html");
        }
        if let Some(anchor) = &link.anchor {
            let local = link
                .target
                .is_empty()
                .then(|| self.doc.anchors().get(anchor))
                .flatten()
                .and_then(|heading| self.anchors.get(&heading).copied());
            let fragment = match local {
                Some(id) => id.to_owned(),
                None if anchor.starts_with('^') => anchor.clone(),
                None => obsidian_anchor(anchor),
            };
            href.push('#');
            href.push_str(&fragment);
        }
        href
    }

    fn wikilink(&mut self, link: &WikiLink) {
        let text = match (&link.alias, link.target.is_empty(), &link.anchor) {
            (Some(alias), ..) => alias.clone(),
            (None, true, Some(anchor)) => anchor.clone(),
            (None, false, Some(anchor)) => format!("{} > {anchor}", link.target),
            (None, _, None) => link.target.clone(),
        };
        self.push("<a class=\"wikilink\" href=\"");
        let href = self.wiki_href(link);
        push_href(&mut self.out, &href);
        self.push("\">");
        push_escaped(&mut self.out, &text);
        self.push("</a>");
    }

    /// An embed: an image for image files (an alias `W` or `WxH` sets the size), else a
    /// link to the embedded note or file.
    fn embed(&mut self, link: &WikiLink) {
        let extension = Path::new(&link.target)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let image = extension
            .as_deref()
            .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e));
        if !image {
            self.push("<a class=\"wikilink embed\" href=\"");
            let href = self.wiki_href(link);
            push_href(&mut self.out, &href);
            self.push("\">");
            push_escaped(&mut self.out, link.alias.as_deref().unwrap_or(&link.target));
            self.push("</a>");
            return;
        }
        let size = link.alias.as_deref().and_then(|alias| {
            let (width, height) = alias.split_once('x').unwrap_or((alias, ""));
            let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
            (number(width) && (height.is_empty() || number(height))).then_some((width, height))
        });
        let alt = match (size, &link.alias) {
            (None, Some(alias)) => alias.as_str(),
            _ => link.target.as_str(),
        };
        self.push("<img src=\"");
        push_href(&mut self.out, &link.target);
        self.push("\" alt=\"");
        push_escaped(&mut self.out, alt);
        self.push("\"");
        if let Some((width, height)) = size {
            self.push(&format!(" width=\"{width}\""));
            if !height.is_empty() {
                self.push(&format!(" height=\"{height}\""));
            }
        }
        self.push(" />");
    }

    /// TeX as `MathML` via `pulldown-latex`, with the source as annotation. Invalid TeX
    /// shows as `<merror>` in the theme's caution color.
    fn math(&mut self, tex: &str, display: bool) {
        let caution = self.renderer.theme_caution;
        let config = RenderConfig {
            display_mode: if display {
                DisplayMode::Block
            } else {
                DisplayMode::Inline
            },
            annotation: Some(tex),
            error_color: (caution.r, caution.g, caution.b),
            xml: false,
            math_style: MathStyle::TeX,
        };
        let storage = Storage::new();
        let mut mathml = String::new();
        if push_mathml(&mut mathml, Parser::new(tex, &storage), config).is_ok() {
            self.push(&mathml);
        } else {
            self.push("<code class=\"math\">");
            push_escaped(&mut self.out, tex);
            self.push("</code>");
        }
    }
}

/// The id of the `n`th reference to footnote `number`.
fn backref_id(number: usize, n: usize) -> String {
    if n == 1 {
        format!("fnref-{number}")
    } else {
        format!("fnref-{number}-{n}")
    }
}

/// The title of an alert, callout or admonition without one: the kind capitalized.
pub(crate) fn default_title(kind: &str) -> String {
    let mut chars = kind.chars();
    chars.next().map_or_else(String::new, |first| {
        first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect()
    })
}

/// The kind as a CSS class: lowercase, with anything but letters, digits, `-` and `_`
/// as `-`.
pub(super) fn kind_class(kind: &str) -> String {
    kind.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                '-'
            }
        })
        .collect()
}
