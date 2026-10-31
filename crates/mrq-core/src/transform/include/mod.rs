//! Include resolution: [`Includer`].
//!
//! An `<!-- mrq:include path="…" -->` region (see [`directive`](crate::directive)) stands
//! for the content of another file. [`Includer::expand`] replaces every include region of
//! a document, markers and old content alike, by that content (`mrq interpolate`);
//! [`Includer::refresh`] regenerates the content between the markers instead
//! (`--in-place`), and [`Includer::include`] gives the content of one region.
//!
//! The content of an include is made in this order:
//!
//! 1. The `path` is resolved like a link ([`Workspace::resolve`](crate::Workspace::resolve):
//!    relative to the including file, `/…` from the [root](Includer::root)) to an
//!    [`IncludeTarget`], read by the [`Loader`] and parsed.
//! 2. Its own includes are expanded, recursively. An include of a file that is already
//!    being included (the including file itself, or one that includes it) is a cycle,
//!    reported with the chain of canonical paths.
//! 3. `section="Heading"` keeps the section of the first heading whose text is
//!    `Heading` (ignoring case) or whose anchor is `Heading` (`section=install`), from
//!    the heading through its subsections. `select="…"` keeps what the
//!    [selector](crate::query::Query::selector) selects (within the section, if both are
//!    given), the results separated by blank lines. Without either, everything after the
//!    frontmatter is kept.
//! 4. `dedent` (`dedent=true`; off by default for Markdown) strips the indentation all
//!    non-blank lines share.
//! 5. `shift=N` shifts the headings by N levels ([`shift_headings`]).
//! 6. Relative links and assets are rebased to work from the including file
//!    ([`rewrite_links`]). On [expansion](Includer::expand), links to anchors of a file
//!    included whole or by `section` go to the headings they became in the expanded
//!    document (`chapter.md#setup` to `#setup-1` when an earlier include also has a
//!    Setup), in the included content and in the including file itself, whose own
//!    `#anchor`s follow its headings the same way. Headings are matched in order, and a
//!    file's links are left as they are if the numbers differ (after a `dedent` that
//!    makes headings of code, say); a link to a file without an anchor, and the links of
//!    `select` and remote includes, are not redirected.
//! 7. Leading blank lines and trailing whitespace are dropped. `frontmatter=keep` puts
//!    the frontmatter (as written) before the content; `frontmatter=merge` adds its keys
//!    to the including file's frontmatter, where the including file and earlier includes
//!    win; `frontmatter=strip` (the default) drops it.
//!
//! An include with `lang="rust"` is a *code include*: the file's text, not parsed or
//! expanded (so a file may include itself), in a fenced code block with `lang` as the
//! info string. `lines` keeps a range of lines (`10-20`, `10-`, `-20` or `10`, counted
//! from 1; an end past the last line stops there), `dedent` is on by default, and the
//! fence is longer than any run of its character in the code (tildes if `lang` has a
//! backtick). `shift`, `section`, `select` and `frontmatter` do not apply and get a
//! warning; `lines` without `lang` is an error.
//!
//! A *symbol include* is a code include of one symbol's definition: `function="main"`,
//! `struct="Point"`, `class="Greeter"`, or `symbol="Point::new"` for any kind. The name
//! may be qualified (`Point::new` or `Point.new`) and selects the symbols whose qualified
//! names end with it; none or several is an error listing the candidates. The
//! [`SymbolLocator`] (with the `tree-sitter` feature, by default) knows the language from
//! the first word of `lang` or else the file's extension, and `lang` defaults to the
//! language's name (`rust`). Doc comments, attributes and decorators before the
//! definition are left out unless `with_docs=true`. `lines` then counts from the
//! symbol's first line, and `dedent` strips a method's indentation. Without a locator, an
//! include selecting a symbol fails.
//!
//! A *remote include* has an `http(s)://` URL as its `path`. Remote includes are refused
//! unless [allowed](Includer::allow_remote) (`include.allow_remote`); with the
//! `remote-include` feature the default loader, `HttpLoader`, fetches them (and can
//! cache them on disk), without it they fail. Remote content links and includes from its
//! URL: its relative links and assets become absolute URLs
//! ([`LinkRewrite::from_url`]), and the `path`s of its own includes are resolved
//! against the URL, so they are remote too (a `file:` URL is an error).
//!
//! `sha256="…"` (64 hexadecimal digits; needs the `remote-include` feature) pins the
//! content of any include, local or remote: the text read must have that SHA-256, or the
//! include fails. A [`Loader`] with a cache may serve pinned content from it
//! ([`Loader::load_pinned`]).
//!
//! An include that fails (a missing file, a cycle, a bad attribute, a section or
//! selector that finds nothing, a `lines` range past the end, a missing or ambiguous
//! symbol, a remote include that is not allowed or cannot be fetched, a `sha256`
//! mismatch) is reported and left as it is, markers included. Unknown attributes get a
//! warning.
//!
//! ```
//! use std::collections::HashMap;
//! use std::io;
//! use mrq_core::document::DocumentOptions;
//! use mrq_core::source::SourceFile;
//! use mrq_core::transform::{IncludeTarget, Includer, Loader};
//!
//! struct Files(HashMap<&'static str, &'static str>);
//!
//! impl Loader for Files {
//!     fn load(&self, target: &IncludeTarget) -> io::Result<String> {
//!         let path = target.file().and_then(|path| path.to_str()).unwrap_or_default();
//!         self.0.get(path).map(|text| text.to_string()).ok_or(io::ErrorKind::NotFound.into())
//!     }
//! }
//!
//! let files = Files(HashMap::from([(
//!     "docs/install.md",
//!     "---\ntitle: Install\n---\n# Install\n\n![shot](img/shot.png)\n",
//! )]));
//! let src = "# Guide\n\n<!-- mrq:include path=\"install.md\" shift=1 -->\n<!-- mrq:end -->\n";
//! let doc = DocumentOptions::new().parse_source(SourceFile::new(Some("docs/guide.md".into()), src))?;
//! let expanded = Includer::new().loader(files).expand(&doc);
//! assert_eq!(expanded.diagnostics, []);
//! assert_eq!(expanded.text, "# Guide\n\n## Install\n\n![shot](img/shot.png)\n");
//! # Ok::<(), mrq_core::flavor::FlavorError>(())
//! ```

mod attrs;
mod code;
#[cfg(feature = "remote-include")]
mod http;
mod loader;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use self::attrs::{FrontmatterMode, Spec};
#[cfg(feature = "remote-include")]
pub use self::http::{HttpLoader, HttpLoaderOptions, IncludeCache};
pub use self::loader::{FileLoader, IncludeTarget, Loader};
use super::links::join_url;
use super::merge::{frontmatter_block, merge_into};
use super::{LinkMap, LinkRewrite, Location, ShiftOptions, rewrite_links, shift_headings};
#[cfg(feature = "remote-include")]
use crate::ast::DirectiveAttribute;
use crate::ast::{FrontMatterFormat, NodeKind, Span};
use crate::diag::{Diagnostic, Severity};
use crate::directive::{DirectiveKind, DirectiveRegion, container_prefix, is_container_char};
use crate::document::{Document, DocumentOptions, resolve};
use crate::frontmatter::Frontmatter;
use crate::query::{Match, Section, sections};
use crate::source::{ByteRange, SourceFile, TextEdit, apply_edits};
use crate::symbol::SymbolLocator;
#[cfg(feature = "tree-sitter")]
use crate::symbol::TreeSitterLocator;
use crate::validate::{codes, is_remote};

/// Code of include errors and warnings other than a missing file
/// ([`codes::MISSING_INCLUDE`]) or a cycle ([`codes::INCLUDE_CYCLE`]): bad or unknown
/// attributes, unreadable files, sections and selectors that find nothing, frontmatter
/// that cannot be merged.
pub const INCLUDE_CODE: &str = "include";

/// Resolves `mrq:include` directives; see the [module docs](self).
///
/// By default files are read from disk, parsed with the default [`DocumentOptions`],
/// and `/…` paths start at the current directory. Remote includes are not allowed; the
/// loader is an `HttpLoader` without a cache with the `remote-include` feature, else a
/// [`FileLoader`]. Symbols are found by a `symbol::TreeSitterLocator` with the
/// `tree-sitter` feature, and not at all without it.
#[derive(Clone)]
pub struct Includer {
    loader: Arc<dyn Loader>,
    symbols: Option<Arc<dyn SymbolLocator>>,
    options: DocumentOptions,
    root: PathBuf,
    allow_remote: bool,
}

