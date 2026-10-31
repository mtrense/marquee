//! Down-conversion for [`format_as`](super::format_as): a copy of the document's tree in
//! which the syntax the target flavor cannot express is rewritten into the closest
//! Markdown it can (the rules are in the [module docs](super#down-conversion)). The
//! printer then prints it with the target's syntax and hooks.
//!
//! Nodes that are changed in place keep their spans; new nodes get an empty span where
//! the node they stand for starts (or, for parsed callout titles, their own spans), so
//! the printer's source lookups stay within the document.

use std::path::Path;

use crate::Document;
use crate::ast::{
    Admonition, Alignment, Ast, Callout, CodeBlock, Comment, CommentSyntax, Fence, FrontMatter,
    Heading, Link, LinkForm, ListItem, Math, NodeId, NodeKind, Span, TableRow, TaskState, WikiLink,
};
use crate::flavor::{self, Flavor};
use crate::transform::encode_segment;
use crate::validate::percent_decode;

/// The GFM alert kinds, lowercase.
const ALERT_KINDS: [&str; 5] = ["note", "tip", "important", "warning", "caution"];

/// Extensions of embeds shown as images.
const IMAGE_EXTENSIONS: [&str; 12] = [
    "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "avif", "ico", "tif", "tiff", "apng",
];

/// A converted tree, and what its target reads back.
pub(super) struct Converted {
    pub(super) ast: Ast,
    pub(super) support: Support,
}

/// The syntax the target flavor reads back, each probed by parsing a sample with it, so
/// a custom flavor gets what its parser and passes recognize.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // independent syntax flags
pub(super) struct Support {
    wikilinks: bool,
    embeds: bool,
    tags: bool,
    percent_comments: bool,
    /// Obsidian callouts: any kind, a title and a fold.
    callouts: bool,
    /// GFM alerts: the five kinds, without title or fold.
    alerts: bool,
    admonitions: bool,
    heading_attrs: bool,
    definition_lists: bool,
    abbreviations: bool,
    math: bool,
    /// mkdocs' `\(…\)` and `\[…\]` math.
    pub(super) backslash_math: bool,
    strikethrough: bool,
    highlight: bool,
    superscript: bool,
    subscript: bool,
    tables: bool,
    tasks: bool,
    footnotes: bool,
    autolinks: bool,
}

impl Support {
    fn of(target: &dyn Flavor) -> Self {
        let probe = |src: &str, found: fn(&NodeKind) -> bool| {
            let parsed = flavor::parse(target, src, Path::new("<probe>"));
            let ast = &parsed.ast;
            ast.descendants(ast.root()).any(|id| found(ast.kind(id)))
        };
        Self {
            wikilinks: probe("[[a]]\n", |kind| matches!(kind, NodeKind::WikiLink(_))),
            embeds: probe("![[a]]\n", |kind| matches!(kind, NodeKind::Embed(_))),
            tags: probe("#a\n", |kind| matches!(kind, NodeKind::Tag(_))),
            percent_comments: probe(
                "%%a%%\n",
                |kind| matches!(kind, NodeKind::Comment(c) if c.syntax == CommentSyntax::Percent),
            ),
            callouts: probe(
                "> [!a]- b\n> c\n",
                |kind| matches!(kind, NodeKind::Callout(c) if c.fold.is_some() && c.title.is_some()),
            ),
            alerts: probe("> [!NOTE]\n> a\n", |kind| {
                matches!(kind, NodeKind::Callout(_))
            }),
            admonitions: probe("!!! note\n    a\n", |kind| {
                matches!(kind, NodeKind::Admonition(_))
            }),
            heading_attrs: probe(
                "# a {#b}\n",
                |kind| matches!(kind, NodeKind::Heading(h) if h.attrs.is_some()),
            ),
            definition_lists: probe("a\n:   b\n", |kind| {
                matches!(kind, NodeKind::DefinitionList)
            }),
            abbreviations: probe("*[a]: b\n", |kind| {
                matches!(kind, NodeKind::AbbreviationDefinition(_))
            }),
            math: probe("$a$\n", |kind| matches!(kind, NodeKind::Math(_))),
            backslash_math: probe("\\(a\\)\n", |kind| matches!(kind, NodeKind::Math(_))),
            strikethrough: probe("~~a~~\n", |kind| matches!(kind, NodeKind::Strikethrough)),
            highlight: probe("==a==\n", |kind| matches!(kind, NodeKind::Highlight)),
            superscript: probe("^a^\n", |kind| matches!(kind, NodeKind::Superscript)),
            subscript: probe("~a~\n", |kind| matches!(kind, NodeKind::Subscript)),
            tables: probe("| a |\n| - |\n", |kind| matches!(kind, NodeKind::Table(_))),
            tasks: probe(
                "- [ ] a\n",
                |kind| matches!(kind, NodeKind::ListItem(item) if item.task.is_some()),
            ),
            footnotes: probe("a[^1]\n\n[^1]: b\n", |kind| {
                matches!(kind, NodeKind::FootnoteReference(_))
            }),
            autolinks: probe(
                "www.example.com\n",
                |kind| matches!(kind, NodeKind::Link(link) if link.form == LinkForm::Bare),
            ),
        }
    }
}

