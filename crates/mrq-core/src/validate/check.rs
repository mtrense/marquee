//! Internal checks: [`Checker`] and the diagnostic [`codes`].

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

#[cfg(feature = "frontmatter-schema")]
use super::FrontmatterSchemas;
use super::brackets::{Unresolved, unresolved};
use super::{DocumentLink, Target, links, reference_definitions};
use crate::ast::{NodeKind, NodeType, Span};
use crate::diag::{Diagnostic, Label, Severity};
use crate::flavor::obsidian_anchor;
use crate::input::has_markdown_extension;
use crate::{Document, Workspace};

/// The diagnostic codes of the internal checks; see the [module docs](super) for what
/// each one means and its severity.
pub mod codes {
    /// A link to a local file that does not exist.
    pub const MISSING_FILE: &str = "missing-file";
    /// An image whose local file does not exist.
    pub const MISSING_ASSET: &str = "missing-asset";
    /// A `#fragment` (or wikilink heading or block id) that the target does not have.
    pub const MISSING_ANCHOR: &str = "missing-anchor";
    /// A heading anchor that an earlier heading of the same document already has.
    pub const DUPLICATE_ANCHOR: &str = "duplicate-anchor";
    /// A footnote reference `[^label]` without a definition.
    pub const UNDEFINED_FOOTNOTE: &str = "undefined-footnote";
    /// A footnote definition that nothing references.
    pub const UNUSED_FOOTNOTE: &str = "unused-footnote";
    /// A full or collapsed reference link whose label has no definition.
    pub const UNDEFINED_REFERENCE: &str = "undefined-reference";
    /// A link reference definition that no link uses.
    pub const UNUSED_REFERENCE: &str = "unused-reference";
    /// A wikilink or embed whose name matches no file in the vault.
    pub const UNRESOLVED_WIKILINK: &str = "unresolved-wikilink";
    /// Frontmatter that its JSON Schema (`validate.frontmatter`) rejects.
    pub const FRONTMATTER_SCHEMA: &str = "frontmatter-schema";
}

/// Runs the internal checks over the documents of a [`Workspace`].
///
/// Link targets are looked up in the workspace first, then on disk: a Markdown file
/// outside the workspace is loaded (with the workspace's options) and cached when a link
/// needs its anchors. The vault for wikilinks (every file under the workspace root,
/// honouring ignore files and skipping hidden ones, plus the workspace's documents) is
/// listed on first use. Reuse one checker for many documents to share both caches.
///
/// ```
/// use mrq_core::{Document, Workspace};
/// use mrq_core::validate::{Checker, codes};
///
/// let ws = Workspace::new(".");
/// let doc = Document::parse("# Intro\n\n[ok](#intro) [bad](#outro)\n");
/// let diagnostics = Checker::new(&ws).check(&doc);
/// assert_eq!(diagnostics.len(), 1);
/// assert_eq!(diagnostics[0].code, codes::MISSING_ANCHOR);
/// ```
#[derive(Debug)]
pub struct Checker<'w> {
    workspace: &'w Workspace,
    /// Markdown files outside the workspace by path; `None` if unreadable.
    loaded: HashMap<PathBuf, Option<Document>>,
    /// The vault's files, listed on first use.
    vault: Option<Vec<VaultFile>>,
    #[cfg(feature = "frontmatter-schema")]
    schemas: Option<&'w FrontmatterSchemas>,
}

/// A file a wikilink may name.
#[derive(Debug)]
struct VaultFile {
    path: PathBuf,
    /// The path relative to the root, `/`-separated and lowercased.
    key: String,
}

impl<'w> Checker<'w> {
    /// A checker for documents of `workspace` (or documents linking into it).
    pub fn new(workspace: &'w Workspace) -> Self {
        Self {
            workspace,
            loaded: HashMap::new(),
            vault: None,
            #[cfg(feature = "frontmatter-schema")]
            schemas: None,
        }
    }

    /// Also checks frontmatter against `schemas` (code [`codes::FRONTMATTER_SCHEMA`]).
    #[cfg(feature = "frontmatter-schema")]
    #[must_use]
    pub const fn with_frontmatter_schemas(mut self, schemas: &'w FrontmatterSchemas) -> Self {
        self.schemas = Some(schemas);
        self
    }