impl fmt::Debug for Includer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Includer")
            .field("options", &self.options)
            .field("root", &self.root)
            .field("allow_remote", &self.allow_remote)
            .field("symbols", &self.symbols.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for Includer {
    fn default() -> Self {
        // Without the `tree-sitter` feature, symbol includes have no locator by default.
        #[cfg(feature = "tree-sitter")]
        let symbols = Some(tree_sitter_locator());
        #[cfg(not(feature = "tree-sitter"))]
        let symbols = None;
        #[cfg(feature = "remote-include")]
        let loader: Arc<dyn Loader> = Arc::new(HttpLoader::default());
        #[cfg(not(feature = "remote-include"))]
        let loader: Arc<dyn Loader> = Arc::new(FileLoader);
        Self {
            loader,
            symbols,
            options: DocumentOptions::new(),
            root: PathBuf::new(),
            allow_remote: false,
        }
    }
}

/// The result of [`Includer::expand`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expanded {
    /// The document with its include regions replaced by their content.
    pub text: String,
    /// Problems with the includes, in the files that have them (the document or the
    /// files it includes).
    pub diagnostics: Vec<Diagnostic>,
}

/// The result of [`Includer::include`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inclusion {
    /// The content, as lines of Markdown ending with a newline (empty if there is none),
    /// not indented for the region's container; `None` if the include failed.
    pub text: Option<String>,
    /// With `frontmatter=merge`, the included file's frontmatter mapping (merged with
    /// that of its own `frontmatter=merge` includes), for the including file's.
    pub frontmatter: Option<Map<String, Value>>,
    /// As for [`Expanded::diagnostics`].
    pub diagnostics: Vec<Diagnostic>,
}

/// What [`Includer::refresh`] does to one include region.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegionRefresh {
    /// The span of the region's start marker.
    pub marker: Span,
    /// Whether the region has no `mrq:end`; its edit inserts one.
    pub bare: bool,
    /// The edit regenerating the region's body; `None` if the body is up to date or the
    /// include failed.
    pub edit: Option<TextEdit>,
    /// Problems with the include, in the files that have them (the document or the files
    /// it includes).
    pub diagnostics: Vec<Diagnostic>,
}

impl RegionRefresh {
    /// Whether including failed somewhere: the region's own include, or one in the files
    /// it includes (whose markers are then dropped from the body). The body cannot be
    /// trusted to be what a successful refresh writes.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

impl Includer {
    /// An includer with the defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads include targets with `loader`.
    #[must_use]
    pub fn loader(mut self, loader: impl Loader + 'static) -> Self {
        self.loader = Arc::new(loader);
        self
    }

    /// Finds the symbols of symbol includes (`function="main"`) with `locator`.
    #[must_use]
    pub fn symbol_locator(mut self, locator: impl SymbolLocator + 'static) -> Self {
        self.symbols = Some(Arc::new(locator));
        self
    }

    /// Parses included files with `options` (flavor registry, `--flavor`, configuration).
    #[must_use]
    pub fn document_options(mut self, options: DocumentOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets the directory a leading `/` in include paths and links stands for, the
    /// workspace root (default: the current directory). An include in a document without
    /// a path (stdin) resolves from it too.
    #[must_use]
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = root.into();
        self
    }

    /// Whether to include `http(s)://` URLs (`include.allow_remote`; default `false`).
    /// Without, a remote include fails before the loader is asked.
    #[must_use]
    pub const fn allow_remote(mut self, allow: bool) -> Self {
        self.allow_remote = allow;
        self
    }

    /// What the include `path` written in `host` names. In remote content (a `host`
    /// whose path is an http(s) URL), every `path` is a URL, resolved against the host's.
    pub fn target(&self, host: &Document, path: &str) -> IncludeTarget {
        if let Some(url) = host.path().and_then(Path::to_str).filter(|p| is_remote(p)) {
            IncludeTarget::Url(join_url(url, path))
        } else if is_remote(path) {
            IncludeTarget::Url(path.to_owned())
        } else {
            IncludeTarget::File(resolve(&self.root, host.path(), Path::new(path)))
        }
    }

    /// `doc` with every `mrq:include` region, from its start marker through its
    /// `mrq:end` (the start marker alone if it is bare), replaced by its content, and
    /// with `frontmatter=merge` frontmatter merged into its own (a new YAML block if it
    /// has none). Lines after the first get the start marker's container prefix (`> `,
    /// list indentation); an empty content removes the markers' lines. A region whose
    /// start marker is inside the old content of another region goes with that content.
    /// Links to the included files' anchors, and `doc`'s own `#anchor`s, are redirected to
    /// the headings in the expansion (see the [module docs](self)).
    pub fn expand(&self, doc: &Document) -> Expanded {
        let mut run = Run::new(self, doc);
        let text = run.expand(doc);
        Expanded {
            text,
            diagnostics: run.diagnostics,
        }
    }

    /// The edits regenerating the body of every `mrq:include` region of `doc`, in order,
    /// as `mrq interpolate --in-place` does: one per region, except regions whose start
    /// marker is inside the old body of another (they go with that body).
    ///
    /// The body becomes the region's [content](Self::include), framed by blank lines and
    /// given the start marker's container prefix and line ending; a bare start marker
    /// gets an `<!-- mrq:end -->` after it. Directive markers left in the content (of
    /// includes that failed, and `mrq:toc` regions) are removed with their lines, so the
    /// region still pairs with its own end marker. A failed include gets no edit. The
    /// document's frontmatter is left alone, so `frontmatter=merge` has no effect: the
    /// including file's frontmatter is its source, not the expansion's. Refreshing a
    /// refreshed document changes nothing, and it expands as the original does.
    pub fn refresh(&self, doc: &Document) -> Vec<RegionRefresh> {
        let ast = doc.ast();
        let src = doc.text();
        let regions: Vec<&DirectiveRegion> = doc
            .directives()
            .iter()
            .filter(|region| region.kind == DirectiveKind::Include)
            .collect();
        let inside_content = |span: Span| {
            regions.iter().any(|region| {
                !region.content.is_empty()
                    && region.content.start <= span.start
                    && span.end <= region.content.end
            })
        };
        let mut refreshed = Vec::new();
        for &region in &regions {
            let marker = ast.span(region.start);
            if inside_content(marker) {
                continue;
            }
            let mut run = Run::new(self, doc);
            let edit = run.include(doc, region).and_then(|included| {
                let body = strip_markers(&self.options, &included.text);
                region.body_edit(src, ast, &body)
            });
            refreshed.push(RegionRefresh {
                marker,
                bare: region.is_bare(),
                edit,
                diagnostics: run.diagnostics,
            });
        }
        refreshed
    }

    /// The content of the include `region` of `doc`, for the region's body.
    ///
    /// # Panics
    ///
    /// If `region` is not an include region of `doc`.
    pub fn include(&self, doc: &Document, region: &DirectiveRegion) -> Inclusion {
        assert_eq!(region.kind, DirectiveKind::Include, "not an include region");
        let mut run = Run::new(self, doc);
        let included = run.include(doc, region);
        let (text, frontmatter) = included.map_or((None, None), |included| {
            (Some(included.text), included.frontmatter)
        });
        Inclusion {
            text,
            frontmatter,
            diagnostics: run.diagnostics,
        }
    }
}

/// The locator of symbol includes by default: a [`TreeSitterLocator`] shared by all
/// includers, so each query is compiled once.
#[cfg(feature = "tree-sitter")]
fn tree_sitter_locator() -> Arc<dyn SymbolLocator> {
    static LOCATOR: std::sync::OnceLock<Arc<TreeSitterLocator>> = std::sync::OnceLock::new();
    LOCATOR.get_or_init(Arc::default).clone()
}

/// One expansion: the files being included, and what went wrong so far.
struct Run<'i> {
    includer: &'i Includer,
    /// The [identities](IncludeTarget::identity) of the files being expanded, outermost
    /// first.
    stack: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

/// The content of an include.
struct Included {
    text: String,
    frontmatter: Option<Map<String, Value>>,
    /// For a local Markdown include, what redirecting its links into the expansion of
    /// the host needs.
    linked: Option<Linked>,
}

/// A local Markdown include before its links are rewritten for the host.
struct Linked {
    /// The content, at the included file's path, with its headings shifted.
    content: Document,
    /// The anchors the content's headings have in the included file, in order; `None`
    /// for a `select`, whose pieces may come in any order.
    anchors: Option<Vec<String>>,
    /// The frontmatter block before the content (`frontmatter=keep`), or nothing.
    front: String,
}

impl<'i> Run<'i> {
    fn new(includer: &'i Includer, host: &Document) -> Self {
        Self {
            includer,
            stack: host
                .path()
                .map(|path| target_of(path).identity())
                .into_iter()
                .collect(),
            diagnostics: Vec::new(),
        }
    }

