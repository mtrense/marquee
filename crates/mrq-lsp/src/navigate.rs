//! Navigation: what is at a position ([`item_at`]), and the answers to
//! `textDocument/definition` ([`definition`]), `textDocument/references`
//! ([`references`]) and `textDocument/hover` ([`hover`]).
//!
//! Links resolve as `mrq validate` resolves them, through a [`Resolver`] of the
//! document's root: files in the index, else on disk, and wikilinks in the root's vault.
//! A link's destination is the line its anchor points at (a heading, or an Obsidian
//! `^block` line), else the top of the file.

use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::ast::{NodeKind, NodeType, Span};
use mrq_core::document::TocEntry;
use mrq_core::frontmatter::Frontmatter;
use mrq_core::source::{PositionEncoding, SourceFile};
use mrq_core::validate::{
    Destination, DocumentLink, ReferenceDefinition, Resolver, Target, normalize_label,
    reference_definitions,
};
use tower_lsp_server::ls_types::{
    Hover, HoverContents, Location, MarkupContent, MarkupKind, Range,
};

use crate::convert::{self, path_to_uri};
use crate::index::{Root, WorkspaceIndex};

/// How many lines of a link's target [`hover`] shows at most.
pub const PREVIEW_LINES: usize = 10;

/// Something of a document that navigation knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A link, image, wikilink or embed.
    Link(DocumentLink),
    /// A footnote reference `[^label]`.
    FootnoteReference { label: String, span: Span },
    /// A footnote definition; `span` is all of it.
    FootnoteDefinition { label: String, span: Span },
    /// A link reference definition `[label]: destination`.
    ReferenceDefinition(ReferenceDefinition),
    /// A heading.
    Heading(TocEntry),
}

impl Item {
    /// The item's source span.
    pub const fn span(&self) -> Span {
        match self {
            Self::Link(link) => link.span,
            Self::FootnoteReference { span, .. } | Self::FootnoteDefinition { span, .. } => *span,
            Self::ReferenceDefinition(definition) => definition.span,
            Self::Heading(entry) => entry.span,
        }
    }
}

/// The item of `document` at byte `offset`, given the document's `links`: the innermost
/// link, else a footnote reference, a reference definition, a footnote definition or a
/// heading, in that order (a link in a heading is the link).
pub fn item_at(document: &Document, links: &[DocumentLink], offset: usize) -> Option<Item> {
    let at = |span: Span| span.start <= offset && offset < span.end;
    if let Some(link) = links
        .iter()
        .filter(|link| at(link.span))
        .min_by_key(|link| link.span.end - link.span.start)
    {
        return Some(Item::Link(link.clone()));
    }
    let ast = document.ast();
    let mut definition = None;
    for node in ast.descendants(ast.root()) {
        let span = ast.span(node);
        if !at(span) {
            continue;
        }
        match ast.kind(node) {
            NodeKind::FootnoteReference(reference) => {
                return Some(Item::FootnoteReference {
                    label: reference.label.clone(),
                    span,
                });
            }
            NodeKind::FootnoteDefinition(footnote) => {
                definition = Some(Item::FootnoteDefinition {
                    label: footnote.label.clone(),
                    span,
                });
            }
            _ => {}
        }
    }
    if let Some(reference) = reference_definitions(document)
        .into_iter()
        .find(|reference| at(reference.span))
    {
        return Some(Item::ReferenceDefinition(reference));
    }
    definition.or_else(|| {
        document
            .toc()
            .into_iter()
            .find(|entry| at(entry.span))
            .map(Item::Heading)
    })
}

/// Where the item at `offset` of the document at `path` leads: a link's destination, a
/// reference link's definition, a reference definition's destination, a footnote
/// reference's definition (its first line). `None` for anything else, a missing target,
/// and external links.
pub fn definition(
    index: &WorkspaceIndex,
    path: &Path,
    offset: usize,
    encoding: PositionEncoding,
) -> Option<Location> {
    let (root, document) = indexed(index, path)?;
    let mut resolver = Resolver::new(root.workspace());
    let destination = match item_at(document, root.links(path), offset)? {
        Item::Link(link) => {
            let reference = link.reference.as_deref();
            if let Some(definition) = reference.and_then(|l| reference_definition(document, l)) {
                return location(path, Some(document.source()), definition.span, encoding);
            }
            resolver.destination(document, &link)?
        }
        Item::ReferenceDefinition(definition) => {
            let target = reference_target(&resolver, document, &definition);
            resolver.target_destination(document, &target, false)?
        }
        Item::FootnoteReference { label, .. } => {
            let span = footnote_definition(document, &label)?;
            let line = line_span(document.text(), span.start);
            return location(path, Some(document.source()), line, encoding);
        }
        _ => return None,
    };
    destination_location(&mut resolver, document, &destination, encoding)
}

