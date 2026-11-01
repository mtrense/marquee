//! Completion: what is being typed at a position ([`context`]), and the answer to
//! `textDocument/completion` ([`completions`]).
//!
//! The context is read from the text before the cursor on its line, not from a parsed
//! link, since what is being typed rarely parses as one yet: `[text](dir/na` (a path),
//! `](#an` and `](file.md#an` (an anchor), `[^la` (a footnote label), `[text][la` (a
//! reference label), `[[na` and `[[note#He` (a wikilink target and heading). Each item
//! replaces the typed part of the token, from its start to the cursor.
//!
//! Paths and anchors resolve as `mrq validate` resolves them (see
//! [`navigate`](crate::navigate)), so what completes is what validates.

use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::ast::{NodeKind, Span};
use mrq_core::input::has_markdown_extension;
use mrq_core::source::PositionEncoding;
use mrq_core::toc::encode_destination;
use mrq_core::validate::{Resolver, Target, normalize_label, reference_definitions};
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Range, TextEdit,
};

use crate::convert;
use crate::index::{Root, WorkspaceIndex};
use crate::navigate::{footnote_text, indexed};

/// The characters that ask the client for completion: they open a destination, an
/// anchor, a directory, a footnote label, a reference label or a wikilink.
pub const TRIGGER_CHARACTERS: [&str; 5] = ["(", "#", "/", "^", "["];

/// What is being typed at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// The path of an inline link or image destination; `dir` is what is typed of it up
    /// to and including the last `/` (empty for the document's directory).
    Path { dir: String },
    /// The anchor of a destination; `file` is the path before the `#`, empty for the
    /// document itself.
    Anchor { file: String },
    /// A footnote reference's label.
    Footnote,
    /// A reference link's label.
    Reference,
    /// A wikilink's (or embed's) target.
    WikiLink,
    /// A heading of the wikilink target `name` (empty for the document itself).
    WikiHeading { name: String },
}

/// The tokens that start a context, by the text that opens them.
const OPENERS: [&str; 4] = ["](", "[[", "[^", "]["];

/// What is being typed at byte `offset` of `text`, and the offset where the typed part
/// of its token starts (what an item replaces). `None` outside every context.
///
/// ```
/// use mrq_lsp::complete::{Context, context};
///
/// let text = "See [x](docs/re";
/// let dir = "docs/".to_owned();
/// assert_eq!(context(text, text.len()), Some((Context::Path { dir }, 13)));
/// assert_eq!(context("[x](a.md) b", 11), None);
/// ```
pub fn context(text: &str, offset: usize) -> Option<(Context, usize)> {
    let line = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line..offset];
    // The opener closest to the cursor (no two of them can end at the same place).
    let (opener, end) = OPENERS
        .iter()
        .filter_map(|&opener| Some((opener, before.rfind(opener)? + opener.len())))
        .max_by_key(|&(_, end)| end)?;
    let typed = &before[end..];
    let start = line + end;
    let found = match opener {
        "](" => {
            let invalid = |c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>');
            if typed.contains(invalid) {
                return None;
            }
            match typed.split_once('#') {
                Some((file, _)) if !file.contains(':') => {
                    let file = file.to_owned();
                    let start = start + file.len() + 1;
                    (Context::Anchor { file }, start)
                }
                None if !typed.contains(':') => {
                    let dir = &typed[..typed.rfind('/').map_or(0, |i| i + 1)];
                    let start = start + dir.len();
                    let dir = dir.to_owned();
                    (Context::Path { dir }, start)
                }
                _ => return None,
            }
        }
        "[[" => {
            if typed.contains([']', '|']) {
                return None;
            }
            match typed.split_once('#') {
                Some((name, _)) => {
                    let name = name.to_owned();
                    let start = start + name.len() + 1;
                    (Context::WikiHeading { name }, start)
                }
                None => (Context::WikiLink, start),
            }
        }
        "[^" if !typed.contains(|c: char| c == ']' || c.is_whitespace()) => {
            (Context::Footnote, start)
        }
        "][" if !typed.contains(']') => (Context::Reference, start),
        _ => return None,
    };
    Some(found)
}