    /// The text of `doc` with its include regions expanded.
    fn expand(&mut self, doc: &Document) -> String {
        let ast = doc.ast();
        let src = doc.text();
        let regions: Vec<&DirectiveRegion> = doc
            .directives()
            .iter()
            .filter(|region| region.kind == DirectiveKind::Include)
            .collect();
        let inside_content = |span: Span| {
            regions.iter().any(|region| {
                !region.content.is_empty()
                    && region.content.start <= span.start
                    && span.end <= region.content.end
            })
        };
        let mut pieces = Vec::new();
        let mut merges = Vec::new();
        for &region in &regions {
            if inside_content(ast.span(region.start)) {
                continue;
            }
            let Some(mut included) = self.include(doc, region) else {
                continue;
            };
            merges.extend(included.frontmatter.take());
            pieces.push((region.span(ast), included));
        }
        let front = self.merge_frontmatter(doc, &merges);
        if pieces.is_empty() && front.is_none() {
            return src.to_owned();
        }
        // Each edit with the index of its piece; the frontmatter's has none.
        let mut edits: Vec<(Option<usize>, TextEdit)> = pieces
            .iter()
            .enumerate()
            .map(|(i, (span, included))| (Some(i), replace_region(src, *span, &included.text)))
            .chain(front.clone().map(|edit| (None, edit)))
            .collect();
        // In the order `apply_edits` applies them, to know where each piece goes.
        edits.sort_by_key(|(_, edit)| (edit.range.start, edit.range.end));
        let plain: Vec<TextEdit> = edits.iter().map(|(_, edit)| edit.clone()).collect();
        let draft = apply_edits(src, &plain).expect("include edits do not overlap");
        let Some(map) = self.link_map(doc, &pieces, &edits, draft.clone()) else {
            return draft;
        };

        // Again, with the links redirected: the host's own, and those of the includes.
        let host = self.host_path(doc);
        let rewrite = LinkRewrite::new(&host, &host)
            .root(&self.includer.root)
            .map(&map);
        let mut edits: Vec<TextEdit> = rewrite_links(doc, ByteRange::new(0, src.len()), &rewrite)
            .into_iter()
            .filter(|edit| {
                !pieces
                    .iter()
                    .any(|(span, _)| span.start <= edit.range.start && edit.range.end <= span.end)
            })
            .collect();
        for (span, included) in &pieces {
            let text = match &included.linked {
                Some(linked) => {
                    let relinked = self.relink(doc, &linked.content, Some(&map));
                    format!("{}{}", linked.front, trim_blank(&relinked))
                }
                None => included.text.clone(),
            };
            edits.push(replace_region(src, *span, &text));
        }
        edits.extend(front);
        edits.sort_by_key(|edit| edit.range.start);
        apply_edits(src, &edits).expect("include and link edits do not overlap")
    }

    /// Where links in the expansion of `doc` go now, for the includes `pieces` (region
    /// span and content) that `edits` (sorted) put into `draft`, the expansion before
    /// links are redirected: the anchors of each included file go to the headings they
    /// became, and the anchors of `doc` itself to its headings, which may have new
    /// anchors next to the included ones. Headings are matched in order, for each
    /// include and for the rest of `doc`, and only if their numbers agree; `None` if
    /// nothing is mapped.
    fn link_map(
        &self,
        doc: &Document,
        pieces: &[(Span, Included)],
        edits: &[(Option<usize>, TextEdit)],
        draft: String,
    ) -> Option<LinkMap> {
        // Where each piece's content is in the draft.
        let mut replaced = vec![ByteRange::empty(0); pieces.len()];
        let mut shift = 0isize;
        for (piece, edit) in edits {
            let start = edit
                .range
                .start
                .checked_add_signed(shift)
                .expect("in the draft");
            if let Some(piece) = piece {
                replaced[*piece] = ByteRange::new(start, start + edit.replacement.len());
            }
            shift += edit.replacement.len().cast_signed() - edit.range.len().cast_signed();
        }
        let draft = doc.reparse(SourceFile::new(doc.path().map(Into::into), draft));
        let anchors: Vec<(usize, &str)> = draft
            .anchors()
            .iter()
            .map(|(id, anchor)| (draft.ast().span(id).start, anchor))
            .collect();
        let within = |range: ByteRange| -> Vec<&str> {
            anchors
                .iter()
                .filter(|&&(at, _)| range.start <= at && at < range.end)
                .map(|&(_, anchor)| anchor)
                .collect()
        };
        let mut map = LinkMap::new();
        let mut mapped = false;
        for ((_, included), &range) in pieces.iter().zip(&replaced) {
            let Some(linked) = &included.linked else {
                continue;
            };
            let (Some(from), Some(path)) = (&linked.anchors, linked.content.path()) else {
                continue;
            };
            let to = within(range);
            if from.len() != to.len() {
                continue;
            }
            for (from, to) in from.iter().zip(to) {
                // The first include of a file wins.
                if map.get(path, Some(from)).is_none() {
                    map.insert_anchor(path, from, Location::output().with_anchor(to));
                    mapped = true;
                }
            }
        }
        let own: Vec<&str> = doc
            .anchors()
            .iter()
            .filter(|&(id, _)| {
                let at = doc.ast().span(id).start;
                !pieces
                    .iter()
                    .any(|(span, _)| span.start <= at && at < span.end)
            })
            .map(|(_, anchor)| anchor)
            .collect();
        let rest: Vec<&str> = anchors
            .iter()
            .filter(|&&(at, _)| !replaced.iter().any(|r| r.start <= at && at < r.end))
            .map(|&(_, anchor)| anchor)
            .collect();
        if own.len() == rest.len() {
            let host = self.host_path(doc);
            for (from, to) in own.into_iter().zip(rest) {
                if map.get(&host, Some(from)).is_none() {
                    map.insert_anchor(&host, from, Location::output().with_anchor(to));
                    mapped = true;
                }
            }
        }
        mapped.then_some(map)
    }

    /// The path links in `host`'s expansion are written from: its own, or a file in the
    /// root for stdin.
    fn host_path(&self, host: &Document) -> PathBuf {
        host.path()
            .map_or_else(|| self.includer.root.join("-"), Path::to_path_buf)
    }

    /// The content of the include `region` of `host`; `None` after reporting why there
    /// is none.
    fn include(&mut self, host: &Document, region: &DirectiveRegion) -> Option<Included> {
        let ast = host.ast();
        let directive = region.directive(ast);
        let spec = attrs::parse(
            host,
            directive,
            ast.span(region.start),
            &mut self.diagnostics,
        )?;
        let written = spec.path.value.as_str();
        let target = self.includer.target(host, written);
        let report = |code: &str, message: String| {
            Diagnostic::new(code, Severity::Error, host.name(), spec.path.span, message)
        };

        // A code include does not expand the file, so including one being expanded is fine.
        let identity = target.identity();
        if spec.code.is_none()
            && let Some(at) = self.stack.iter().position(|open| *open == identity)
        {
            let message = format!("`mrq:include` cycle: {}", describe_cycle(&self.stack[at..]));
            self.diagnostics.push(
                report(codes::INCLUDE_CYCLE, message)
                    .with_help("remove one of the includes in the chain"),
            );
            return None;
        }
        let text = self.load(host, &spec, &target)?;
        if let Some(code) = &spec.code {
            return self.code(host, &spec, code, &text);
        }
        // Remote content's path is its URL, so its links and includes resolve against it.
        let path = Some(match &target {
            IncludeTarget::File(path) => path.clone(),
            IncludeTarget::Url(url) => PathBuf::from(url),
        });
        let included = match self
            .includer
            .options
            .parse_source(SourceFile::new(path.clone(), text))
        {
            Ok(doc) => doc,
            Err(err) => {
                self.diagnostics.push(report(
                    INCLUDE_CODE,
                    format!("cannot include `{written}`: {err}"),
                ));
                return None;
            }
        };

        self.stack.push(identity);
        let expanded = self.expand(&included);
        self.stack.pop();
        let included = if expanded == included.text() {
            included
        } else {
            included.reparse(SourceFile::new(path, expanded))
        };

        let (text, selected) = self.select(host, &spec, &included)?;
        let text = if spec.dedent() { dedent(&text) } else { text };
        let content = self.shifted(host, &spec, &included, text);
        let text = trim_blank(&self.relink(host, &content, None));
        let front = included.frontmatter();
        let block = match (spec.frontmatter, front) {
            (FrontmatterMode::Keep, Some(front)) => format!(
                "{}\n",
                &included.text()[front.span().start..front.span().end]
            ),
            _ => String::new(),
        };
        let frontmatter = match spec.frontmatter {
            FrontmatterMode::Merge => front
                .and_then(Frontmatter::value)
                .and_then(Value::as_object)
                .cloned(),
            FrontmatterMode::Strip | FrontmatterMode::Keep => None,
        };
        let remote = included
            .path()
            .and_then(Path::to_str)
            .is_some_and(is_remote);
        let linked = (!remote).then(|| Linked {
            anchors: selected.map(|span| {
                included
                    .anchors()
                    .iter()
                    .filter(|&(id, _)| {
                        let at = included.ast().span(id).start;
                        span.start <= at && at < span.end
                    })
                    .map(|(_, anchor)| anchor.to_owned())
                    .collect()
            }),
            content,
            front: block.clone(),
        });
        Some(Included {
            text: block + &text,
            frontmatter,
            linked,
        })
    }

