//! Internal checks: [`Checker`] and the diagnostic [`codes`].

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

#[cfg(feature = "frontmatter-schema")]
use super::FrontmatterSchemas;
use super::brackets::{Unresolved, unresolved};
use super::directives::check_toc;
use super::external::External;
use super::{DocumentLink, Target, links, reference_definitions};
use super::{ExternalChecker, ExternalExclude};
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
    /// An `mrq:toc` region whose list differs from what `mrq toc --insert` writes.
    pub const STALE_TOC: &str = "stale-toc";
    /// An `mrq:include` region whose body differs from what `mrq interpolate --in-place`
    /// writes.
    pub const STALE_INCLUDE: &str = "stale-include";
    /// An `mrq:include` without a `path`, or whose file does not exist.
    pub const MISSING_INCLUDE: &str = "missing-include";
    /// An `mrq:include` that, through the files it includes, includes its own file.
    pub const INCLUDE_CYCLE: &str = "include-cycle";
    /// An external http(s) link that the [`ExternalChecker`](super::ExternalChecker)
    /// finds broken.
    pub const BROKEN_EXTERNAL_LINK: &str = "broken-external-link";
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
    pub(super) workspace: &'w Workspace,
    /// Markdown files outside the workspace by path; `None` if unreadable.
    loaded: HashMap<PathBuf, Option<Document>>,
    /// The vault's files, listed on first use.
    vault: Option<Vault>,
    #[cfg(feature = "frontmatter-schema")]
    schemas: Option<&'w FrontmatterSchemas>,
    /// External link checks, if enabled.
    external: Option<External<'w>>,
}

/// The files wikilinks name, the way Obsidian finds them: every file under a
/// workspace's root (honouring ignore files and skipping hidden ones), plus the
/// workspace's documents. Listed once by [`list`](Self::list); look names up with
/// [`find`](Self::find).
#[derive(Debug, Clone)]
pub struct Vault {
    files: Vec<VaultFile>,
}