/// `document`'s tree converted for `target`.
pub(super) fn convert(document: &Document, target: &dyn Flavor) -> Converted {
    let mut converter = Converter {
        ast: document.ast().clone(),
        src: document.text(),
        document,
        target,
        support: Support::of(target),
        anchors: Vec::new(),
    };
    converter.blocks();
    converter.anchors = converter.anchor_pairs();
    converter.inlines();
    Converted {
        ast: converter.ast,
        support: converter.support,
    }
}

/// One heading's anchor in the document's flavor and in the target's.
struct Anchors {
    /// The heading's text, for same-page wikilinks.
    text: String,
    source: String,
    target: String,
}

struct Converter<'a> {
    ast: Ast,
    src: &'a str,
    document: &'a Document,
    target: &'a dyn Flavor,
    support: Support,
    anchors: Vec<Anchors>,
}

impl Converter<'_> {
    /// The nodes of the tree in document order, for a pass that may restructure it.
    fn nodes(&self) -> Vec<NodeId> {
        self.ast.descendants(self.ast.root()).collect()
    }

    /// Block conversions: everything that changes the block structure, so the inline
    /// pass sees the final tree (callout titles included).
    fn blocks(&mut self) {
        for id in self.nodes() {
            if !self.ast.is_attached(id) {
                continue;
            }
            match self.ast.kind(id).clone() {
                NodeKind::FrontMatter(front) => self.frontmatter(id, &front),
                NodeKind::Heading(heading) if heading.attrs.is_some() => {
                    if !self.support.heading_attrs {
                        self.ast.set_kind(
                            id,
                            NodeKind::Heading(Heading {
                                attrs: None,
                                ..heading
                            }),
                        );
                    }
                }
                NodeKind::Callout(callout) => self.callout(id, &callout),
                NodeKind::Admonition(admonition) => self.admonition(id, &admonition),
                NodeKind::DefinitionList if !self.support.definition_lists => {
                    self.definition_list(id);
                }
                NodeKind::AbbreviationDefinition(_) if !self.support.abbreviations => {
                    self.detach(id);
                }
                NodeKind::Table(table) if !self.support.tables => {
                    self.table(id, &table.alignments);
                }
                NodeKind::ListItem(ListItem { task: Some(task) }) if !self.support.tasks => {
                    self.task(id, task);
                }
                NodeKind::FootnoteDefinition(def) if !self.support.footnotes => {
                    self.footnote_definition(id, &def.label);
                }
                NodeKind::Paragraph if !self.support.math => {
                    if let [only] = self.ast.children(id)
                        && let NodeKind::Math(math) = self.ast.kind(*only).clone()
                        && math.display
                    {
                        let literal = self.math_literal(*only, &math);
                        self.ast.take_children(id);
                        self.ast.set_kind(id, math_code(&literal));
                    }
                }
                NodeKind::Math(math) if !self.support.math && !self.in_inlines(id) => {
                    let literal = self.math_literal(id, &math);
                    self.ast.set_kind(id, math_code(&literal));
                }
                _ => {}
            }
        }
    }

    /// Inline conversions, and links to anchors of the document that moved.
    fn inlines(&mut self) {
        for id in self.nodes() {
            if !self.ast.is_attached(id) {
                continue;
            }
            let support = self.support;
            match self.ast.kind(id).clone() {
                NodeKind::WikiLink(link) if !support.wikilinks => self.wikilink(id, &link),
                NodeKind::Embed(link) if !support.embeds => self.embed(id, &link),
                NodeKind::Tag(tag) if !support.tags => {
                    self.ast
                        .set_kind(id, NodeKind::Text(format!("#{}", tag.name)));
                }
                NodeKind::Comment(comment)
                    if comment.syntax == CommentSyntax::Percent && !support.percent_comments =>
                {
                    self.percent_comment(id, &comment);
                }
                NodeKind::Math(math) => self.math(id, &math),
                NodeKind::Strikethrough if !support.strikethrough => self.html_wrap(id, "del"),
                NodeKind::Highlight if !support.highlight => self.html_wrap(id, "mark"),
                NodeKind::Superscript if !support.superscript => self.html_wrap(id, "sup"),
                NodeKind::Subscript if !support.subscript => self.html_wrap(id, "sub"),
                NodeKind::FootnoteReference(reference) if !support.footnotes => {
                    self.ast
                        .set_kind(id, NodeKind::HtmlInline(superscript(&reference.label)));
                }
                NodeKind::Link(link) => self.link(id, link),
                _ => {}
            }
        }
    }

    fn detach(&mut self, id: NodeId) {
        self.ast
            .detach(id)
            .expect("only attached non-root nodes are converted");
    }

    fn insert_before(&mut self, sibling: NodeId, node: NodeId) {
        self.ast
            .insert_before(sibling, node)
            .expect("a new node goes before an attached one");
    }

    fn append(&mut self, parent: NodeId, child: NodeId) {
        self.ast
            .append_child(parent, child)
            .expect("a new node goes into a new parent");
    }

    /// A new node with an empty span at `at`.
    fn create(&mut self, kind: NodeKind, at: usize) -> NodeId {
        self.ast.create(kind, Span::new(at, at))
    }

    fn start(&self, id: NodeId) -> usize {
        self.ast.span(id).start
    }

    // --- Blocks -----------------------------------------------------------------------

    /// `mrq.flavor` in the frontmatter names the target, so the output is read with it.
    fn frontmatter(&mut self, id: NodeId, front: &FrontMatter) {
        let Some(entry) = self
            .document
            .frontmatter()
            .and_then(|fm| fm.entry(&["mrq", "flavor"]))
        else {
            return;
        };
        let span = self.ast.span(id);
        let Some(offset) = self.src[span.start..span.end]
            .find(&front.literal)
            .map(|at| span.start + at)
        else {
            return;
        };
        let value = entry.value;
        if value.start < offset || value.end > offset + front.literal.len() {
            return;
        }
        let old = &front.literal[value.start - offset..value.end - offset];
        let quote = old
            .chars()
            .next()
            .filter(|&c| c == '"' || c == '\'')
            .map(String::from)
            .unwrap_or_default();
        let literal = format!(
            "{}{quote}{}{quote}{}",
            &front.literal[..value.start - offset],
            self.target.name(),
            &front.literal[value.end - offset..]
        );
        self.ast.set_kind(
            id,
            NodeKind::FrontMatter(FrontMatter {
                literal,
                format: front.format,
            }),
        );
        // An empty span: printed from the literal, not the source.
        self.ast.set_span(id, Span::new(span.start, span.start));
    }

    /// A callout as the target has it: unchanged, an admonition, a GFM alert (the title
    /// as a bold first paragraph) or a block quote with a bold title.
    fn callout(&mut self, id: NodeId, callout: &Callout) {
        if self.support.callouts {
            return;
        }
        let alert = self.support.alerts && is_alert_kind(&callout.kind);
        let title = callout.title.as_deref().filter(|title| !title.is_empty());
        if alert && title.is_none() && callout.fold.is_none() {
            self.set_alert(id, &callout.kind);
            return;
        }
        if self.support.admonitions {
            self.ast.set_kind(
                id,
                NodeKind::Admonition(Admonition {
                    kind: callout.kind.to_lowercase(),
                    classes: Vec::new(),
                    title: title.map(str::to_owned),
                    fold: callout.fold,
                }),
            );
            return;
        }
        let inlines = match title {
            Some(title) if alert && title.eq_ignore_ascii_case(&callout.kind) => None,
            Some(title) => Some(self.callout_title(id, title)),
            None if alert => None,
            None => Some(vec![self.create(
                NodeKind::Text(default_title(&callout.kind)),
                self.start(id),
            )]),
        };
        self.boxed(id, &callout.kind, alert, inlines);
    }

    /// An admonition as the target has it: unchanged, a callout, a GFM alert (the title
    /// as a bold first paragraph) or a block quote with a bold title (none for `""`).
    fn admonition(&mut self, id: NodeId, admonition: &Admonition) {
        if self.support.admonitions {
            return;
        }
        if self.support.callouts {
            let title = admonition.title.clone().filter(|title| !title.is_empty());
            self.ast.set_kind(
                id,
                NodeKind::Callout(Callout {
                    kind: admonition.kind.clone(),
                    title,
                    fold: admonition.fold,
                }),
            );
            return;
        }
        let alert = self.support.alerts && is_alert_kind(&admonition.kind);
        let title = match admonition.title.as_deref() {
            Some("") => None,
            Some(title) if alert && title.eq_ignore_ascii_case(&admonition.kind) => None,
            Some(title) => Some(title.to_owned()),
            None if alert => None,
            None => Some(default_title(&admonition.kind)),
        };
        let inlines = title.map(|title| vec![self.create(NodeKind::Text(title), self.start(id))]);
        self.boxed(id, &admonition.kind, alert, inlines);
    }

    /// `id` as a GFM alert of `kind`, or a block quote, with `title` as a bold first
    /// paragraph.
    fn boxed(&mut self, id: NodeId, kind: &str, alert: bool, title: Option<Vec<NodeId>>) {
        if alert {
            self.set_alert(id, kind);
        } else {
            self.ast.set_kind(id, NodeKind::BlockQuote);
        }
        if let Some(inlines) = title {
            let paragraph = self.bold_paragraph(inlines, self.start(id));
            self.ast
                .insert_child(id, 0, paragraph)
                .expect("a new paragraph goes first");
        }
    }

    fn set_alert(&mut self, id: NodeId, kind: &str) {
        self.ast.set_kind(
            id,
            NodeKind::Callout(Callout {
                kind: kind.to_uppercase(),
                title: None,
                fold: None,
            }),
        );
    }

    /// A paragraph of `inlines` in strong emphasis.
    fn bold_paragraph(&mut self, inlines: Vec<NodeId>, at: usize) -> NodeId {
        let paragraph = self.create(NodeKind::Paragraph, at);
        let strong = self.create(NodeKind::Strong, at);
        for inline in inlines {
            self.append(strong, inline);
        }
        self.append(paragraph, strong);
        paragraph
    }

    /// The inlines of a callout's title, which is Markdown source on its marker line:
    /// parsed with the document's flavor, with spans into the document.
    fn callout_title(&mut self, id: NodeId, title: &str) -> Vec<NodeId> {
        let start = self.start(id);
        let line_end = self.src[start..]
            .find('\n')
            .map_or(self.src.len(), |nl| start + nl);
        let line = self.src[start..line_end].trim_end();
        let offset = start + line.len() - title.len().min(line.len());
        if !self.src[offset..].starts_with(title) {
            return vec![self.create(NodeKind::Text(title.to_owned()), start)];
        }
        let parsed = flavor::parse(self.document.flavor(), title, self.document.name());
        let fragment = parsed.ast;
        let block = fragment.children(fragment.root()).first().copied();
        let Some(block) = block.filter(|&block| *fragment.kind(block) == NodeKind::Paragraph)
        else {
            return vec![self.create(NodeKind::Text(title.to_owned()), start)];
        };
        let shift = |span: Span| Span::new(span.start + offset, span.end + offset);
        fragment
            .children(block)
            .iter()
            .map(|&child| self.ast.import(&fragment, child, shift))
            .collect()
    }

    /// Each term as a paragraph in strong emphasis, followed by the blocks of its
    /// details.
    fn definition_list(&mut self, list: NodeId) {
        let mut blocks = Vec::new();
        for item in self.ast.children(list).to_vec() {
            for part in self.ast.children(item).to_vec() {
                let at = self.start(part);
                match self.ast.kind(part) {
                    NodeKind::DefinitionTerm => {
                        let children = self.ast.take_children(part);
                        let inline = children
                            .iter()
                            .all(|&child| self.ast.kind(child).is_inline());
                        if inline {
                            blocks.push(self.bold_paragraph(children, at));
                            continue;
                        }
                        let mut rest = children.into_iter();
                        if let Some(first) = rest.next() {
                            if *self.ast.kind(first) == NodeKind::Paragraph {
                                let inlines = self.ast.take_children(first);
                                blocks.push(self.bold_paragraph(inlines, at));
                            } else {
                                blocks.push(first);
                            }
                        }
                        blocks.extend(rest);
                    }
                    _ => blocks.extend(self.ast.take_children(part)),
                }
            }
        }
        for block in blocks {
            self.insert_before(list, block);
        }
        self.detach(list);
    }

    /// An HTML table whose cells hold their content as Markdown paragraphs, between
    /// blank lines so `CommonMark` reads them as Markdown.
    fn table(&mut self, table: NodeId, alignments: &[Alignment]) {
        let at = self.start(table);
        let mut parts: Vec<NodeId> = Vec::new();
        let mut html = vec!["<table>".to_owned()];
        let rows = self.ast.children(table).to_vec();
        let header = rows
            .iter()
            .take_while(|&&row| {
                matches!(
                    self.ast.kind(row),
                    NodeKind::TableRow(TableRow { header: true })
                )
            })
            .count();
        for (index, &row) in rows.iter().enumerate() {
            if index == 0 && header > 0 {
                html.push("<thead>".to_owned());
            }
            if index == header {
                html.push("<tbody>".to_owned());
            }
            html.push("<tr>".to_owned());
            let tag = if index < header { "th" } else { "td" };
            for (column, cell) in self.ast.children(row).to_vec().into_iter().enumerate() {
                let align = match alignments.get(column) {
                    Some(Alignment::Left) => " align=\"left\"",
                    Some(Alignment::Center) => " align=\"center\"",
                    Some(Alignment::Right) => " align=\"right\"",
                    _ => "",
                };
                let open = format!("<{tag}{align}>");
                let children = self.ast.take_children(cell);
                if children.is_empty() {
                    html.push(format!("{open}</{tag}>"));
                    continue;
                }
                html.push(open);
                parts.push(self.html_block(&html, at));
                let paragraph = self.create(NodeKind::Paragraph, self.start(cell));
                for child in children {
                    self.append(paragraph, child);
                }
                parts.push(paragraph);
                html = vec![format!("</{tag}>")];
            }
            html.push("</tr>".to_owned());
            if index + 1 == header {
                html.push("</thead>".to_owned());
            }
        }
        if rows.len() > header {
            html.push("</tbody>".to_owned());
        }
        html.push("</table>".to_owned());
        parts.push(self.html_block(&html, at));
        for part in parts {
            self.insert_before(table, part);
        }
        self.detach(table);
    }

    /// A task item as a plain item whose text starts with its marker, which the
    /// target reads as text.
    fn task(&mut self, item: NodeId, task: TaskState) {
        self.ast
            .set_kind(item, NodeKind::ListItem(ListItem { task: None }));
        let marker = match task {
            TaskState::Open => ' ',
            TaskState::Done => 'x',
            TaskState::Other(c) => c,
        };
        let at = self.start(item);
        let text = self.create(NodeKind::Text(format!("[{marker}] ")), at);
        match self.ast.children(item).first().copied() {
            Some(first) if *self.ast.kind(first) == NodeKind::Paragraph => {
                self.ast
                    .insert_child(first, 0, text)
                    .expect("a new node goes first");
            }
            _ => {
                let paragraph = self.create(NodeKind::Paragraph, at);
                self.append(paragraph, text);
                self.ast
                    .insert_child(item, 0, paragraph)
                    .expect("a new paragraph goes first");
            }
        }
    }

    fn html_block(&mut self, lines: &[String], at: usize) -> NodeId {
        let mut html = lines.join("\n");
        html.push('\n');
        self.create(NodeKind::HtmlBlock(html), at)
    }

    /// The blocks of the definition where it was, its first paragraph starting with its
    /// label as a superscript.
    fn footnote_definition(&mut self, def: NodeId, label: &str) {
        let at = self.start(def);
        let marker = self.create(NodeKind::HtmlInline(superscript(label)), at);
        let mut children = self.ast.take_children(def);
        match children.first() {
            Some(&first) if *self.ast.kind(first) == NodeKind::Paragraph => {
                let space = self.create(NodeKind::Text(" ".to_owned()), at);
                self.ast
                    .insert_child(first, 0, space)
                    .expect("a new node goes first");
                self.ast
                    .insert_child(first, 0, marker)
                    .expect("a new node goes first");
            }
            _ => {
                let paragraph = self.create(NodeKind::Paragraph, at);
                self.append(paragraph, marker);
                children.insert(0, paragraph);
            }
        }
        for child in children {
            self.insert_before(def, child);
        }
        self.detach(def);
    }

    // --- Inlines ----------------------------------------------------------------------

    /// Each heading's anchor in the document's flavor and in the target's (after the
    /// block pass, so dropped `{#id}`s give way to slugs).
    fn anchor_pairs(&self) -> Vec<Anchors> {
        let source = self.document.ast();
        let source_anchors =
            flavor::heading_anchors(source, &mut *self.document.flavor().slugger());
        let target_anchors = flavor::heading_anchors(&self.ast, &mut *self.target.slugger());
        source_anchors
            .into_iter()
            .filter_map(|(id, source_anchor)| {
                let target = target_anchors
                    .iter()
                    .find(|(heading, _)| *heading == id)?
                    .1
                    .clone();
                Some(Anchors {
                    text: source.text(id),
                    source: source_anchor,
                    target,
                })
            })
            .collect()
    }

    /// The target's anchor for a heading of this document written as `text` (ignoring
    /// case, as Obsidian does), else the target's slug of `text`.
    fn heading_anchor(&self, text: &str) -> String {
        self.anchors
            .iter()
            .find(|anchors| anchors.text.trim().eq_ignore_ascii_case(text))
            .map_or_else(|| self.target.slugger().slug(text), |a| a.target.clone())
    }

    /// `[[target#anchor|alias]]` as `[alias](target.md#slug)`.
    fn wikilink(&mut self, id: NodeId, link: &WikiLink) {
        let url = self.wikilink_url(link);
        let text = wikilink_text(link);
        self.ast.set_kind(id, NodeKind::Link(Link::inline(url)));
        let text = self.create(NodeKind::Text(text), self.start(id));
        self.append(id, text);
    }

    /// `![[image.png|alt]]` as an image, other embeds as links.
    fn embed(&mut self, id: NodeId, link: &WikiLink) {
        let target = link.target.trim();
        if !is_image(target) {
            self.wikilink(id, link);
            return;
        }
        let alt = link
            .alias
            .as_deref()
            .map(str::trim)
            .filter(|alias| !alias.is_empty() && !is_size(alias))
            .map_or_else(
                || target.rsplit('/').next().unwrap_or(target).to_owned(),
                str::to_owned,
            );
        self.ast
            .set_kind(id, NodeKind::Image(Link::inline(encode_path(target))));
        let alt = self.create(NodeKind::Text(alt), self.start(id));
        self.append(id, alt);
    }

    /// The destination of a wikilink: the note's file (`.md` added unless it has an
    /// extension) and the target slug of its heading; block references (`#^id`) have
    /// no anchor to go to.
    fn wikilink_url(&self, link: &WikiLink) -> String {
        let target = link.target.trim();
        let mut url = if target.is_empty() {
            String::new()
        } else if has_extension(target) {
            encode_path(target)
        } else {
            encode_path(&format!("{target}.md"))
        };
        let heading = link
            .anchor
            .as_deref()
            .and_then(|anchor| anchor.rsplit('#').next())
            .map(str::trim)
            .filter(|heading| !heading.is_empty() && !heading.starts_with('^'));
        if let Some(heading) = heading {
            let slug = if target.is_empty() || self.is_this_document(target) {
                self.heading_anchor(heading)
            } else {
                self.target.slugger().slug(heading)
            };
            url.push('#');
            url.push_str(&encode_segment(&slug));
        }
        url
    }

    /// Whether the wikilink target `target` names this document.
    fn is_this_document(&self, target: &str) -> bool {
        let stem = self.document.path().and_then(|path| path.file_stem());
        stem.is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(target))
    }

    /// A `%%` comment as an HTML comment, or nothing where that cannot hold it.
    fn percent_comment(&mut self, id: NodeId, comment: &Comment) {
        let block = if self.in_inlines(id) {
            self.block_start(id)
        } else {
            self.start(id)
        };
        let literal = super::unprefixed(self.src, &comment.literal, block);
        let unsafe_html = literal.contains("-->")
            || literal.contains("--!>")
            || literal.starts_with('>')
            || literal.starts_with("->")
            || literal.ends_with('-')
            || literal.trim_start().starts_with("mrq:");
        if unsafe_html {
            self.detach(id);
        } else {
            self.ast.set_kind(
                id,
                NodeKind::Comment(Comment {
                    syntax: CommentSyntax::Html,
                    literal,
                }),
            );
        }
    }

    /// Math the target reads in another form: mkdocs' `\(…\)` as `$…$`, or, without
    /// math, the TeX with its delimiters as text.
    fn math(&mut self, id: NodeId, math: &Math) {
        let backslash = self.is_backslash_math(id);
        if self.support.math && (!backslash || self.support.backslash_math) {
            return;
        }
        let literal = self.math_literal(id, math);
        if self.support.math {
            self.ast.set_kind(
                id,
                NodeKind::Math(Math {
                    display: math.display,
                    literal,
                }),
            );
        } else {
            let delimiter = if math.display { "$$" } else { "$" };
            self.ast.set_kind(
                id,
                NodeKind::Text(format!("{delimiter}{literal}{delimiter}")),
            );
        }
    }

    fn is_backslash_math(&self, id: NodeId) -> bool {
        let start = self.start(id);
        matches!(self.src.get(start..start + 2), Some("\\(" | "\\["))
    }

    /// The TeX of `math` without container prefixes (which backslash math keeps).
    fn math_literal(&self, id: NodeId, math: &Math) -> String {
        if self.is_backslash_math(id) {
            super::unprefixed(self.src, &math.literal, self.block_start(id))
        } else {
            math.literal.clone()
        }
    }

    /// `id`'s children between `<tag>` and `</tag>`, in its place.
    fn html_wrap(&mut self, id: NodeId, tag: &str) {
        let span = self.ast.span(id);
        let open = self.create(NodeKind::HtmlInline(format!("<{tag}>")), span.start);
        let close = self.create(NodeKind::HtmlInline(format!("</{tag}>")), span.end);
        self.insert_before(id, open);
        for child in self.ast.take_children(id) {
            self.insert_before(id, child);
        }
        self.insert_before(id, close);
        self.detach(id);
    }

    /// Bare URLs as autolinks where the target has no extended autolinks, and links to
    /// a heading of this document at the anchor the target gives it.
    fn link(&mut self, id: NodeId, mut link: Link) {
        let mut changed = false;
        if link.form == LinkForm::Bare && !self.support.autolinks {
            let text = self.ast.text(id);
            link.form = if link.url == text || link.url == format!("mailto:{text}") {
                LinkForm::Autolink
            } else {
                LinkForm::Inline
            };
            changed = true;
        }
        if link.form == LinkForm::Inline
            && let Some(fragment) = link.url.strip_prefix('#')
        {
            let fragment = percent_decode(fragment);
            let ignore_case = self.document.flavor().case_insensitive_anchors();
            let found = self.anchors.iter().find(|anchors| {
                anchors.source == fragment
                    || (ignore_case && anchors.source.eq_ignore_ascii_case(&fragment))
            });
            if let Some(anchors) = found
                && anchors.target != fragment
            {
                link.url = format!("#{}", encode_segment(&anchors.target));
                changed = true;
            }
        }
        if changed {
            self.ast.set_kind(id, NodeKind::Link(link));
        }
    }

    /// Whether `id` is printed among inlines (of a paragraph, heading, cell or term),
    /// rather than as a block of its own.
    fn in_inlines(&self, id: NodeId) -> bool {
        self.ast.parent(id).is_some_and(|parent| {
            let kind = self.ast.kind(parent);
            kind.is_inline()
                || matches!(
                    kind,
                    NodeKind::Paragraph
                        | NodeKind::Heading(_)
                        | NodeKind::TableCell
                        | NodeKind::DefinitionTerm
                )
        })
    }

    /// The start of the closest block around `id`, as the printer finds it.
    fn block_start(&self, id: NodeId) -> usize {
        self.ast
            .ancestors(id)
            .find(|&ancestor| self.ast.kind(ancestor).is_block())
            .map_or_else(|| self.start(id), |block| self.start(block))
    }
}