/// What [`references`] looks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wanted {
    /// Links to the heading (or block) at `offset` of `path`, by an anchor.
    Anchor { path: PathBuf, offset: usize },
    /// Links to the file `path` from anywhere, with or without an anchor.
    File(PathBuf),
}

/// The links (and reference definitions) of every indexed document that point where the
/// item at `offset` of the document at `path` is: a heading's anchor; a link's or
/// reference definition's destination; else the document itself. With
/// `include_declaration`, an anchor's heading too. Sorted by URI and position.
pub fn references(
    index: &WorkspaceIndex,
    path: &Path,
    offset: usize,
    include_declaration: bool,
    encoding: PositionEncoding,
) -> Vec<Location> {
    let Some((root, document)) = indexed(index, path) else {
        return Vec::new();
    };
    let mut resolver = Resolver::new(root.workspace());
    let wanted = match item_at(document, root.links(path), offset) {
        Some(Item::Heading(entry)) => Some(Wanted::Anchor {
            path: path.to_owned(),
            offset: entry.span.start,
        }),
        Some(Item::Link(link)) => resolver
            .destination(document, &link)
            .and_then(|destination| wanted(path, &destination, &link.target)),
        Some(Item::ReferenceDefinition(definition)) => {
            let target = reference_target(&resolver, document, &definition);
            resolver
                .target_destination(document, &target, false)
                .and_then(|destination| wanted(path, &destination, &target))
        }
        _ => Some(Wanted::File(path.to_owned())),
    };
    let Some(wanted) = wanted else {
        return Vec::new();
    };
    let mut found = Vec::new();
    if include_declaration && let Wanted::Anchor { path, offset } = &wanted {
        let source = resolver.markdown(path).map(Document::source);
        if let Some(source) = source {
            let line = line_span(source.text(), *offset);
            found.extend(location(path, Some(source), line, encoding));
        }
    }
    let mut resolvers: Vec<(&Root, Resolver<'_>)> = Vec::new();
    for (root, document) in index.documents() {
        let Some(from) = document.path() else {
            continue;
        };
        let i = resolvers
            .iter()
            .position(|(r, _)| std::ptr::eq(*r, root))
            .unwrap_or_else(|| {
                resolvers.push((root, Resolver::new(root.workspace())));
                resolvers.len() - 1
            });
        let resolver = &mut resolvers[i].1;
        let source = Some(document.source());
        for link in root.links(from) {
            if resolver
                .destination(document, link)
                .is_some_and(|destination| points_at(from, &destination, &link.target, &wanted))
            {
                found.extend(location(from, source, link.span, encoding));
            }
        }
        for definition in reference_definitions(document) {
            let target = reference_target(resolver, document, &definition);
            if resolver
                .target_destination(document, &target, false)
                .is_some_and(|destination| points_at(from, &destination, &target, &wanted))
            {
                found.extend(location(from, source, definition.span, encoding));
            }
        }
    }
    found.sort_by(|a, b| {
        let key = |l: &Location| (l.uri.as_str().to_owned(), l.range.start, l.range.end);
        key(a).cmp(&key(b))
    });
    found.dedup();
    found
}

/// What references of `destination`, the destination of `target` in the document at
/// `from`, are: those of its anchor if `target` has one, else of its file.
fn wanted(from: &Path, destination: &Destination, target: &Target) -> Option<Wanted> {
    let anchored = has_anchor(target);
    match destination {
        Destination::Here {
            offset: Some(offset),
        } if anchored => Some(Wanted::Anchor {
            path: from.to_owned(),
            offset: *offset,
        }),
        Destination::Here { offset: Some(_) } => Some(Wanted::File(from.to_owned())),
        Destination::File {
            path,
            offset: Some(offset),
        } if anchored => Some(Wanted::Anchor {
            path: path.clone(),
            offset: *offset,
        }),
        Destination::File { path, .. } if !anchored => Some(Wanted::File(path.clone())),
        _ => None,
    }
}

/// Whether `destination`, that of `target` in the document at `from`, is `wanted`.
/// A file is only wanted by links from elsewhere (an anchor of the document itself is not
/// a reference to its file).
fn points_at(from: &Path, destination: &Destination, target: &Target, wanted: &Wanted) -> bool {
    match (destination, wanted) {
        (Destination::Here { offset }, Wanted::Anchor { path, offset: at }) => {
            has_anchor(target) && from == path && *offset == Some(*at)
        }
        (
            Destination::File { path, offset },
            Wanted::Anchor {
                path: p,
                offset: at,
            },
        ) => has_anchor(target) && path == p && *offset == Some(*at),
        (Destination::File { path, .. }, Wanted::File(p)) => path == p,
        _ => false,
    }
}

/// Whether `target` names an anchor.
const fn has_anchor(target: &Target) -> bool {
    matches!(
        target,
        Target::Anchor(_)
            | Target::File {
                anchor: Some(_),
                ..
            }
            | Target::WikiLink {
                anchor: Some(_),
                ..
            }
    )
}

/// A preview of the item at `offset` of the document at `path`, in Markdown: for a link
/// or reference definition, its destination as written and up to [`PREVIEW_LINES`] of
/// its target (the anchor's section, or the top of a Markdown file), just the
/// destination for other files and external links; for a footnote reference, the text of
/// its definition. `None` for anything else, and for a missing target.
pub fn hover(
    index: &WorkspaceIndex,
    path: &Path,
    offset: usize,
    encoding: PositionEncoding,
) -> Option<Hover> {
    let (root, document) = indexed(index, path)?;
    let mut resolver = Resolver::new(root.workspace());
    let item = item_at(document, root.links(path), offset)?;
    let span = item.span();
    let value = match item {
        Item::Link(link) => {
            let written = written_destination(document, &link);
            let wiki = matches!(link.kind, NodeType::WikiLink | NodeType::Embed);
            preview(&mut resolver, document, &link.target, wiki, &written)?
        }
        Item::ReferenceDefinition(definition) => {
            let target = reference_target(&resolver, document, &definition);
            preview(&mut resolver, document, &target, false, &definition.url)?
        }
        Item::FootnoteReference { label, .. } => {
            let definition = footnote_definition(document, &label)?;
            footnote_text(document, definition)?
        }
        _ => return None,
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(convert::range(document.source(), span, encoding)),
    })
}

