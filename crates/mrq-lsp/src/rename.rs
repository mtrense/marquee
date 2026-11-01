//! Rename: of a heading ([`heading_at`], [`rename_heading`]) with every link to its
//! anchor, and of files and directories ([`rename_files`], for
//! `workspace/willRenameFiles`) with every link to them and in them. Both return
//! [`Edits`], which [`workspace_edit`] makes a `WorkspaceEdit` for the client.
//!
//! Links are found as [`references`](crate::navigate::references) finds them: inline
//! links and images, wikilinks and embeds, and the reference definitions that reference
//! links use (the definition's URL is edited, never `[text][label]`), in every indexed
//! document, resolved as `mrq validate` resolves them.
//!
//! A heading is renamed by replacing its text (between the `#`s, without a `{#id}`
//! attribute); the document is parsed again with the new text to learn the new anchors
//! from the flavor's slugger. Every link to an anchor of the document that changes is
//! updated: the renamed heading's, and those of duplicates whose `-1` suffix shifts.
//! A heading with an explicit `{#id}` keeps its anchor, so only wikilinks, which name
//! headings by text, change.
//!
//! A file or directory rename rewrites the links to it (and, for a directory, to
//! anything under it) as [`rewrite_links`] does, and rebases the relative links of the
//! documents that move. Written style is kept: a `./` prefix, a path from the root
//! (`/…`), the query string and fragment, and links that still work from the new place
//! are left alone. Wikilinks to a moved file get its new name, with as many path
//! segments as they had.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::ast::{Heading, NodeKind, NodeType, Span};
use mrq_core::document::normalize_path;
use mrq_core::source::{PositionEncoding, TextEdit};
use mrq_core::toc::{TocEntry, encode_destination};
use mrq_core::transform::{LinkMap, LinkRewrite, Location, markdown_destinations, rewrite_links};
use mrq_core::validate::{Destination, DocumentLink, Target, reference_definitions};
use tower_lsp_server::ls_types::{
    self, DocumentChanges, FileOperationFilter, FileOperationPattern, FileOperationPatternKind,
    FileOperationRegistrationOptions, OneOf, OptionalVersionedTextDocumentIdentifier,
    TextDocumentEdit, WorkspaceEdit,
};

use crate::convert::{self, path_to_uri};
use crate::index::{Root, WorkspaceIndex};
use crate::navigate::{Item, indexed, item_at, reference_target, resolver_for};
use crate::store::DocumentStore;

/// Text edits by the path of the document they apply to, in its indexed text.
pub type Edits = BTreeMap<PathBuf, Vec<TextEdit>>;

/// The files `workspace/willRenameFiles` and `didRenameFiles` are wanted for: any file
/// (links point at images too) and any folder.
pub fn file_operations() -> FileOperationRegistrationOptions {
    let filter = |matches| FileOperationFilter {
        scheme: Some("file".into()),
        pattern: FileOperationPattern {
            glob: "**/*".into(),
            matches: Some(matches),
            options: None,
        },
    };
    FileOperationRegistrationOptions {
        filters: vec![
            filter(FileOperationPatternKind::File),
            filter(FileOperationPatternKind::Folder),
        ],
    }
}

/// The text of the heading at `offset` of the document at `path` (what a rename
/// replaces): its span and the text as written. `None` elsewhere, and for an empty
/// heading.
pub fn heading_at(index: &WorkspaceIndex, path: &Path, offset: usize) -> Option<(Span, String)> {
    let (root, document) = indexed(index, path)?;
    let Item::Heading(entry) = item_at(document, root.links(path), offset)? else {
        return None;
    };
    let span = heading_text(document, &entry)?;
    Some((span, document.text()[span.start..span.end].to_owned()))
}