/// A completion before it is placed: what it shows and what it inserts.
struct Candidate {
    label: String,
    kind: CompletionItemKind,
    detail: Option<String>,
    insert: String,
}

impl Candidate {
    fn new(label: String, kind: CompletionItemKind, detail: Option<String>) -> Self {
        Self {
            insert: label.clone(),
            label,
            kind,
            detail,
        }
    }

    /// The item replacing `range`, sorted as the `rank`th.
    fn item(self, range: Range, rank: usize) -> CompletionItem {
        CompletionItem {
            label: self.label,
            kind: Some(self.kind),
            detail: self.detail,
            sort_text: Some(format!("{rank:05}")),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, self.insert))),
            ..CompletionItem::default()
        }
    }
}

/// The completions at byte `offset` of the indexed document at `path` (see
/// [`context`]), in the order to show them; `None` outside every context and in code.
///
/// - Paths: the entries of the directory typed so far (relative to the document, or the
///   root for `/`), without hidden ones: Markdown files with their first heading, then
///   directories (inserted with a `/`), then other files.
/// - Anchors: those of the document, or of the Markdown file before the `#`, with the
///   heading text.
/// - Footnote and reference labels: the document's definitions, with the footnote's
///   text or the destination.
/// - Wikilinks: the other documents of the root, by the shortest name that finds them,
///   with their path; after a `#`, the headings of the target by text.
pub fn completions(
    index: &WorkspaceIndex,
    path: &Path,
    offset: usize,
    encoding: PositionEncoding,
) -> Option<Vec<CompletionItem>> {
    let (root, document) = indexed(index, path)?;
    if in_code(document, offset) {
        return None;
    }
    let (context, start) = context(document.text(), offset)?;
    let mut resolver = Resolver::new(root.workspace());
    let candidates = match context {
        Context::Path { dir } => paths(index, root, path, &dir),
        Context::Anchor { file } => {
            let target = if file.is_empty() {
                Some(document)
            } else {
                match Target::parse(&file).resolve(root.workspace(), Some(path)) {
                    Target::File { path, .. } => resolver.markdown(&path),
                    _ => None,
                }
            };
            target.map(anchors).unwrap_or_default()
        }
        Context::Footnote => footnotes(document),
        Context::Reference => references(document),
        Context::WikiLink => wikilinks(&mut resolver, root, path),
        Context::WikiHeading { name } => {
            let target = if name.is_empty() {
                Some(path.to_owned())
            } else {
                resolver.wikilink(&name)
            };
            target
                .and_then(|target| resolver.markdown(&target))
                .map(headings)
                .unwrap_or_default()
        }
    };
    let range = convert::range(document.source(), Span::new(start, offset), encoding);
    let items = candidates
        .into_iter()
        .enumerate()
        .map(|(rank, candidate)| candidate.item(range, rank))
        .collect();
    Some(items)
}

/// Whether `offset` is inside a code span or code block of `document`.
fn in_code(document: &Document, offset: usize) -> bool {
    let ast = document.ast();
    ast.descendants(ast.root()).any(|node| {
        let span = ast.span(node);
        span.start < offset
            && offset < span.end
            && matches!(ast.kind(node), NodeKind::Code(_) | NodeKind::CodeBlock(_))
    })
}

/// The entries of the directory `dir` (as typed in a link of the document at `from`).
fn paths(index: &WorkspaceIndex, root: &Root, from: &Path, dir: &str) -> Vec<Candidate> {
    let base = match Target::parse(dir) {
        Target::Empty => from.parent().map(Path::to_path_buf),
        Target::File { path, .. } => Some(root.workspace().resolve(Some(from), path)),
        _ => None,
    };
    let Some(entries) = base.and_then(|base| std::fs::read_dir(base).ok()) else {
        return Vec::new();
    };
    let mut entries: Vec<(String, PathBuf, bool)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let is_dir = entry.file_type().ok()?.is_dir();
            (!name.starts_with('.')).then(|| (name, entry.path(), is_dir))
        })
        .collect();
    let group = |(_, path, is_dir): &(String, PathBuf, bool)| match (is_dir, path) {
        (false, path) if has_markdown_extension(path) => 0,
        (true, _) => 1,
        _ => 2,
    };
    entries.sort_by(|a, b| (group(a), &a.0).cmp(&(group(b), &b.0)));
    entries
        .into_iter()
        .map(|(name, path, is_dir)| {
            let mut candidate = if is_dir {
                Candidate::new(format!("{name}/"), CompletionItemKind::FOLDER, None)
            } else {
                let title = index
                    .document(&path)
                    .and_then(|document| document.toc().into_iter().next())
                    .map(|entry| entry.text);
                Candidate::new(name, CompletionItemKind::FILE, title)
            };
            candidate.insert = encode_destination(&candidate.label);
            candidate
        })
        .collect()
}