/// The hover text for `target` of `document`, written as `written`.
fn preview(
    resolver: &mut Resolver<'_>,
    document: &Document,
    target: &Target,
    wiki: bool,
    written: &str,
) -> Option<String> {
    let header = code(written);
    if matches!(target, Target::External(_) | Target::Mailto(_)) {
        return Some(header);
    }
    let destination = resolver.target_destination(document, target, wiki)?;
    let anchored = has_anchor(target);
    let excerpt = match &destination {
        Destination::Here {
            offset: Some(offset),
        } => excerpt(document, *offset, anchored),
        Destination::Here { offset: None } => return None,
        Destination::File {
            path,
            offset: Some(offset),
        } => resolver
            .markdown(path)
            .map(|target| excerpt(target, *offset, anchored))
            .unwrap_or_default(),
        _ => String::new(),
    };
    Some(if excerpt.is_empty() {
        header
    } else {
        format!("{header}\n\n{excerpt}")
    })
}

/// Up to [`PREVIEW_LINES`] of `document` from `offset`: to the next heading if
/// `anchored` (the section), else from after the frontmatter.
fn excerpt(document: &Document, offset: usize, anchored: bool) -> String {
    let text = document.text();
    let (start, end) = if anchored {
        let next = document
            .toc()
            .iter()
            .map(|entry| entry.span.start)
            .find(|&start| start > offset);
        (offset, next.unwrap_or(text.len()))
    } else {
        let body = document.frontmatter().map_or(0, Frontmatter::body_start);
        (offset.max(body), text.len())
    };
    let lines: Vec<&str> = text[start..end].lines().take(PREVIEW_LINES).collect();
    lines.join("\n").trim().to_owned()
}

/// `text` as a Markdown code span, with a fence longer than any backtick run in it.
fn code(text: &str) -> String {
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

/// A link's destination as written: the URL of a link or image, `name#anchor` of a
/// wikilink or embed.
fn written_destination(document: &Document, link: &DocumentLink) -> String {
    match document.ast().kind(link.node) {
        NodeKind::Link(link) | NodeKind::Image(link) => link.url.clone(),
        NodeKind::WikiLink(link) | NodeKind::Embed(link) => match &link.anchor {
            Some(anchor) => format!("{}#{anchor}", link.target),
            None => link.target.clone(),
        },
        _ => String::new(),
    }
}

/// The root and the indexed document at `path`.
pub(crate) fn indexed<'i>(
    index: &'i WorkspaceIndex,
    path: &Path,
) -> Option<(&'i Root, &'i Document)> {
    let root = index.root_of(path)?;
    Some((root, root.workspace().get(path)?))
}

/// The first reference definition of `document` with `label`.
pub fn reference_definition(document: &Document, label: &str) -> Option<ReferenceDefinition> {
    let label = normalize_label(label);
    reference_definitions(document)
        .into_iter()
        .find(|definition| normalize_label(&definition.label) == label)
}