    /// The text of `target`, the include `spec` of `host` names, checked against its
    /// `sha256`; `None` after reporting why there is none.
    fn load(&mut self, host: &Document, spec: &Spec<'_>, target: &IncludeTarget) -> Option<String> {
        let written = spec.path.value.as_str();
        let report = |code: &str, message: String| {
            Diagnostic::new(code, Severity::Error, host.name(), spec.path.span, message)
        };
        if let IncludeTarget::Url(url) = target {
            if !is_remote(url) {
                self.diagnostics.push(report(
                    INCLUDE_CODE,
                    format!("cannot include `{written}` in remote content: only http(s) URLs can be included there"),
                ));
                return None;
            }
            if !self.includer.allow_remote {
                self.diagnostics.push(
                    report(
                        INCLUDE_CODE,
                        format!("remote include `{url}` is not allowed"),
                    )
                    .with_help(
                        "set `include.allow_remote: true` in `.mrq.yaml` to fetch http(s) includes",
                    ),
                );
                return None;
            }
        }
        let loaded = match &spec.sha256 {
            Some((sha256, _)) => self.includer.loader.load_pinned(target, sha256),
            None => self.includer.loader.load(target),
        };
        let text = match loaded {
            Ok(text) => text,
            Err(err) => {
                let diagnostic = if err.kind() == io::ErrorKind::NotFound {
                    let what = if target.file().is_some() {
                        "file"
                    } else {
                        "URL"
                    };
                    report(
                        codes::MISSING_INCLUDE,
                        format!("included {what} `{written}` does not exist"),
                    )
                    .with_help(format!("resolved to `{target}`"))
                } else {
                    report(INCLUDE_CODE, format!("cannot include `{written}`: {err}"))
                };
                self.diagnostics.push(diagnostic);
                return None;
            }
        };
        // Without the feature, `sha256` is rejected with the attributes.
        #[cfg(feature = "remote-include")]
        if let Some((pinned, attr)) = &spec.sha256 {
            self.check_pin(host, written, &text, pinned, attr)?;
        }
        Some(text)
    }

    /// The part of `included` that `spec`'s `section` and `select` keep, else its body,
    /// and where that is in `included` unless it was selected.
    fn select(
        &mut self,
        host: &Document,
        spec: &Spec<'_>,
        included: &Document,
    ) -> Option<(String, Option<Span>)> {
        let written = &spec.path.value;
        let report = |span: Span, message: String| {
            Diagnostic::new(INCLUDE_CODE, Severity::Error, host.name(), span, message)
        };
        let mut sectioned = None;
        if let Some(attr) = spec.section {
            let Some(span) = find_section(included, &attr.value) else {
                self.diagnostics.push(report(
                    attr.span,
                    format!("no section `{}` in `{written}`", attr.value),
                ));
                return None;
            };
            sectioned = Some((piece(included.text(), span), span));
        }
        let Some((query, attr)) = &spec.select else {
            return Some(sectioned.map_or_else(
                || {
                    let body = included.frontmatter().map_or(0, Frontmatter::body_start);
                    let span = Span::new(body, included.text().len());
                    (included.text()[body..].to_owned(), Some(span))
                },
                |(text, span)| (text, Some(span)),
            ));
        };
        let scope = match sectioned.map(|(text, _)| text) {
            Some(text) => included.reparse(SourceFile::new(included.path().map(Into::into), text)),
            None => included.clone(),
        };
        let found = match scope.select(query) {
            Ok(found) => found,
            Err(err) => {
                self.diagnostics.push(report(attr.span, err.to_string()));
                return None;
            }
        };
        let mut spans: Vec<Span> = Vec::new();
        for span in found.iter().map(Match::span) {
            if !spans
                .iter()
                .any(|kept| kept.start <= span.start && span.end <= kept.end)
            {
                spans.push(span);
            }
        }
        if spans.is_empty() {
            self.diagnostics.push(report(
                attr.span,
                format!("`select` finds nothing in `{written}`"),
            ));
            return None;
        }
        let pieces: Vec<String> = spans
            .into_iter()
            .map(|span| piece(scope.text(), span).trim_end().to_owned())
            .collect();
        Some((pieces.join("\n\n"), None))
    }

    /// `text`, a part of `included`, as a document at `included`'s path with its headings
    /// shifted.
    fn shifted(
        &mut self,
        host: &Document,
        spec: &Spec<'_>,
        included: &Document,
        text: String,
    ) -> Document {
        let path = included.path().map(Path::to_path_buf);
        let mut doc = included.reparse(SourceFile::new(path, text));
        if let Some((by, attr)) = spec.shift
            && by != 0
        {
            let shifted = shift_headings(
                &doc,
                ByteRange::new(0, doc.text().len()),
                ShiftOptions::by(by),
            );
            // The spans are in a text made for the include: point at the attribute.
            self.diagnostics
                .extend(shifted.diagnostics.into_iter().map(|mut diagnostic| {
                    host.name().clone_into(&mut diagnostic.file);
                    diagnostic.range = attr.span;
                    diagnostic.labels.clear();
                    diagnostic
                }));
            if !shifted.edits.is_empty() {
                let text =
                    apply_edits(doc.text(), &shifted.edits).expect("shift edits do not overlap");
                doc = doc.reparse(SourceFile::new(doc.path().map(Into::into), text));
            }
        }
        doc
    }

    /// The text of `content`, included in `host`, with its links rebased for `host` and
    /// redirected as `map` says.
    fn relink(&self, host: &Document, content: &Document, map: Option<&LinkMap>) -> String {
        let Some(from) = content.path() else {
            return content.text().to_owned();
        };
        let range = ByteRange::new(0, content.text().len());
        if let Some(url) = from.to_str().filter(|from| is_remote(from)) {
            let edits = rewrite_links(content, range, &LinkRewrite::from_url(url));
            return apply_edits(content.text(), &edits).expect("link edits do not overlap");
        }
        let mut rewrite = LinkRewrite::new(from, self.host_path(host)).root(&self.includer.root);
        if let Some(map) = map {
            rewrite = rewrite.map(map);
        }
        let edits = rewrite_links(content, range, &rewrite);
        apply_edits(content.text(), &edits).expect("link edits do not overlap")
    }

    /// `Some` if `text`, the content of the include `written` in `host`, has the SHA-256
    /// `pinned` its `attr` gives; `None` after reporting that it does not.
    #[cfg(feature = "remote-include")]
    fn check_pin(
        &mut self,
        host: &Document,
        written: &str,
        text: &str,
        pinned: &str,
        attr: &DirectiveAttribute,
    ) -> Option<()> {
        let actual = http::sha256(text);
        if actual == pinned {
            return Some(());
        }
        self.diagnostics.push(
            Diagnostic::new(
                INCLUDE_CODE,
                Severity::Error,
                host.name(),
                attr.span,
                format!("the content of `{written}` does not match its `sha256`: it is `{actual}`"),
            )
            .with_help(format!(
                "if the change is expected, pin the new content with `sha256={actual}`"
            )),
        );
        None
    }