/// The edits that give the heading at `offset` of the document at `path` the text
/// `new_name` and update the links to the anchors that change. `None` if there is no
/// heading there; an error message for a name that is empty or not one line.
pub fn rename_heading(
    index: &WorkspaceIndex,
    path: &Path,
    offset: usize,
    new_name: &str,
) -> Option<Result<Edits, String>> {
    let (span, _) = heading_at(index, path, offset)?;
    let name = new_name.trim();
    if name.is_empty() || name.contains(['\n', '\r']) {
        return Some(Err(
            "a heading's new text must be one line, not empty".into()
        ));
    }
    let document = index.document(path)?;
    let text = document.text();
    let renamed = document.with_text(format!(
        "{}{name}{}",
        &text[..span.start],
        &text[span.end..]
    ));
    let (old, new) = (document.toc(), renamed.toc());
    if old.len() != new.len() {
        return Some(Err(format!("`{name}` does not keep the heading a heading")));
    }
    let i = old
        .iter()
        .position(|entry| entry.span.start <= span.start && span.end <= entry.span.end)?;
    let rename = HeadingRename {
        heading: old[i].span.start,
        text: new[i].text.clone(),
        anchors: old
            .iter()
            .zip(&new)
            .filter(|(old, new)| old.slug != new.slug)
            .map(|(old, new)| (old.span.start, new.slug.clone()))
            .collect(),
    };
    let mut edits = Edits::new();
    edits.entry(path.to_owned()).or_default().push(TextEdit {
        range: span,
        replacement: name.to_owned(),
    });
    let mut resolvers = Vec::new();
    for (root, document) in index.documents() {
        let Some(from) = document.path() else {
            continue;
        };
        let resolver = resolver_for(&mut resolvers, root);
        let destinations = markdown_destinations(document);
        let mut found = Vec::new();
        for link in root.links(from) {
            // A reference link's definition is edited instead.
            if link.reference.is_some() {
                continue;
            }
            let Some(at) = resolver
                .destination(document, link)
                .and_then(|destination| anchor_of(from, &destination, &link.target, path))
            else {
                continue;
            };
            if matches!(link.kind, NodeType::WikiLink | NodeType::Embed) {
                if at == rename.heading
                    && let Some(range) = wikilink_heading(document, link)
                {
                    found.push(TextEdit {
                        range,
                        replacement: rename.text.clone(),
                    });
                }
            } else {
                found.extend(rename.fragment_edit(document, &destinations, link.span, at));
            }
        }
        for definition in reference_definitions(document) {
            let target = reference_target(resolver, document, &definition);
            if let Some(at) = resolver
                .target_destination(document, &target, false)
                .and_then(|destination| anchor_of(from, &destination, &target, path))
            {
                found.extend(rename.fragment_edit(document, &destinations, definition.span, at));
            }
        }
        // A link in the heading goes with its text.
        if from == path {
            found.retain(|edit| edit.range.end <= span.start || edit.range.start >= span.end);
        }
        if !found.is_empty() {
            edits.entry(from.to_owned()).or_default().extend(found);
        }
    }
    for edits in edits.values_mut() {
        edits.sort_by_key(|edit| edit.range.start);
    }
    Some(Ok(edits))
}

/// What renaming a heading changes.
struct HeadingRename {
    /// Where the renamed heading starts.
    heading: usize,
    /// The renamed heading's new plain text, which wikilinks name it by.
    text: String,
    /// The new anchor of each heading whose anchor changes, by where it starts.
    anchors: HashMap<usize, String>,
}

impl HeadingRename {
    /// The edit of the fragment of the Markdown destination in `span` of `document`,
    /// which points at the heading starting at `at` of the renamed heading's document,
    /// if that heading's anchor changes.
    fn fragment_edit(
        &self,
        document: &Document,
        destinations: &[Span],
        span: Span,
        at: usize,
    ) -> Option<TextEdit> {
        let anchor = self.anchors.get(&at)?;
        // The last destination in the span: a link's own comes after its text's.
        let destination = destinations
            .iter()
            .rev()
            .find(|d| span.start <= d.start && d.end <= span.end)?;
        let written = &document.text()[destination.start..destination.end];
        let hash = destination.start + written.find('#')? + 1;
        Some(TextEdit {
            range: Span::new(hash, destination.end),
            replacement: encode_destination(anchor),
        })
    }
}

/// Where in the document at `path` the (resolved) `target`'s `destination` points,
/// for a link of the document at `from`, if it names an anchor there.
fn anchor_of(
    from: &Path,
    destination: &Destination,
    target: &Target,
    path: &Path,
) -> Option<usize> {
    let anchored = matches!(
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
    );
    match destination {
        Destination::Here { offset } if anchored && from == path => *offset,
        Destination::File { path: to, offset } if anchored && to == path => *offset,
        _ => None,
    }
}

/// The span of the text of the heading `entry` of `document`: from its first child to
/// its last, before any `{#id}` attribute, without surrounding whitespace. `None` for a
/// heading without text.
fn heading_text(document: &Document, entry: &TocEntry) -> Option<Span> {
    let ast = document.ast();
    let children = ast.children(entry.node);
    let start = ast.span(*children.first()?).start;
    let mut end = ast.span(*children.last()?).end;
    if let NodeKind::Heading(Heading {
        attrs: Some(attrs), ..
    }) = ast.kind(entry.node)
        && attrs.span.start >= start
    {
        end = end.min(attrs.span.start);
    }
    let text = &document.text()[start..end];
    let trimmed = text.trim();
    let start = start + (text.len() - text.trim_start().len());
    (!trimmed.is_empty()).then(|| Span::new(start, start + trimmed.len()))
}

