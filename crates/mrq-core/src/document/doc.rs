//! [`Document`] and the [`DocumentOptions`] it is parsed with.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use super::AnchorIndex;
use crate::ast::json::{self, JsonAst};
use crate::ast::{Ast, NodeKind};
use crate::diag::Diagnostic;
use crate::directive::DirectiveRegion;
use crate::flavor::{self, Flavor, FlavorError, FlavorOrigin, Registry, Resolved};
use crate::frontmatter::Frontmatter;
use crate::input::{Input, InputError};
use crate::query::{JsonOutput, Match, Query, QueryError};
use crate::source::SourceFile;
use crate::toc::TocEntry;

/// The name diagnostics use for a document without a path.
const STDIN_NAME: &str = "<stdin>";

/// Why a document could not be loaded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DocumentError {
    /// The file (or stdin) could not be read.
    #[error(transparent)]
    Input(#[from] InputError),
    /// The `--flavor` or configured flavor is not registered.
    #[error(transparent)]
    Flavor(#[from] FlavorError),
}

/// How documents are parsed: the flavor registry and the flavor choices made outside the
/// document (`--flavor`, the configuration).
///
/// The [`Default`] (also [`new`](Self::new)) uses [`Registry::builtin`] and leaves the
/// choice to the frontmatter, then [`DEFAULT_FLAVOR`](crate::flavor::DEFAULT_FLAVOR).
///
/// ```
/// use mrq_core::document::DocumentOptions;
///
/// let options = DocumentOptions::new().config_flavor("mkdocs");
/// let doc = options.parse("---\nmrq:\n  flavor: obsidian\n---\n# Notes\n")?;
/// assert_eq!(doc.flavor().name(), "obsidian");
/// # Ok::<(), mrq_core::flavor::FlavorError>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct DocumentOptions {
    registry: Registry,
    flavor: Option<String>,
    config_flavor: Option<String>,
}

impl DocumentOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks flavors up in `registry` instead of [`Registry::builtin`].
    #[must_use]
    pub fn registry(mut self, registry: Registry) -> Self {
        self.registry = registry;
        self
    }

    /// Forces the flavor called `name`, as `--flavor` does: it outranks the frontmatter.
    #[must_use]
    pub fn flavor(mut self, name: impl Into<String>) -> Self {
        self.flavor = Some(name.into());
        self
    }

    /// The configured flavor (`flavor` in `.mrq.yaml`), used when the frontmatter picks
    /// none. Leave it unset when the configuration only holds its default, so documents
    /// report [`FlavorOrigin::Default`].
    #[must_use]
    pub fn config_flavor(mut self, name: impl Into<String>) -> Self {
        self.config_flavor = Some(name.into());
        self
    }

    /// The flavor registry.
    pub const fn flavor_registry(&self) -> &Registry {
        &self.registry
    }

    /// The forced flavor, if any.
    pub fn flavor_name(&self) -> Option<&str> {
        self.flavor.as_deref()
    }

    /// The configured flavor, if any.
    pub fn config_flavor_name(&self) -> Option<&str> {
        self.config_flavor.as_deref()
    }

    /// Parses in-memory text without a path; see [`parse_source`](Self::parse_source).
    pub fn parse(&self, src: impl Into<String>) -> Result<Document, FlavorError> {
        self.parse_source(SourceFile::new(None, src))
    }

    /// Reads and parses the file at `path`.
    pub fn load(&self, path: impl AsRef<Path>) -> Result<Document, DocumentError> {
        self.read(&Input::file(path.as_ref()))
    }

    /// Reads and parses an input; stdin is read from the process's standard input.
    pub fn read(&self, input: &Input) -> Result<Document, DocumentError> {
        Ok(self.parse_source(input.read()?)?)
    }

    /// Parses `source`, whose path (if any) names the document.
    ///
    /// The flavor is resolved with [`Registry::resolve`]. A bad frontmatter `mrq.flavor`
    /// (unknown, or not a string) does not fail: it becomes a diagnostic, and the
    /// configured or default flavor is used instead.
    ///
    /// # Errors
    ///
    /// The [`flavor`](Self::flavor) or [`config_flavor`](Self::config_flavor) is not in
    /// the registry.
    pub fn parse_source(&self, source: SourceFile) -> Result<Document, FlavorError> {
        let name = source
            .path()
            .map_or_else(|| PathBuf::from(STDIN_NAME), Path::to_path_buf);
        let (resolved, flavor_diagnostic) =
            self.resolve(Frontmatter::extract(source.text()).as_ref(), &name)?;
        let parsed = flavor::parse(&*resolved.flavor, source.text(), &name);
        let diagnostics = parsed
            .frontmatter
            .as_ref()
            .and_then(|fm| fm.diagnostic(&name))
            .into_iter()
            .chain(flavor_diagnostic)
            .chain(parsed.diagnostics)
            .collect();
        Ok(Document {
            source,
            name,
            flavor: resolved.flavor,
            flavor_origin: resolved.origin,
            ast: parsed.ast,
            frontmatter: parsed.frontmatter,
            directives: parsed.directives,
            diagnostics,
            anchors: OnceLock::new(),
        })
    }

    /// The document's flavor, plus a diagnostic if its frontmatter chose a bad one.
    fn resolve(
        &self,
        frontmatter: Option<&Frontmatter>,
        file: &Path,
    ) -> Result<(Resolved, Option<Diagnostic>), FlavorError> {
        let config = self.config_flavor.as_deref();
        match self
            .registry
            .resolve(self.flavor.as_deref(), frontmatter, config)
        {
            Ok(resolved) => Ok((resolved, None)),
            Err(
                err @ (FlavorError::NotAString { .. }
                | FlavorError::Unknown {
                    origin: FlavorOrigin::Frontmatter,
                    ..
                }),
            ) => {
                let resolved = self.registry.resolve(None, None, config)?;
                Ok((resolved, Some(err.to_diagnostic(file))))
            }
            Err(err) => Err(err),
        }
    }
}