    /// The diagnostics for `document`, ordered by position.
    pub fn check(&mut self, document: &Document) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        self.check_links(document, &mut out);
        check_duplicate_anchors(document, &mut out);
        check_footnotes(document, &mut out);
        check_references(document, &self.links_of(document), &mut out);
        #[cfg(feature = "frontmatter-schema")]
        if let Some(schemas) = self.schemas {
            out.extend(schemas.check(document));
        }
        out.sort_by_key(|diag| (diag.range.start, diag.range.end));
        out
    }

    /// The diagnostics for every document of the workspace, document by document in
    /// workspace order.
    pub fn check_workspace(&mut self) -> Vec<Diagnostic> {
        let workspace = self.workspace;
        workspace
            .documents()
            .flat_map(|document| self.check(document))
            .collect()
    }

    fn links_of(&self, document: &Document) -> Vec<DocumentLink> {
        links(document, self.workspace)
    }

    /// Missing files, assets, anchors and wikilink targets.
    fn check_links(&mut self, document: &Document, out: &mut Vec<Diagnostic>) {
        for link in self.links_of(document) {
            let wiki = matches!(link.kind, NodeType::WikiLink | NodeType::Embed);
            let report = |code, severity, message: String| {
                Diagnostic::new(code, severity, document.name(), link.span, message)
            };
            match &link.target {
                Target::Anchor(anchor) if !has_anchor(document, anchor, wiki) => {
                    out.push(report(
                        codes::MISSING_ANCHOR,
                        Severity::Error,
                        format!("no anchor `#{anchor}` in this document"),
                    ));
                }
                Target::File { path, anchor } => {
                    let written = written_destination(document, &link);
                    let resolved = || format!("resolved to `{}`", path.display());
                    if self.workspace.get(path).is_none() && !path.exists() {
                        let (code, what) = if link.kind == NodeType::Image {
                            (codes::MISSING_ASSET, "image")
                        } else {
                            (codes::MISSING_FILE, "file")
                        };
                        let message = format!("{what} `{written}` does not exist");
                        out.push(report(code, Severity::Error, message).with_help(resolved()));
                    } else if let Some(anchor) = anchor
                        && self.lacks_anchor(path, anchor, false)
                    {
                        let message = format!("no anchor `#{anchor}` in `{written}`");
                        out.push(
                            report(codes::MISSING_ANCHOR, Severity::Error, message)
                                .with_help(resolved()),
                        );
                    }
                }
                Target::WikiLink { name, anchor } => match self.wikilink_target(name) {
                    None => out.push(report(
                        codes::UNRESOLVED_WIKILINK,
                        Severity::Warning,
                        format!("no file named `{name}` in the vault"),
                    )),
                    Some(path) => {
                        if let Some(anchor) = anchor
                            && self.lacks_anchor(&path, anchor, true)
                        {
                            let message = format!("no anchor `#{anchor}` in `{name}`");
                            let help = format!("`{name}` is `{}`", path.display());
                            out.push(
                                report(codes::MISSING_ANCHOR, Severity::Error, message)
                                    .with_help(help),
                            );
                        }
                    }
                },
                _ => {}
            }
        }
    }

    /// Whether the Markdown file at `path` lacks `anchor`; `false` if it is not a
    /// readable Markdown file (a directory, an image, …), whose anchors are unknown.
    fn lacks_anchor(&mut self, path: &Path, anchor: &str, wiki: bool) -> bool {
        self.markdown(path)
            .is_some_and(|target| !has_anchor(target, anchor, wiki))
    }

    /// The Markdown document at `path`: from the workspace, else loaded from disk.
    fn markdown(&mut self, path: &Path) -> Option<&Document> {
        if let Some(document) = self.workspace.get(path) {
            return Some(document);
        }
        if !has_markdown_extension(path) || !path.is_file() {
            return None;
        }
        let options = self.workspace.options();
        self.loaded
            .entry(path.to_owned())
            .or_insert_with(|| options.load(path).ok())
            .as_ref()
    }

    /// The file a wikilink `name` points at, the way Obsidian finds it: the vault file
    /// whose path ends with `name` or `name.md` (whole components, ignoring case), the
    /// shortest such path if several do.
    fn wikilink_target(&mut self, name: &str) -> Option<PathBuf> {
        let workspace = self.workspace;
        let vault = self.vault.get_or_insert_with(|| list_vault(workspace));
        let name = name.trim_start_matches("./").to_lowercase();
        let with_md = format!("{name}.md");
        let names_it = |key: &str, name: &str| {
            key.strip_suffix(name)
                .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
        };
        vault
            .iter()
            .filter(|file| names_it(&file.key, &name) || names_it(&file.key, &with_md))
            .min_by_key(|file| file.key.len())
            .map(|file| file.path.clone())
    }
}

/// The path of a [`Target::File`] link as written: its destination up to any `?` or `#`.
fn written_destination<'d>(document: &'d Document, link: &DocumentLink) -> &'d str {
    match document.ast().kind(link.node) {
        NodeKind::Link(link) | NodeKind::Image(link) => {
            let end = link.url.find(['?', '#']).unwrap_or(link.url.len());
            &link.url[..end]
        }
        _ => "",
    }
}

/// Every file under the workspace root, plus the workspace's documents.
fn list_vault(workspace: &Workspace) -> Vec<VaultFile> {
    let root = workspace.root();
    let walk_root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    let walked = WalkBuilder::new(walk_root)
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path);
    let documents = workspace
        .documents()
        .filter_map(|doc| doc.path().map(Path::to_owned));
    let mut seen = HashSet::new();
    walked
        .chain(documents)
        .filter_map(|path| {
            let relative = path.strip_prefix(walk_root).unwrap_or(&path);
            let key = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                .collect::<Vec<_>>()
                .join("/");
            seen.insert(key.clone()).then_some(VaultFile { path, key })
        })
        .collect()
}