/// The span of the heading a wikilink or embed names after its `#` (the last part of
/// `Parent#Child`), as written.
fn wikilink_heading(document: &Document, link: &DocumentLink) -> Option<Span> {
    let (NodeKind::WikiLink(wiki) | NodeKind::Embed(wiki)) = document.ast().kind(link.node) else {
        return None;
    };
    let anchor = wiki.anchor.as_deref()?;
    let target = wikilink_target(document, link)?;
    let start = target.end + 1;
    let text = document.text();
    if !text[target.end..].starts_with('#') || !text[start..].starts_with(anchor) {
        return None;
    }
    let last = anchor.rfind('#').map_or(0, |i| i + 1);
    Some(Span::new(start + last, start + anchor.len()))
}

/// The span of the name of a wikilink or embed, as written (empty for `[[#heading]]`).
fn wikilink_target(document: &Document, link: &DocumentLink) -> Option<Span> {
    let (NodeKind::WikiLink(wiki) | NodeKind::Embed(wiki)) = document.ast().kind(link.node) else {
        return None;
    };
    let text = document.text();
    let open = link.span.start
        + if text[link.span.start..].starts_with('!') {
            3
        } else {
            2
        };
    text[open..]
        .starts_with(&wiki.target)
        .then(|| Span::new(open, open + wiki.target.len()))
}

/// The edits that keep the links working when each file or directory of `renames` (old
/// path, new path) moves: the links to it (or to anything under a directory) from every
/// indexed document, and the relative links of the documents that move.
pub fn rename_files(index: &WorkspaceIndex, renames: &[(PathBuf, PathBuf)]) -> Edits {
    // Every path that moves, a directory's contents included.
    let mut paths: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (old, new) in renames {
        let (old, new) = (normalize_path(old), normalize_path(new));
        let mut under = Vec::new();
        if old.is_dir() {
            walk(&old, &mut under);
        }
        // Documents that are open and not saved yet are not on disk.
        under.extend(
            index
                .documents()
                .filter_map(|(_, document)| document.path())
                .filter(|path| path.starts_with(&old) && *path != old)
                .map(Path::to_path_buf),
        );
        under.sort();
        under.dedup();
        for path in under {
            let to = new.join(path.strip_prefix(&old).expect("under the old path"));
            paths.push((path, to));
        }
        paths.push((old, new));
    }
    let mut map = LinkMap::new();
    for (old, new) in &paths {
        map.insert_file(old, Location::file(new));
    }
    let moved = |path: &Path| {
        paths
            .iter()
            .find(|(old, _)| old == path)
            .map(|(_, new)| new)
    };
    let mut edits = Edits::new();
    let mut resolvers = Vec::new();
    for (root, document) in index.documents() {
        let Some(from) = document.path() else {
            continue;
        };
        let to = moved(from).map_or(from, PathBuf::as_path);
        let rewrite = LinkRewrite::new(from, to).root(root.path()).map(&map);
        let links = root.links(from);
        let wiki = |link: &&DocumentLink| matches!(link.kind, NodeType::WikiLink | NodeType::Embed);
        let in_wikilink = |range: Span| {
            links
                .iter()
                .filter(wiki)
                .any(|link| link.span.start <= range.start && range.end <= link.span.end)
        };
        let text = document.text();
        let mut found = Vec::new();
        for edit in rewrite_links(document, Span::new(0, text.len()), &rewrite) {
            // Wikilinks are names, looked up below.
            if in_wikilink(edit.range) {
                continue;
            }
            let written = &text[edit.range.start..edit.range.end];
            let replacement = restyle(written, &edit.replacement, to, root);
            if !same_place(written, &replacement, to, root) {
                found.push(TextEdit {
                    range: edit.range,
                    replacement,
                });
            }
        }
        let resolver = resolver_for(&mut resolvers, root);
        for link in links.iter().filter(wiki) {
            let (Target::WikiLink { name, .. }, Some(range)) =
                (&link.target, wikilink_target(document, link))
            else {
                continue;
            };
            let Some(old) = resolver.wikilink(name) else {
                continue;
            };
            if let Some(new) = moved(&old) {
                let replacement = wikilink_name(name, &old, new, root.path());
                if replacement != *name {
                    found.push(TextEdit { range, replacement });
                }
            }
        }
        if !found.is_empty() {
            found.sort_by_key(|edit| edit.range.start);
            edits.insert(from.to_owned(), found);
        }
    }
    edits
}

/// Every file and directory under `dir`, depth first; symbolic links are not followed.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        out.push(path.clone());
        if is_dir {
            walk(&path, out);
        }
    }
}