/// A file a wikilink may name.
#[derive(Debug, Clone)]
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
            external: None,
        }
    }

    /// Also checks frontmatter against `schemas` (code [`codes::FRONTMATTER_SCHEMA`]).
    #[cfg(feature = "frontmatter-schema")]
    #[must_use]
    pub const fn with_frontmatter_schemas(mut self, schemas: &'w FrontmatterSchemas) -> Self {
        self.schemas = Some(schemas);
        self
    }

    /// Also checks external http(s) links with `checker`, except those `exclude` matches
    /// (code [`codes::BROKEN_EXTERNAL_LINK`]). Without this, no external link is checked.
    ///
    /// Each distinct URL is checked once per checker: [`check_workspace`](Self::check_workspace)
    /// hands the URLs of all documents to `checker` in one batch, [`check`](Self::check)
    /// those of its document that are not checked yet.
    #[must_use]
    pub fn with_external_links(
        mut self,
        checker: &'w dyn ExternalChecker,
        exclude: &'w ExternalExclude,
    ) -> Self {
        self.external = Some(External::new(checker, exclude));
        self
    }

    /// The diagnostics for `document`, ordered by position.
    pub fn check(&mut self, document: &Document) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        self.check_links(document, &mut out);
        if let Some(external) = &mut self.external {
            external.check(document, &links(document, self.workspace), &mut out);
        }
        check_duplicate_anchors(document, &mut out);
        check_footnotes(document, &mut out);
        check_references(document, &self.links_of(document), &mut out);
        check_toc(document, &mut out);
        self.check_includes(document, &mut out);
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
        if let Some(external) = &mut self.external {
            let all: Vec<DocumentLink> = workspace
                .documents()
                .flat_map(|document| links(document, workspace))
                .collect();
            external.prefetch(&all);
        }
        workspace
            .documents()
            .flat_map(|document| self.check(document))
            .collect()
    }

    fn links_of(&self, document: &Document) -> Vec<DocumentLink> {
        links(document, self.workspace)
    }

    /// The [`codes::MISSING_ANCHOR`] findings of `document` that need no vault: links to
    /// anchors of the document itself and, with `cross_file`, to anchors of other
    /// Markdown files by path. Wikilinks to other notes are skipped. Lint rule `MRQ001`
    /// mirrors these.
    pub(crate) fn missing_anchors(
        &mut self,
        document: &Document,
        cross_file: bool,
    ) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for link in self.links_of(document) {
            match &link.target {
                Target::Anchor(anchor) => out.extend(missing_own_anchor(document, &link, anchor)),
                Target::File {
                    path,
                    anchor: Some(anchor),
                } if cross_file => {
                    out.extend(self.missing_file_anchor(document, &link, path, anchor));
                }
                _ => {}
            }
        }
        out
    }

    /// A finding if the Markdown file at `path`, which `link` of `document` points at,
    /// lacks `anchor`.
    fn missing_file_anchor(
        &mut self,
        document: &Document,
        link: &DocumentLink,
        path: &Path,
        anchor: &str,
    ) -> Option<Diagnostic> {
        if !self.lacks_anchor(path, anchor, false) {
            return None;
        }
        let written = written_destination(document, link);
        let message = format!("no anchor `#{anchor}` in `{written}`");
        Some(
            Diagnostic::new(
                codes::MISSING_ANCHOR,
                Severity::Error,
                document.name(),
                link.span,
                message,
            )
            .with_help(format!("resolved to `{}`", path.display())),
        )
    }

    /// Missing files, assets, anchors and wikilink targets.
    fn check_links(&mut self, document: &Document, out: &mut Vec<Diagnostic>) {
        for link in self.links_of(document) {
            let report = |code, severity, message: String| {
                Diagnostic::new(code, severity, document.name(), link.span, message)
            };
            match &link.target {
                Target::Anchor(anchor) => out.extend(missing_own_anchor(document, &link, anchor)),
                Target::File { path, anchor } => {
                    if self.workspace.get(path).is_none() && !path.exists() {
                        let (code, what) = if link.kind == NodeType::Image {
                            (codes::MISSING_ASSET, "image")
                        } else {
                            (codes::MISSING_FILE, "file")
                        };
                        let written = written_destination(document, &link);
                        let message = format!("{what} `{written}` does not exist");
                        let help = format!("resolved to `{}`", path.display());
                        out.push(report(code, Severity::Error, message).with_help(help));
                    } else if let Some(anchor) = anchor {
                        out.extend(self.missing_file_anchor(document, &link, path, anchor));
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
    pub(super) fn markdown(&mut self, path: &Path) -> Option<&Document> {
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

    /// The file a wikilink `name` points at; see [`Vault::find`].
    fn wikilink_target(&mut self, name: &str) -> Option<PathBuf> {
        let workspace = self.workspace;
        let vault = self.vault.get_or_insert_with(|| Vault::list(workspace));
        vault.find(name).map(Path::to_owned)
    }
}

impl Vault {
    /// The vault of `workspace`: the files under its root and its documents.
    pub fn list(workspace: &Workspace) -> Self {
        Self {
            files: list_vault(workspace),
        }
    }

    /// The file a wikilink `name` points at: the one whose path ends with `name` or
    /// `name.md` (whole components, ignoring case), the shortest such path if several do.
    pub fn find(&self, name: &str) -> Option<&Path> {
        let name = name.trim_start_matches("./").to_lowercase();
        let with_md = format!("{name}.md");
        let names_it = |key: &str, name: &str| {
            key.strip_suffix(name)
                .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
        };
        self.files
            .iter()
            .filter(|file| names_it(&file.key, &name) || names_it(&file.key, &with_md))
            .min_by_key(|file| file.key.len())
            .map(|file| file.path.as_path())
    }
}

/// A finding if `document`, which `link` points into, lacks `anchor`.
fn missing_own_anchor(
    document: &Document,
    link: &DocumentLink,
    anchor: &str,
) -> Option<Diagnostic> {
    let wiki = matches!(link.kind, NodeType::WikiLink | NodeType::Embed);
    (!has_anchor(document, anchor, wiki)).then(|| {
        Diagnostic::new(
            codes::MISSING_ANCHOR,
            Severity::Error,
            document.name(),
            link.span,
            format!("no anchor `#{anchor}` in this document"),
        )
    })
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

/// Whether `document` has `anchor`; see [`anchor_offset`].
fn has_anchor(document: &Document, anchor: &str, wiki: bool) -> bool {
    anchor_offset(document, anchor, wiki).is_some()
}

/// Where `anchor` (a link's fragment, without `#`) points in `document`, as a source
/// offset: the start of the heading with that [anchor](Document::anchors), or 0 for an
/// empty anchor (the top). Wikilink anchors (`wiki`) are a heading's text, of which only
/// the last `#` part counts (`[[Note#Part#Sub]]`), or `^id` for the line ending with
/// that block id. `None` if `document` has no such target.
///
/// ```
/// use mrq_core::Document;
/// use mrq_core::validate::anchor_offset;
///
/// let doc = Document::parse("# Intro\n\n## Setup\n");
/// assert_eq!(anchor_offset(&doc, "setup", false), Some(9));
/// assert_eq!(anchor_offset(&doc, "nope", false), None);
/// ```
pub fn anchor_offset(document: &Document, anchor: &str, wiki: bool) -> Option<usize> {
    if anchor.is_empty() {
        return Some(0);
    }
    let heading = if wiki {
        if let Some(id) = anchor.strip_prefix('^') {
            return block_id_offset(document.text(), id);
        }
        let heading = anchor.rsplit('#').next().unwrap_or(anchor);
        document.anchors().get(&obsidian_anchor(heading))
    } else {
        document.anchors().get(anchor)
    };
    heading.map(|node| document.ast().span(node).start)
}

/// The offset of the first line of `text` that ends with the Obsidian block id `^id`.
fn block_id_offset(text: &str, id: &str) -> Option<usize> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let found = line
            .trim_end()
            .strip_suffix(id)
            .and_then(|rest| rest.strip_suffix('^'))
            .is_some_and(|rest| rest.is_empty() || rest.ends_with(char::is_whitespace));
        if found {
            return Some(offset);
        }
        offset += line.len();
    }
    None
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