/// Whether `document` has `anchor` (empty: the top, always there). Wikilink anchors
/// (`wiki`) are a heading's text, of which only the last `#` part counts
/// (`[[Note#Part#Sub]]`), or `^id` for a block.
fn has_anchor(document: &Document, anchor: &str, wiki: bool) -> bool {
    if anchor.is_empty() {
        return true;
    }
    if !wiki {
        return document.anchors().contains(anchor);
    }
    if let Some(id) = anchor.strip_prefix('^') {
        return has_block_id(document.text(), id);
    }
    let heading = anchor.rsplit('#').next().unwrap_or(anchor);
    document.anchors().contains(&obsidian_anchor(heading))
}

/// Whether some line of `text` ends with the Obsidian block id `^id`.
fn has_block_id(text: &str, id: &str) -> bool {
    text.lines().any(|line| {
        line.trim_end()
            .strip_suffix(id)
            .and_then(|rest| rest.strip_suffix('^'))
            .is_some_and(|rest| rest.is_empty() || rest.ends_with(char::is_whitespace))
    })
}

/// Headings whose anchor (compared as the anchor index compares) repeats an earlier one.
fn check_duplicate_anchors(document: &Document, out: &mut Vec<Diagnostic>) {
    let ast = document.ast();
    let anchors = document.anchors();
    let mut first = HashMap::new();
    for (heading, anchor) in anchors.iter() {
        let key = if anchors.is_case_insensitive() {
            anchor.to_lowercase()
        } else {
            anchor.to_owned()
        };
        match first.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(heading);
            }
            Entry::Occupied(entry) => out.push(
                Diagnostic::new(
                    codes::DUPLICATE_ANCHOR,
                    Severity::Warning,
                    document.name(),
                    ast.span(heading),
                    format!("anchor `#{anchor}` is already used by an earlier heading"),
                )
                .with_label(Label::new(ast.span(*entry.get()), "first used here")),
            ),
        }
    }
}

/// Undefined footnote references and unused footnote definitions, if the flavor has
/// footnotes.
fn check_footnotes(document: &Document, out: &mut Vec<Diagnostic>) {
    if !document.flavor().parse_options().footnotes {
        return;
    }
    let ast = document.ast();
    let mut referenced = HashSet::new();
    let mut definitions = Vec::new();
    for node in ast.descendants(ast.root()) {
        match ast.kind(node) {
            NodeKind::FootnoteReference(reference) => {
                referenced.insert(normalize_label(&reference.label));
            }
            NodeKind::FootnoteDefinition(definition) => {
                definitions.push((ast.span(node), &definition.label));
            }
            _ => {}
        }
    }
    let defined: HashSet<_> = definitions
        .iter()
        .map(|(_, label)| normalize_label(label))
        .collect();
    for found in unresolved(document, true) {
        if let Unresolved::Footnote { label, span } = found
            && !defined.contains(&normalize_label(&label))
        {
            out.push(Diagnostic::new(
                codes::UNDEFINED_FOOTNOTE,
                Severity::Error,
                document.name(),
                span,
                format!("footnote `[^{label}]` is not defined"),
            ));
        }
    }
    for (span, label) in definitions {
        if !referenced.contains(&normalize_label(label)) {
            // Just the `[^label]` part.
            let end = document.text()[span.start..span.end]
                .find(']')
                .map_or(span.end, |i| span.start + i + 1);
            out.push(Diagnostic::new(
                codes::UNUSED_FOOTNOTE,
                Severity::Warning,
                document.name(),
                Span::new(span.start, end),
                format!("footnote `[^{label}]` is never referenced"),
            ));
        }
    }
}

/// Undefined references and unused reference definitions.
fn check_references(document: &Document, links: &[DocumentLink], out: &mut Vec<Diagnostic>) {
    let definitions = reference_definitions(document);
    let defined: HashSet<_> = definitions
        .iter()
        .map(|def| normalize_label(&def.label))
        .collect();
    for found in unresolved(document, false) {
        if let Unresolved::Reference { label, span } = found
            && !defined.contains(&normalize_label(&label))
        {
            out.push(Diagnostic::new(
                codes::UNDEFINED_REFERENCE,
                Severity::Error,
                document.name(),
                span,
                format!("reference `[{label}]` is not defined"),
            ));
        }
    }
    let used: HashSet<_> = links
        .iter()
        .filter_map(|link| link.reference.as_deref().map(normalize_label))
        .collect();
    for def in definitions {
        if !used.contains(&normalize_label(&def.label)) {
            out.push(Diagnostic::new(
                codes::UNUSED_REFERENCE,
                Severity::Warning,
                document.name(),
                def.span,
                format!("reference definition `[{}]` is never used", def.label),
            ));
        }
    }
}

/// A label as `CommonMark` matches it: whitespace runs collapsed, case folded.
fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