    /// The edit merging `merges` into `doc`'s frontmatter, if it changes anything.
    fn merge_frontmatter(
        &mut self,
        doc: &Document,
        includes: &[Map<String, Value>],
    ) -> Option<TextEdit> {
        if includes.is_empty() {
            return None;
        }
        let (format, own, range) = match doc.frontmatter() {
            None => (FrontMatterFormat::Yaml, Map::new(), ByteRange::empty(0)),
            Some(front) => {
                let Some(Value::Object(own)) = front.value() else {
                    self.diagnostics.push(Diagnostic::new(
                        INCLUDE_CODE,
                        Severity::Warning,
                        doc.name(),
                        front.span(),
                        "included frontmatter cannot be merged into frontmatter that is not a valid mapping",
                    ));
                    return None;
                };
                let range = ByteRange::new(front.span().start, front.span().end);
                (front.format(), own.clone(), range)
            }
        };
        let mut merged = own.clone();
        for from in includes {
            merge_into(&mut merged, from);
        }
        if merged == own {
            return None;
        }
        let block = frontmatter_block(format, merged)?;
        let replacement = if range.is_empty() {
            block
        } else {
            block.strip_suffix('\n').unwrap_or(&block).to_owned()
        };
        Some(TextEdit { range, replacement })
    }
}

/// What the path of a document names: an http(s) URL for remote content, else a file.
fn target_of(path: &Path) -> IncludeTarget {
    match path.to_str() {
        Some(url) if is_remote(url) => IncludeTarget::Url(url.to_owned()),
        _ => IncludeTarget::File(path.to_owned()),
    }
}

/// `a includes b, which includes c, which includes a` for the files `chain`, each
/// including the next and the last the first; `a includes itself` for one file.
fn describe_cycle(chain: &[String]) -> String {
    let first = &chain[0];
    if chain.len() == 1 {
        return format!("{first} includes itself");
    }
    let mut text = format!("{first} includes ");
    for file in &chain[1..] {
        text.push_str(file);
        text.push_str(", which includes ");
    }
    text.push_str(first);
    text
}

/// The span of the first section of `doc` whose heading text is `name` ignoring case,
/// or whose anchor is `name` (with or without a leading `#`).
fn find_section(doc: &Document, name: &str) -> Option<Span> {
    let name = name.trim();
    let anchor = name.strip_prefix('#').unwrap_or(name);
    sections(doc.ast())
        .into_iter()
        .find(|section| {
            let heading = section.heading();
            heading.text().trim().to_lowercase() == name.to_lowercase()
                || doc.anchors().anchor_of(heading.id()) == Some(anchor)
        })
        .map(Section::span)
}

/// The text of `span` in `src`. A span that starts after indentation starts at its line
/// instead, and loses the indentation its lines share, so a nested block reads the same
/// on its own.
fn piece(src: &str, span: Span) -> String {
    let line_start = src[..span.start].rfind('\n').map_or(0, |i| i + 1);
    let lead = &src[line_start..span.start];
    if lead.is_empty() || !lead.chars().all(|c| c == ' ' || c == '\t') {
        return src[span.start..span.end].to_owned();
    }
    dedent(&src[line_start..span.end])
}

/// `text` without the leading whitespace its non-blank lines share; blank lines become
/// empty.
fn dedent(text: &str) -> String {
    let indent = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();
    let common = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| &line[..indent(line)])
        .reduce(|common, lead| {
            let shared = common
                .bytes()
                .zip(lead.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            &common[..shared]
        })
        .unwrap_or_default()
        .len();
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (content, eol) = match line.strip_suffix('\n') {
            Some(content) => (content, "\n"),
            None => (line, ""),
        };
        if content.trim().is_empty() {
            out.push_str(eol);
        } else {
            out.push_str(&content[common..]);
            out.push_str(eol);
        }
    }
    out
}

/// `text` without leading blank lines and trailing whitespace, ending with a newline
/// unless it is empty.
fn trim_blank(text: &str) -> String {
    let mut rest = text;
    while let Some((line, after)) = rest.split_once('\n') {
        if !line.trim().is_empty() {
            break;
        }
        rest = after;
    }
    let rest = rest.trim_end();
    if rest.is_empty() {
        String::new()
    } else {
        format!("{rest}\n")
    }
}

/// `text`, the content of an include, without directive markers (parsed with `options`):
/// a marker alone on its lines goes with them, and with a blank line after them if one
/// comes before, so removing markers between blank lines leaves one.
fn strip_markers(options: &DocumentOptions, text: &str) -> String {
    let Ok(doc) = options.parse_source(SourceFile::new(None, text)) else {
        return text.to_owned();
    };
    let ast = doc.ast();
    let markers: Vec<Span> = ast
        .descendants(ast.root())
        .filter(|&id| {
            matches!(
                ast.kind(id),
                NodeKind::IncludeDirective(_) | NodeKind::TocDirective(_) | NodeKind::DirectiveEnd
            )
        })
        .map(|id| ast.span(id))
        .collect();
    if markers.is_empty() {
        return text.to_owned();
    }
    let mut removed: Vec<ByteRange> = Vec::new();
    for marker in markers {
        let edit = replace_region(text, marker, "");
        match removed.last_mut() {
            Some(last) if last.end == edit.range.start => last.end = edit.range.end,
            _ => removed.push(edit.range),
        }
    }
    let is_blank = |line: &str| {
        line.chars()
            .all(|c| is_container_char(c) || c == '\r' || c == '\n')
    };
    let edits: Vec<TextEdit> = removed
        .into_iter()
        .map(|mut range| {
            let whole_lines = (range.start == 0 || text[..range.start].ends_with('\n'))
                && (range.end == text.len() || text[..range.end].ends_with('\n'));
            let before_blank = range.start == 0
                || is_blank(
                    text[..range.start - 1]
                        .rsplit('\n')
                        .next()
                        .unwrap_or_default(),
                );
            let after = text[range.end..]
                .split_inclusive('\n')
                .next()
                .unwrap_or_default();
            if whole_lines && before_blank && !after.is_empty() && is_blank(after) {
                range.end += after.len();
            }
            TextEdit {
                range,
                replacement: String::new(),
            }
        })
        .collect();
    apply_edits(text, &edits).expect("marker removals do not overlap")
}