/// The anchors of `document`, in document order, with their heading's text.
fn anchors(document: &Document) -> Vec<Candidate> {
    let ast = document.ast();
    document
        .anchors()
        .iter()
        .map(|(node, anchor)| {
            let text = ast.text(node);
            let detail = (!text.is_empty()).then_some(text);
            let mut candidate =
                Candidate::new(anchor.to_owned(), CompletionItemKind::REFERENCE, detail);
            candidate.insert = encode_destination(anchor);
            candidate
        })
        .collect()
}

/// The headings of `document` by text, as wikilinks name them.
fn headings(document: &Document) -> Vec<Candidate> {
    document
        .toc()
        .into_iter()
        .map(|entry| {
            let detail = format!("{} {}", "#".repeat(entry.level.into()), entry.text);
            Candidate::new(entry.text, CompletionItemKind::REFERENCE, Some(detail))
        })
        .collect()
}

/// The footnote labels of `document`, with their text; the first of each label.
fn footnotes(document: &Document) -> Vec<Candidate> {
    let ast = document.ast();
    let mut seen = Vec::new();
    ast.descendants(ast.root())
        .filter_map(|node| match ast.kind(node) {
            NodeKind::FootnoteDefinition(footnote) => Some((footnote.label.clone(), node)),
            _ => None,
        })
        .filter(|(label, _)| first(&mut seen, label))
        .map(|(label, node)| {
            let detail = footnote_text(document, ast.span(node));
            Candidate::new(label, CompletionItemKind::REFERENCE, detail)
        })
        .collect()
}

/// The reference definition labels of `document`, with their destination; the first of
/// each label.
fn references(document: &Document) -> Vec<Candidate> {
    let mut seen = Vec::new();
    reference_definitions(document)
        .into_iter()
        .filter(|definition| first(&mut seen, &definition.label))
        .map(|definition| {
            let detail = Some(definition.url);
            Candidate::new(definition.label, CompletionItemKind::REFERENCE, detail)
        })
        .collect()
}

/// Whether `label` is not in `seen` (as labels match), adding it.
fn first(seen: &mut Vec<String>, label: &str) -> bool {
    let label = normalize_label(label);
    let new = !seen.contains(&label);
    if new {
        seen.push(label);
    }
    new
}

/// The documents of `root` other than the one at `from`, by path, each named as short
/// as the vault still finds it: the file name without `.md`, else the path from the root.
fn wikilinks(resolver: &mut Resolver<'_>, root: &Root, from: &Path) -> Vec<Candidate> {
    let workspace = root.workspace();
    let mut documents: Vec<(String, &Path)> = workspace
        .documents()
        .filter_map(Document::path)
        .filter(|&path| path != from)
        .map(|path| {
            let relative = path.strip_prefix(workspace.root()).unwrap_or(path);
            (relative.to_string_lossy().replace('\\', "/"), path)
        })
        .collect();
    documents.sort();
    documents
        .into_iter()
        .map(|(relative, path)| {
            let name = relative.strip_suffix(".md").unwrap_or(&relative);
            let short = name.rsplit('/').next().unwrap_or(name);
            let name = if resolver.wikilink(short).as_deref() == Some(path) {
                short
            } else {
                name
            };
            Candidate::new(
                name.to_owned(),
                CompletionItemKind::FILE,
                Some(relative.clone()),
            )
        })
        .collect()
}