/// One parsed Markdown document: its source, flavor, frontmatter, syntax tree and the
/// diagnostics found while parsing.
///
/// [`Document::parse`] and [`Document::load`] cover the common case (built-in flavors,
/// no `--flavor` or configuration); [`DocumentOptions`] covers the rest. See the
/// [module docs](super) for an example.
#[derive(Debug, Clone)]
pub struct Document {
    source: SourceFile,
    /// The path, or [`STDIN_NAME`].
    name: PathBuf,
    flavor: Arc<dyn Flavor>,
    flavor_origin: FlavorOrigin,
    ast: Ast,
    frontmatter: Option<Frontmatter>,
    directives: Vec<DirectiveRegion>,
    diagnostics: Vec<Diagnostic>,
    anchors: OnceLock<AnchorIndex>,
}

impl Document {
    /// Parses in-memory text with the built-in flavors: the frontmatter's `mrq.flavor`,
    /// else `gfm`. The document has no path, like stdin.
    pub fn parse(src: impl Into<String>) -> Self {
        DocumentOptions::new()
            .parse(src)
            .expect("the built-in registry has the default flavor")
    }

    /// Reads and parses the file at `path` with the built-in flavors.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DocumentError> {
        DocumentOptions::new().load(path)
    }

    /// `source` parsed with this document's flavor, whatever its frontmatter says: an
    /// edited version of the document, or one made from it.
    pub(crate) fn reparse(&self, source: SourceFile) -> Self {
        let name = source
            .path()
            .map_or_else(|| PathBuf::from(STDIN_NAME), Path::to_path_buf);
        let parsed = flavor::parse(&*self.flavor, source.text(), &name);
        Self {
            source,
            name,
            flavor: Arc::clone(&self.flavor),
            flavor_origin: self.flavor_origin,
            ast: parsed.ast,
            frontmatter: parsed.frontmatter,
            directives: parsed.directives,
            diagnostics: parsed.diagnostics,
            anchors: OnceLock::new(),
        }
    }

    /// The source text with its path and line index.
    pub const fn source(&self) -> &SourceFile {
        &self.source
    }

    /// The source text.
    pub fn text(&self) -> &str {
        self.source.text()
    }

    /// The file's path; `None` for stdin and in-memory text.
    pub fn path(&self) -> Option<&Path> {
        self.source.path()
    }

    /// The name diagnostics use: the path, or `<stdin>`.
    pub fn name(&self) -> &Path {
        &self.name
    }

    /// Whether results may be written back to the document's file. Documents without a
    /// path (stdin) never are.
    pub fn is_writable(&self) -> bool {
        self.path().is_some()
    }

    /// The flavor the document was parsed with.
    pub fn flavor(&self) -> &dyn Flavor {
        &*self.flavor
    }

    /// Where the flavor was chosen.
    pub const fn flavor_origin(&self) -> FlavorOrigin {
        self.flavor_origin
    }

    /// The syntax tree, after the flavor's passes. Spans index [`text`](Self::text).
    pub const fn ast(&self) -> &Ast {
        &self.ast
    }

    /// The frontmatter block, also when malformed.
    pub const fn frontmatter(&self) -> Option<&Frontmatter> {
        self.frontmatter.as_ref()
    }

    /// What parsing found: malformed frontmatter, a bad frontmatter flavor, what the
    /// flavor passes reported, then malformed or unpaired
    /// [directives](crate::directive).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The `mrq:include` and `mrq:toc` regions in document order (by start marker),
    /// paired with their `mrq:end` markers. Broken markers are left out and reported in
    /// [`diagnostics`](Self::diagnostics); see [`directive`](crate::directive).
    pub fn directives(&self) -> &[DirectiveRegion] {
        &self.directives
    }

    /// The heading anchors, built on first use.
    pub fn anchors(&self) -> &AnchorIndex {
        self.anchors.get_or_init(|| {
            let mut slugger = self.flavor.slugger();
            AnchorIndex::new(
                flavor::heading_anchors(&self.ast, &mut *slugger),
                self.flavor.case_insensitive_anchors(),
            )
        })
    }

    /// Every heading in document order, with its level, text and anchor. See
    /// [`toc`](crate::toc) for rendering it.
    pub fn toc(&self) -> Vec<TocEntry> {
        self.anchors()
            .iter()
            .filter_map(|(node, slug)| match self.ast.kind(node) {
                NodeKind::Heading(heading) => Some(TocEntry {
                    node,
                    level: heading.level,
                    text: self.ast.text(node),
                    slug: slug.to_owned(),
                    span: self.ast.span(node),
                }),
                _ => None,
            })
            .collect()
    }

    /// The [JSON projection](crate::ast::json) of the document.
    pub fn to_json_ast(&self) -> JsonAst {
        json::project(self)
    }

    /// The nodes and sections `query` matches. For selector, CSS and plan queries these
    /// are in document order (see
    /// [`QueryPlan::evaluate`](crate::query::QueryPlan::evaluate)); for jq they are the
    /// program's outputs in output order, each [mapped back](JsonOutput::to_match) to the
    /// node or section it is the projection of.
    ///
    /// # Errors
    ///
    /// For jq: [`QueryError::Runtime`] if the program fails, and
    /// [`QueryError::NotANode`] for an output that is not a node or section of the
    /// document (use [`select_json`](Self::select_json) for arbitrary values).
    pub fn select(&self, query: &Query) -> Result<Vec<Match<'_>>, QueryError> {
        if let Some(plan) = query.plan() {
            return Ok(plan.evaluate(self));
        }
        query
            .run_jq(self)?
            .into_iter()
            .map(|output| {
                output.to_match(self).ok_or_else(|| QueryError::NotANode {
                    value: output.value().to_string(),
                })
            })
            .collect()
    }

    /// The JSON results of `query`: a jq program's outputs, or the
    /// [projections](JsonOutput::from_match) of the matches of any other query.
    ///
    /// # Errors
    ///
    /// [`QueryError::Runtime`] if a jq program fails.
    pub fn select_json(&self, query: &Query) -> Result<Vec<JsonOutput>, QueryError> {
        match query.plan() {
            Some(plan) => Ok(plan
                .evaluate(self)
                .iter()
                .map(|found| JsonOutput::from_match(self, found))
                .collect()),
            None => query.run_jq(self),
        }
    }
}