/// The destination `replacement` that [`rewrite_links`] wrote for `written` in the
/// document that will be at `to`, in `written`'s style: its query string and fragment
/// as written, a path from the root if it had one, `./` if it started with that, a
/// trailing `/`, and the file's name rather than nothing for a link to itself.
fn restyle(written: &str, replacement: &str, to: &Path, root: &Root) -> String {
    fn split(text: &str) -> (&str, &str) {
        text.split_at(text.find(['?', '#']).unwrap_or(text.len()))
    }
    let (path, suffix) = split(written);
    let (new_path, _) = split(replacement);
    if path.is_empty() {
        return replacement.to_owned();
    }
    let mut out = if new_path.is_empty() {
        to.file_name()
            .map(|name| encode_destination(&name.to_string_lossy()))
            .unwrap_or_default()
    } else {
        new_path.to_owned()
    };
    if path.starts_with('/') {
        if let Target::File { path: target, .. } =
            Target::parse(&out).resolve(root.workspace(), Some(to))
            && let Ok(relative) = target.strip_prefix(root.path())
        {
            let segments: Vec<String> = relative
                .components()
                .map(|segment| encode_destination(&segment.as_os_str().to_string_lossy()))
                .collect();
            out = format!("/{}", segments.join("/"));
        }
    } else if path.starts_with("./") && !out.starts_with("../") && !out.starts_with("./") {
        out.insert_str(0, "./");
    }
    if path.ends_with('/') && !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(suffix);
    out
}

/// Whether the destinations `written` and `replacement` lead to the same place from
/// the document that will be at `to`.
fn same_place(written: &str, replacement: &str, to: &Path, root: &Root) -> bool {
    let resolve = |text: &str| Target::parse(text).resolve(root.workspace(), Some(to));
    written == replacement || resolve(written) == resolve(replacement)
}

/// The wikilink name `written`, which finds the file at `old`, made to find it at
/// `new`: the same number of trailing path segments of its path from `root`, with the
/// extension only if `written` has it, and `./` kept.
fn wikilink_name(written: &str, old: &Path, new: &Path, root: &Path) -> String {
    let (dot, name) = written
        .strip_prefix("./")
        .map_or(("", written), |rest| ("./", rest));
    let relative = new.strip_prefix(root).unwrap_or(new);
    let segments: Vec<String> = relative
        .components()
        .map(|segment| segment.as_os_str().to_string_lossy().into_owned())
        .collect();
    let count = name.split('/').count().min(segments.len());
    let mut segments = segments[segments.len() - count..].to_vec();
    let extension = old
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()));
    let with_extension = extension.is_some_and(|ext| name.to_lowercase().ends_with(&ext));
    if !with_extension
        && let Some(last) = segments.last_mut()
        && let Some(stem) = Path::new(last.as_str()).file_stem()
    {
        *last = stem.to_string_lossy().into_owned();
    }
    format!("{dot}{}", segments.join("/"))
}

/// `edits` for the client: positions in `encoding` in each document's indexed text,
/// as versioned `documentChanges` (the open version, `null` for files) if `versioned`,
/// else as `changes`. `None` if there is nothing to change.
pub fn workspace_edit(
    index: &WorkspaceIndex,
    store: &DocumentStore,
    edits: Edits,
    encoding: PositionEncoding,
    versioned: bool,
) -> Option<WorkspaceEdit> {
    let mut changes = HashMap::new();
    let mut document_changes = Vec::new();
    for (path, edits) in edits {
        let (Some(document), Some(uri)) = (index.document(&path), path_to_uri(&path)) else {
            continue;
        };
        let edits: Vec<ls_types::TextEdit> = edits
            .into_iter()
            .map(|edit| {
                let range = convert::range(document.source(), edit.range, encoding);
                ls_types::TextEdit::new(range, edit.replacement)
            })
            .collect();
        if versioned {
            document_changes.push(TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri,
                    version: store.get_path(&path).map(|open| open.version),
                },
                edits: edits.into_iter().map(OneOf::Left).collect(),
            });
        } else {
            changes.insert(uri, edits);
        }
    }
    if changes.is_empty() && document_changes.is_empty() {
        return None;
    }
    Some(if versioned {
        WorkspaceEdit {
            document_changes: Some(DocumentChanges::Edits(document_changes)),
            ..WorkspaceEdit::default()
        }
    } else {
        WorkspaceEdit {
            changes: Some(changes),
            ..WorkspaceEdit::default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wikilink_names_keep_their_shape() {
        let root = Path::new("/v");
        let name = |written, old: &str, new: &str| {
            wikilink_name(written, Path::new(old), Path::new(new), root)
        };
        assert_eq!(name("b", "/v/n/b.md", "/v/n/c.md"), "c");
        assert_eq!(name("B.md", "/v/n/b.md", "/v/m/c.md"), "c.md");
        assert_eq!(name("n/b", "/v/n/b.md", "/v/m/b.md"), "m/b");
        assert_eq!(name("./b", "/v/b.md", "/v/x/c.md"), "./c");
        assert_eq!(name("a/b/c", "/v/b.md", "/v/d.md"), "d");
        assert_eq!(name("pic.png", "/v/pic.png", "/v/i/photo.png"), "photo.png");
    }
}