/// The edit replacing the region spanning `span` in `src` by `content`: lines after the
/// first get the container prefix of the start marker's line, in the marker line's line
/// ending; spaces after the end marker stay after it. An empty `content` removes the
/// markers' lines if nothing else is on them.
fn replace_region(src: &str, span: Span, content: &str) -> TextEdit {
    let line_start = src[..span.start].rfind('\n').map_or(0, |i| i + 1);
    let lead = &src[line_start..span.start];
    let rest = &src[span.end..];
    // The end marker line's ending; the file's first one if it is the last line.
    let crlf = match rest.find('\n') {
        Some(i) => rest[..i].ends_with('\r'),
        None => src.contains("\r\n"),
    };
    let eol = if crlf { "\r\n" } else { "\n" };
    if content.is_empty() {
        let line_end = if rest.starts_with(eol) {
            Some(span.end + eol.len())
        } else {
            rest.is_empty().then_some(span.end)
        };
        if let Some(line_end) = line_end
            && lead.chars().all(is_container_char)
        {
            return TextEdit {
                range: ByteRange::new(line_start, line_end),
                replacement: String::new(),
            };
        }
        return TextEdit {
            range: ByteRange::new(span.start, span.end),
            replacement: String::new(),
        };
    }
    let prefix = container_prefix(lead);
    let mut replacement = String::with_capacity(content.len());
    for (n, line) in content.lines().enumerate() {
        if n > 0 {
            replacement.push_str(eol);
            replacement.push_str(if line.is_empty() {
                prefix.trim_end()
            } else {
                &prefix
            });
        }
        replacement.push_str(line);
    }
    let marked = src[span.start..span.end].trim_end_matches([' ', '\t']);
    TextEdit {
        range: ByteRange::new(span.start, span.start + marked.len()),
        replacement,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::symbol::{Symbol, SymbolKind};
    use crate::transform::HEADING_CLAMP_CODE;

    /// Files in memory, by path as resolved, and URLs.
    struct Files(HashMap<PathBuf, String>);

    impl Loader for Files {
        fn load(&self, target: &IncludeTarget) -> io::Result<String> {
            let key = match target {
                IncludeTarget::File(path) => path.clone(),
                IncludeTarget::Url(url) => PathBuf::from(url),
            };
            self.0
                .get(&key)
                .cloned()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }
    }

    fn includer(files: &[(&str, &str)]) -> Includer {
        let files = files
            .iter()
            .map(|&(path, text)| (PathBuf::from(path), text.to_owned()))
            .collect();
        Includer::new().loader(Files(files))
    }

    fn doc(path: &str, text: &str) -> Document {
        DocumentOptions::new()
            .parse_source(SourceFile::new(Some(path.into()), text))
            .unwrap()
    }

    fn expanded(files: &[(&str, &str)], host: &str) -> String {
        let expanded = includer(files).expand(&doc("host.md", host));
        assert_eq!(expanded.diagnostics, []);
        expanded.text
    }

    fn codes_and_messages(expanded: &Expanded) -> Vec<(&str, &str)> {
        expanded
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.message.as_str()))
            .collect()
    }

    #[test]
    fn a_bare_start_marker_is_replaced_and_an_empty_include_removes_its_lines() {
        let files = [("a.md", "A\n"), ("empty.md", "---\nk: v\n---\n\n")];
        assert_eq!(
            expanded(&files, "x\n<!-- mrq:include path=a.md -->\ny\n"),
            "x\nA\ny\n"
        );
        assert_eq!(
            expanded(
                &files,
                "x\n\n<!-- mrq:include path=empty.md -->\nold\n<!-- mrq:end -->\n\ny\n"
            ),
            "x\n\n\ny\n"
        );
        assert_eq!(
            expanded(&files, "x\n\n<!-- mrq:include path=empty.md -->"),
            "x\n\n"
        );
    }

    #[test]
    fn content_takes_the_container_prefix_and_line_ending() {
        let files = [("a.md", "# A\n\ntext\n")];
        assert_eq!(
            expanded(
                &files,
                "> <!-- mrq:include path=a.md -->\r\n> <!-- mrq:end -->\r\n"
            ),
            "> # A\r\n>\r\n> text\r\n"
        );
        assert_eq!(
            expanded(&files, "- <!-- mrq:include path=a.md -->\n"),
            "- # A\n\n  text\n"
        );
    }

    #[test]
    fn included_files_are_included_once_per_include_and_only_cycles_fail() {
        let files = [
            ("a.md", "<!-- mrq:include path=d.md -->\n"),
            ("b.md", "<!-- mrq:include path=d.md -->\n"),
            ("d.md", "D\n"),
        ];
        assert_eq!(
            expanded(
                &files,
                "<!-- mrq:include path=a.md -->\n\n<!-- mrq:include path=b.md -->\n"
            ),
            "D\n\nD\n"
        );
    }

    #[test]
    fn links_to_included_files_and_their_anchors_go_into_the_expansion() {
        let files = [
            (
                "a.md",
                "# Setup\n\nSee [b setup](b.md#setup), [own](#set%75p) and [b](b.md).\n",
            ),
            (
                "b.md",
                "# Guide B\n\n## Setup\n\nBack to [a](a.md#setup), [self](#setup), [gone](a.md#gone).\n",
            ),
        ];
        let host = "# Book\n\n[Setup](a.md#setup), [B's](./b.md#setup), [top](#book)\n\n\
                    <!-- mrq:include path=a.md -->\n\n<!-- mrq:include path=b.md -->\n";
        assert_eq!(
            expanded(&files, host),
            "# Book\n\n[Setup](#setup), [B's](#setup-1), [top](#book)\n\n\
             # Setup\n\nSee [b setup](#setup-1), [own](#set%75p) and [b](b.md).\n\n\
             # Guide B\n\n## Setup\n\nBack to [a](#setup), [self](#setup-1), [gone](a.md#gone).\n"
        );
    }

    #[test]
    fn anchors_of_the_host_and_of_sections_follow_the_expansion() {
        let files = [
            ("a.md", "## Usage\n"),
            ("c.md", "# One\n\n## Setup\n\n# Two\n\n## Setup\n"),
        ];
        let host = "<!-- mrq:include path=a.md -->\n\n# Usage\n\n[mine](#usage), \
                    [two](c.md#setup-1), [one](c.md#setup)\n\n\
                    <!-- mrq:include path=c.md section=two -->\n";
        assert_eq!(
            expanded(&files, host),
            "## Usage\n\n# Usage\n\n[mine](#usage-1), [two](#setup), [one](c.md#setup)\n\n\
             # Two\n\n## Setup\n"
        );
    }

    #[test]
    fn a_file_including_itself_is_a_cycle() {
        let host = "# Self\n\n<!-- mrq:include path=./host.md -->\n<!-- mrq:end -->\n";
        let expanded = includer(&[]).expand(&doc("host.md", host));
        assert_eq!(expanded.text, host);
        let identity = IncludeTarget::File("host.md".into()).identity();
        assert_eq!(
            codes_and_messages(&expanded),
            [(
                codes::INCLUDE_CYCLE,
                &*format!("`mrq:include` cycle: {identity} includes itself")
            )]
        );
    }

    #[test]
    fn frontmatter_is_merged_into_the_host_with_the_host_and_earlier_includes_winning() {
        let files = [
            ("a.md", "---\ntitle: A\ntags: [a]\n---\nA\n"),
            ("b.md", "---\ntags: [b]\nauthor: B\n---\nB\n"),
        ];
        let includes = "<!-- mrq:include path=a.md frontmatter=merge -->\n\n<!-- mrq:include path=b.md frontmatter=merge -->\n";
        assert_eq!(
            expanded(&files, includes),
            "---\ntitle: A\ntags:\n- a\nauthor: B\n---\nA\n\nB\n"
        );
        assert_eq!(
            expanded(&files, &format!("+++\ntitle = \"Host\"\n+++\n{includes}")),
            "+++\ntitle = \"Host\"\ntags = [\"a\"]\nauthor = \"B\"\n+++\nA\n\nB\n"
        );
        assert_eq!(
            expanded(
                &files,
                &format!("---\n# c\ntitle: H\ntags: []\nauthor: H\n---\n{includes}")
            ),
            "---\n# c\ntitle: H\ntags: []\nauthor: H\n---\nA\n\nB\n"
        );
    }

    #[test]
    fn clamped_headings_are_reported_at_the_shift() {
        let files = [("a.md", "##### Five\n\n###### Six\n")];
        let host = "<!-- mrq:include path=a.md shift=1 -->\n";
        let expanded = includer(&files).expand(&doc("host.md", host));
        assert_eq!(expanded.text, "###### Five\n\n###### Six\n");
        let [clamp] = expanded.diagnostics.as_slice() else {
            panic!("{:?}", expanded.diagnostics);
        };
        assert_eq!(clamp.code, HEADING_CLAMP_CODE);
        assert_eq!(clamp.file, Path::new("host.md"));
        assert_eq!(&host[clamp.range.start..clamp.range.end], "shift=1");
        assert_eq!(clamp.labels.len(), 0);
    }

    #[test]
    fn sections_are_found_by_text_or_anchor() {
        let files = [("a.md", "# A\n\n## Set *up*\n\nS\n\n## Other\n")];
        for section in ["\"set up\"", "set-up", "#set-up"] {
            assert_eq!(
                expanded(
                    &files,
                    &format!("<!-- mrq:include path=a.md section={section} -->\n")
                ),
                "## Set *up*\n\nS\n",
                "{section}"
            );
        }
    }

    #[test]
    fn nested_selections_lose_their_indentation() {
        let files = [("a.md", "- one\n\n  ```sh\n  run\n  ```\n")];
        assert_eq!(
            expanded(&files, "<!-- mrq:include path=a.md select=\"```sh\" -->\n"),
            "```sh\nrun\n```\n"
        );
    }

    #[test]
    fn remote_includes_need_allow_remote() {
        let host = "<!-- mrq:include path=https://example.com/a.md -->\n";
        let files = [("https://example.com/a.md", "A\n")];
        let expanded = includer(&files).expand(&doc("host.md", host));
        assert_eq!(expanded.text, host);
        assert_eq!(
            codes_and_messages(&expanded),
            [(
                INCLUDE_CODE,
                "remote include `https://example.com/a.md` is not allowed"
            )]
        );
        assert_eq!(
            expanded.diagnostics[0].help.as_deref(),
            Some("set `include.allow_remote: true` in `.mrq.yaml` to fetch http(s) includes")
        );
        let expanded = includer(&files)
            .allow_remote(true)
            .expand(&doc("host.md", host));
        assert_eq!(expanded.diagnostics, []);
        assert_eq!(expanded.text, "A\n");
    }

    #[cfg(not(feature = "remote-include"))]
    #[test]
    fn remote_includes_need_the_feature() {
        let host = "<!-- mrq:include path=https://example.com/a.md -->\n\n\
                    <!-- mrq:include path=a.md sha256=00 -->\n";
        let expanded = Includer::new()
            .allow_remote(true)
            .expand(&doc("host.md", host));
        assert_eq!(expanded.text, host);
        assert_eq!(
            codes_and_messages(&expanded),
            [
                (
                    INCLUDE_CODE,
                    "cannot include `https://example.com/a.md`: remote includes need mrq built with the `remote-include` feature"
                ),
                (
                    INCLUDE_CODE,
                    "`sha256` needs mrq built with the `remote-include` feature; the include is skipped"
                ),
            ]
        );
    }

    #[test]
    fn remote_content_links_to_and_includes_from_its_url() {
        let files = [
            (
                "https://example.com/docs/guide.md",
                "# Guide\n\n[next](next.md#a) ![logo](../img/logo.png) [top](#guide) [abs](/x.md)\n\n\
                 <!-- mrq:include path=part.md -->\n",
            ),
            ("https://example.com/docs/part.md", "[p](sub/p.md)\n"),
        ];
        let host = "<!-- mrq:include path=\"https://example.com/docs/guide.md\" shift=1 -->\n";
        let expanded = includer(&files)
            .allow_remote(true)
            .expand(&doc("site/host.md", host));
        assert_eq!(expanded.diagnostics, []);
        assert_eq!(
            expanded.text,
            "## Guide\n\n[next](https://example.com/docs/next.md#a) \
             ![logo](https://example.com/img/logo.png) [top](#guide) [abs](https://example.com/x.md)\n\n\
             [p](https://example.com/docs/sub/p.md)\n"
        );
    }

    #[test]
    fn remote_content_includes_only_urls_and_its_cycles_fail() {
        let files = [
            (
                "https://example.com/a.md",
                "<!-- mrq:include path=/etc/passwd -->\n\n\
                 <!-- mrq:include path=file:///etc/passwd -->\n\n\
                 <!-- mrq:include path=./a.md -->\n",
            ),
            ("/etc/passwd", "secret\n"),
        ];
        let host = "<!-- mrq:include path=https://example.com/a.md -->\n";
        let expanded = includer(&files)
            .allow_remote(true)
            .expand(&doc("host.md", host));
        assert_eq!(
            codes_and_messages(&expanded),
            [
                (
                    codes::MISSING_INCLUDE,
                    "included URL `/etc/passwd` does not exist"
                ),
                (
                    INCLUDE_CODE,
                    "cannot include `file:///etc/passwd` in remote content: only http(s) URLs can be included there"
                ),
                (
                    codes::INCLUDE_CYCLE,
                    "`mrq:include` cycle: https://example.com/a.md includes itself"
                ),
            ]
        );
        assert_eq!(
            expanded.diagnostics[0].help.as_deref(),
            Some("resolved to `https://example.com/etc/passwd`")
        );
        assert_eq!(
            expanded.diagnostics[0].file,
            Path::new("https://example.com/a.md")
        );
    }

    #[cfg(feature = "remote-include")]
    #[test]
    fn sha256_pins_the_content() {
        let good = "06f961b802bc46ee168555f066d28f4f0e9afdf3f88174c1ee6f9de004fc30a0";
        let bad = "1e2315908e1fe3cc0b0bb744cb45f26b5601d0804642191f95128d20c4206d73";
        let files = [("a.md", "A\n"), ("https://example.com/a.md", "A\n")];
        let host = format!(
            "<!-- mrq:include path=a.md sha256={good} -->\n\n\
             <!-- mrq:include path=https://example.com/a.md sha256={} -->\n\n\
             <!-- mrq:include path=a.md lang=text sha256={good} -->\n",
            good.to_uppercase()
        );
        let expanded = includer(&files)
            .allow_remote(true)
            .expand(&doc("host.md", &host));
        assert_eq!(expanded.diagnostics, []);
        assert_eq!(expanded.text, "A\n\nA\n\n```text\nA\n```\n");

        let host = format!(
            "<!-- mrq:include path=a.md sha256={bad} -->\n\n\
             <!-- mrq:include path=a.md sha256=xyz -->\n"
        );
        let expanded = includer(&files).expand(&doc("host.md", &host));
        assert_eq!(expanded.text, host);
        assert_eq!(
            codes_and_messages(&expanded),
            [
                (
                    INCLUDE_CODE,
                    &*format!("the content of `a.md` does not match its `sha256`: it is `{good}`")
                ),
                (
                    INCLUDE_CODE,
                    "`sha256` must be 64 hexadecimal digits, not `xyz`"
                ),
            ]
        );
        let mismatch = &expanded.diagnostics[0];
        assert_eq!(
            &host[mismatch.range.start..mismatch.range.end],
            format!("sha256={bad}")
        );
        assert_eq!(
            mismatch.help.as_deref(),
            Some(&*format!(
                "if the change is expected, pin the new content with `sha256={good}`"
            ))
        );
    }

    #[cfg(not(feature = "tree-sitter"))]
    #[test]
    fn symbol_includes_need_a_locator() {
        let files = [("main.rs", "fn main() {}\n")];
        let host = "<!-- mrq:include path=main.rs function=main -->\n";
        let expanded = includer(&files).expand(&doc("host.md", host));
        assert_eq!(expanded.text, host);
        assert_eq!(
            codes_and_messages(&expanded),
            [(
                INCLUDE_CODE,
                "`function` needs mrq built with the `tree-sitter` feature; the include is skipped"
            )]
        );
    }

    /// Finds `name:` lines in `.toy` files; a symbol is its line, docs the `#` lines above.
    struct Toy;

    impl SymbolLocator for Toy {
        fn language(&self, path: &Path, lang: Option<&str>) -> Option<String> {
            let toy = path.extension().is_some_and(|ext| ext == "toy") || lang == Some("toy");
            toy.then(|| "toy".to_owned())
        }

        fn languages(&self) -> Vec<String> {
            vec!["toy".to_owned()]
        }

        fn symbols(&self, _: &str, code: &str) -> Vec<Symbol> {
            let mut symbols = Vec::new();
            let mut offset = 0;
            let mut docs = None;
            for line in code.split_inclusive('\n') {
                if line.starts_with('#') {
                    docs.get_or_insert(offset);
                } else if let Some((name, _)) = line.split_once(':') {
                    symbols.push(Symbol {
                        kind: SymbolKind::Function,
                        name: name.to_owned(),
                        range: ByteRange::new(offset, offset + line.trim_end().len()),
                        docs_start: docs.take().unwrap_or(offset),
                    });
                } else {
                    docs = None;
                }
                offset += line.len();
            }
            symbols
        }
    }

    #[test]
    fn symbols_come_from_the_locator() {
        let files = [("a.toy", "x: 1\n# Why.\ny: 2\nlonger_name: 3\n")];
        let includer = includer(&files).symbol_locator(Toy);
        let host = "<!-- mrq:include path=a.toy function=y with_docs=true -->\n";
        let expanded = includer.expand(&doc("host.md", host));
        assert_eq!(expanded.diagnostics, []);
        assert_eq!(expanded.text, "```toy\n# Why.\ny: 2\n```\n");

        let host = "<!-- mrq:include path=a.toy function=zzz -->\n\n\
                    <!-- mrq:include path=a.toy class=x -->\n";
        let expanded = includer.expand(&doc("host.md", host));
        assert_eq!(
            codes_and_messages(&expanded),
            [
                (
                    INCLUDE_CODE,
                    "no function `zzz` in `a.toy`; it has function `x` (line 1), function `y` (line 3), function `longer_name` (line 4)"
                ),
                (
                    INCLUDE_CODE,
                    "no class `x` in `a.toy`; did you mean function `x` (line 1)?"
                ),
            ]
        );
        assert_eq!(
            expanded.diagnostics[1].help.as_deref(),
            Some("`symbol=` selects a symbol of any kind")
        );
    }

    #[test]
    fn code_fences_outlast_any_backtick_run_and_use_tildes_for_backtick_info() {
        let files = [
            ("a.txt", "x ``````` y\n"),
            ("b.txt", "~~~~\n```\n"),
            ("c.txt", "\r\n  one\r\n\r\n    two\r\n\r\n"),
        ];
        assert_eq!(
            expanded(&files, "<!-- mrq:include path=a.txt lang=text -->\n"),
            "````````text\nx ``````` y\n````````\n"
        );
        assert_eq!(
            expanded(&files, "<!-- mrq:include path=b.txt lang=\"a`b\" -->\n"),
            "~~~~~a`b\n~~~~\n```\n~~~~~\n"
        );
        assert_eq!(
            expanded(&files, "<!-- mrq:include path=c.txt lang=text -->\r\n"),
            "```text\r\none\r\n\r\n  two\r\n```\r\n"
        );
    }

    #[test]
    fn one_code_region_gives_its_fenced_content() {
        let files = [("src/lib.rs", "fn a() {}\nfn b() {}\n")];
        let host = doc(
            "host.md",
            "<!-- mrq:include path=src/lib.rs lang=rust lines=2 -->\n<!-- mrq:end -->\n",
        );
        let inclusion = includer(&files).include(&host, &host.directives()[0]);
        assert_eq!(inclusion.diagnostics, []);
        assert_eq!(inclusion.text.as_deref(), Some("```rust\nfn b() {}\n```\n"));
        assert_eq!(inclusion.frontmatter, None);
    }

    #[test]
    fn includes_from_stdin_resolve_from_the_root() {
        let files = [("site/a.md", "![i](img/i.png)\n")];
        let host = Document::parse("<!-- mrq:include path=a.md -->\n");
        let expanded = includer(&files).root("site").expand(&host);
        assert_eq!(expanded.diagnostics, []);
        assert_eq!(expanded.text, "![i](img/i.png)\n");
    }

    /// The text of `host` (as `host.md`) after one refresh, which must report nothing,
    /// checking that a second refresh changes nothing.
    fn refreshed(files: &[(&str, &str)], host: &str) -> String {
        let includer = includer(files);
        let refresh = includer.refresh(&doc("host.md", host));
        assert!(
            refresh.iter().all(|region| region.diagnostics.is_empty()),
            "{refresh:?}"
        );
        let edits: Vec<TextEdit> = refresh
            .into_iter()
            .filter_map(|region| region.edit)
            .collect();
        let text = apply_edits(host, &edits).unwrap();
        let again = includer.refresh(&doc("host.md", &text));
        assert!(
            again.iter().all(|region| region.edit.is_none()),
            "{text:?}: {again:?}"
        );
        text
    }

    #[test]
    fn a_refresh_writes_the_content_between_the_markers_framed_by_blank_lines() {
        let files = [("a.md", "# A\n\ntext\n"), ("empty.md", "---\nk: v\n---\n")];
        assert_eq!(
            refreshed(
                &files,
                "x\n\n<!-- mrq:include path=a.md -->\nold\n<!-- mrq:end -->\n\ny\n"
            ),
            "x\n\n<!-- mrq:include path=a.md -->\n\n# A\n\ntext\n\n<!-- mrq:end -->\n\ny\n"
        );
        assert_eq!(
            refreshed(
                &files,
                "<!-- mrq:include path=empty.md -->\nold\n<!-- mrq:end -->\n"
            ),
            "<!-- mrq:include path=empty.md -->\n<!-- mrq:end -->\n"
        );
        let fresh = "<!-- mrq:include path=a.md -->\n\n# A\n\ntext\n\n<!-- mrq:end -->\n";
        let refresh = includer(&files).refresh(&doc("host.md", fresh));
        let [region] = refresh.as_slice() else {
            panic!("{refresh:?}");
        };
        assert_eq!(region.edit, None);
        assert_eq!(
            &fresh[region.marker.start..region.marker.end],
            "<!-- mrq:include path=a.md -->"
        );
        assert!(!region.bare && !region.has_errors());
    }

    #[test]
    fn a_bare_start_marker_gets_its_content_and_an_end_marker() {
        let files = [("a.md", "# A\n\ntext\n")];
        let filled = "<!-- mrq:include path=a.md -->\n\n# A\n\ntext\n\n<!-- mrq:end -->\n";
        assert_eq!(
            refreshed(&files, "<!-- mrq:include path=a.md -->\n"),
            filled
        );
        assert_eq!(refreshed(&files, "<!-- mrq:include path=a.md -->"), filled);
        let refresh = includer(&files).refresh(&doc("host.md", "<!-- mrq:include path=a.md -->\n"));
        assert!(refresh[0].bare);
    }

    #[test]
    fn refreshed_content_takes_the_container_prefix_and_line_ending() {
        let files = [("a.md", "# A\n\ntext\n")];
        assert_eq!(
            refreshed(
                &files,
                "> <!-- mrq:include path=a.md -->\r\n> <!-- mrq:end -->\r\n"
            ),
            "> <!-- mrq:include path=a.md -->\r\n>\r\n> # A\r\n>\r\n> text\r\n>\r\n> <!-- mrq:end -->\r\n"
        );
        assert_eq!(
            refreshed(&files, "- <!-- mrq:include path=a.md -->\n"),
            "- <!-- mrq:include path=a.md -->\n\n  # A\n\n  text\n\n  <!-- mrq:end -->\n"
        );
    }

    #[test]
    fn markers_left_in_included_content_are_removed_so_the_region_still_pairs() {
        let files = [(
            "a.md",
            "A\n\n<!-- mrq:include path=gone.md -->\n<!-- mrq:end -->\n\n\
             <!-- mrq:toc -->\n- [A](#a)\n<!-- mrq:end -->\n\nB\n",
        )];
        let host = "<!-- mrq:include path=a.md -->\n<!-- mrq:end -->\n\nafter\n";
        let includer = includer(&files);
        let refresh = includer.refresh(&doc("host.md", host));
        let [region] = refresh.as_slice() else {
            panic!("{refresh:?}");
        };
        assert_eq!(
            region
                .diagnostics
                .iter()
                .map(|d| (d.code.as_str(), d.file.to_str().unwrap()))
                .collect::<Vec<_>>(),
            [(codes::MISSING_INCLUDE, "a.md")]
        );
        assert!(region.has_errors());
        let text = apply_edits(host, region.edit.as_slice()).unwrap();
        assert_eq!(
            text,
            "<!-- mrq:include path=a.md -->\n\nA\n\n- [A](#a)\n\nB\n\n<!-- mrq:end -->\n\nafter\n"
        );
        let again = doc("host.md", &text);
        assert_eq!(again.directives().len(), 1);
        assert!(!again.directives()[0].is_bare());
        assert_eq!(includer.refresh(&again)[0].edit, None);
    }

    #[test]
    fn a_failed_include_is_not_refreshed() {
        let host = "<!-- mrq:include path=gone.md -->\nold\n<!-- mrq:end -->\n";
        let refresh = includer(&[]).refresh(&doc("host.md", host));
        let [region] = refresh.as_slice() else {
            panic!("{refresh:?}");
        };
        assert_eq!(region.edit, None);
        assert!(region.has_errors());
        assert_eq!(region.diagnostics[0].code, codes::MISSING_INCLUDE);
    }

    #[test]
    fn a_refresh_leaves_the_host_frontmatter_alone() {
        let files = [("a.md", "---\nk: v\n---\nA\n")];
        assert_eq!(
            refreshed(
                &files,
                "---\ntitle: H\n---\n<!-- mrq:include path=a.md frontmatter=merge -->\n"
            ),
            "---\ntitle: H\n---\n<!-- mrq:include path=a.md frontmatter=merge -->\n\nA\n\n<!-- mrq:end -->\n"
        );
    }

    #[test]
    fn one_region_gives_its_content_and_frontmatter() {
        let files = [("docs/a.md", "---\nk: v\n---\n# A\n\n[b](b.md)\n")];
        let host = doc(
            "host.md",
            "> <!-- mrq:include path=docs/a.md frontmatter=merge -->\n> <!-- mrq:end -->\n",
        );
        let inclusion = includer(&files).include(&host, &host.directives()[0]);
        assert_eq!(inclusion.diagnostics, []);
        assert_eq!(inclusion.text.as_deref(), Some("# A\n\n[b](docs/b.md)\n"));
        assert_eq!(
            inclusion.frontmatter,
            Some(Map::from_iter([("k".into(), Value::from("v"))]))
        );
    }
}