/// A fenced `math` code block of `literal`, as GitLab, GitHub and many renderers draw
/// it.
fn math_code(literal: &str) -> NodeKind {
    let mut literal = literal.trim_matches('\n').to_owned();
    literal.push('\n');
    NodeKind::CodeBlock(CodeBlock {
        fence: Some(Fence { char: '`', len: 3 }),
        info: "math".to_owned(),
        literal,
    })
}

/// A footnote label as `<sup>label</sup>`.
fn superscript(label: &str) -> String {
    let mut html = String::from("<sup>");
    for c in label.chars() {
        match c {
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '&' => html.push_str("&amp;"),
            c => html.push(c),
        }
    }
    html.push_str("</sup>");
    html
}

/// The text of a converted wikilink: the alias, else `target > heading` (the
/// heading alone for the same page), as Obsidian shows it.
fn wikilink_text(link: &WikiLink) -> String {
    if let Some(alias) = link.alias.as_deref().map(str::trim)
        && !alias.is_empty()
    {
        return alias.to_owned();
    }
    let target = link.target.trim();
    match link.anchor.as_deref().map(str::trim) {
        Some(anchor) if !anchor.is_empty() => {
            let anchor = anchor.replace('#', " > ");
            if target.is_empty() {
                anchor
            } else {
                format!("{target} > {anchor}")
            }
        }
        _ => target.to_owned(),
    }
}

/// `path` percent-encoded segment by segment.
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether a wikilink target names a file with an extension (an attachment or a
/// `.md` note), rather than a note by name: `report.pdf`, but not `Version 1.2`.
fn has_extension(target: &str) -> bool {
    Path::new(target)
        .extension()
        .map(|ext| ext.to_string_lossy())
        .is_some_and(|ext| {
            !ext.is_empty()
                && ext.len() <= 8
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && !ext.chars().all(|c| c.is_ascii_digit())
        })
}

fn is_image(target: &str) -> bool {
    Path::new(target)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// An embed size: `100` or `100x200`.
fn is_size(alias: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    match alias.split_once('x') {
        Some((width, height)) => digits(width) && digits(height),
        None => digits(alias),
    }
}

fn is_alert_kind(kind: &str) -> bool {
    ALERT_KINDS
        .iter()
        .any(|alert| alert.eq_ignore_ascii_case(kind))
}

/// The title of an untitled callout or admonition: its kind, capitalized.
fn default_title(kind: &str) -> String {
    let mut chars = kind.chars();
    chars.next().map_or_else(String::new, |first| {
        first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect()
    })
}