/// The target of a reference definition of `document`, resolved.
fn reference_target(
    resolver: &Resolver<'_>,
    document: &Document,
    definition: &ReferenceDefinition,
) -> Target {
    Target::parse(&definition.url).resolve(resolver.workspace(), document.path())
}

/// The span of the first footnote definition of `document` with `label`.
pub fn footnote_definition(document: &Document, label: &str) -> Option<Span> {
    let label = normalize_label(label);
    let ast = document.ast();
    ast.descendants(ast.root())
        .find_map(|node| match ast.kind(node) {
            NodeKind::FootnoteDefinition(footnote) if normalize_label(&footnote.label) == label => {
                Some(ast.span(node))
            }
            _ => None,
        })
}

/// The text of the footnote definition at `span` of `document`, without its label.
pub(crate) fn footnote_text(document: &Document, span: Span) -> Option<String> {
    let ast = document.ast();
    let node = ast.descendants(ast.root()).find(|&node| {
        ast.span(node) == span && matches!(ast.kind(node), NodeKind::FootnoteDefinition(_))
    })?;
    let first = *ast.children(node).first()?;
    let text = document.text()[ast.span(first).start..span.end].trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The location of `destination`, of a link of `document`: the line its anchor points at,
/// else the top of the file. `None` for a missing anchor of the document itself, and for
/// a directory.
fn destination_location(
    resolver: &mut Resolver<'_>,
    document: &Document,
    destination: &Destination,
    encoding: PositionEncoding,
) -> Option<Location> {
    match destination {
        Destination::Here { offset } => {
            let line = line_span(document.text(), (*offset)?);
            location(document.path()?, Some(document.source()), line, encoding)
        }
        Destination::File { path, .. } if path.is_dir() => None,
        Destination::File { path, offset } => {
            let source = resolver.markdown(path).map(Document::source);
            let line = match (source, offset) {
                (Some(source), Some(offset)) => line_span(source.text(), *offset),
                _ => Span::new(0, 0),
            };
            location(path, source, line, encoding)
        }
        _ => None,
    }
}

/// The span of the line of `text` that starts at `offset`, without its line ending.
fn line_span(text: &str, offset: usize) -> Span {
    let offset = offset.min(text.len());
    let end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    let end = if text[..end].ends_with('\r') {
        end - 1
    } else {
        end
    };
    Span::new(offset, end.max(offset))
}

/// The location of `span` in the file at `path` with `source` (the top if `None`).
fn location(
    path: &Path,
    source: Option<&SourceFile>,
    span: Span,
    encoding: PositionEncoding,
) -> Option<Location> {
    let range = source.map_or_else(Range::default, |source| {
        convert::range(source, span, encoding)
    });
    Some(Location::new(path_to_uri(path)?, range))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_spans_outfence_backticks() {
        assert_eq!(code("a.md"), "`a.md`");
        assert_eq!(code("a`b"), "``a`b``");
        assert_eq!(code("`a``"), "``` `a`` ```");
    }

    #[test]
    fn line_spans_stop_before_the_line_ending() {
        let text = "ab\r\ncd\nef";
        assert_eq!(line_span(text, 0), Span::new(0, 2));
        assert_eq!(line_span(text, 4), Span::new(4, 6));
        assert_eq!(line_span(text, 7), Span::new(7, 9));
        assert_eq!(line_span(text, 99), Span::new(9, 9));
    }

    #[test]
    fn links_win_over_what_contains_them() {
        let document = Document::parse("# A [x](#a) [^n]\n\n[^n]: B [y](#a).\n\n[r]: #a\n");
        let workspace = mrq_core::Workspace::new(".");
        let links = mrq_core::validate::links(&document, &workspace);
        let kind = |offset| match item_at(&document, &links, offset) {
            Some(Item::Link(_)) => "link",
            Some(Item::FootnoteReference { .. }) => "footnote reference",
            Some(Item::FootnoteDefinition { .. }) => "footnote definition",
            Some(Item::ReferenceDefinition(_)) => "reference definition",
            Some(Item::Heading(_)) => "heading",
            None => "none",
        };
        let at = |text: &str| document.text().find(text).unwrap();
        assert_eq!(kind(0), "heading");
        assert_eq!(kind(at("[x]")), "link");
        assert_eq!(kind(at("[^n]")), "footnote reference");
        assert_eq!(kind(at("[^n]:")), "footnote definition");
        assert_eq!(kind(at("[y]")), "link");
        assert_eq!(kind(at("[r]")), "reference definition");
        assert_eq!(kind(document.text().len()), "none");
    }
}
