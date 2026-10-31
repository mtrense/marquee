//! Include resolution: [`Includer`].
//!
//! An `<!-- mrq:include path="…" -->` region (see [`directive`](crate::directive)) stands
//! for the content of another file. [`Includer::expand`] replaces every include region of
//! a document, markers and old content alike, by that content; [`Includer::include`]
//! gives the content of one region, for tools that write it between the markers.
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
//!    ([`rewrite_links`]).
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
//! An include that fails (a missing file, a cycle, a bad attribute, a section or
//! selector that finds nothing, a `lines` range past the end, a missing or ambiguous
//! symbol) is reported and left as it is, markers included. Unknown attributes get a
//! warning. Remote includes and `sha256` are not supported yet; an include using them
//! fails.
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
mod loader;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use self::attrs::{FrontmatterMode, Spec};
pub use self::loader::{FileLoader, IncludeTarget, Loader};
use super::merge::{frontmatter_block, merge_into};
use super::{LinkRewrite, ShiftOptions, rewrite_links, shift_headings};
use crate::ast::{FrontMatterFormat, Span};
use crate::diag::{Diagnostic, Severity};
use crate::directive::{DirectiveKind, DirectiveRegion};
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
/// By default files are read from disk ([`FileLoader`]), parsed with the default
/// [`DocumentOptions`], and `/…` paths start at the current directory. Symbols are found
/// by a `symbol::TreeSitterLocator` with the `tree-sitter` feature, and not at all
/// without it.
#[derive(Clone)]
pub struct Includer {
    loader: Arc<dyn Loader>,
    symbols: Option<Arc<dyn SymbolLocator>>,
    options: DocumentOptions,
    root: PathBuf,
}

impl fmt::Debug for Includer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Includer")
            .field("options", &self.options)
            .field("root", &self.root)
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
        Self {
            loader: Arc::new(FileLoader),
            symbols,
            options: DocumentOptions::new(),
            root: PathBuf::new(),
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

    /// What the include `path` written in `host` names.
    pub fn target(&self, host: &Document, path: &str) -> IncludeTarget {
        if is_remote(path) {
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
    pub fn expand(&self, doc: &Document) -> Expanded {
        let mut run = Run::new(self, doc);
        let text = run.expand(doc);
        Expanded {
            text,
            diagnostics: run.diagnostics,
        }
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
}

impl<'i> Run<'i> {
    fn new(includer: &'i Includer, host: &Document) -> Self {
        Self {
            includer,
            stack: host
                .path()
                .map(|path| IncludeTarget::File(path.to_owned()).identity())
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
        let mut edits = Vec::new();
        let mut merges = Vec::new();
        for &region in &regions {
            if inside_content(ast.span(region.start)) {
                continue;
            }
            let Some(included) = self.include(doc, region) else {
                continue;
            };
            merges.extend(included.frontmatter);
            edits.push(replace_region(src, region.span(ast), &included.text));
        }
        edits.extend(self.merge_frontmatter(doc, &merges));
        if edits.is_empty() {
            return src.to_owned();
        }
        edits.sort_by_key(|edit| edit.range.start);
        apply_edits(src, &edits).expect("include edits do not overlap")
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
        let text = match self.includer.loader.load(&target) {
            Ok(text) => text,
            Err(err) => {
                let diagnostic = if err.kind() == io::ErrorKind::NotFound {
                    report(
                        codes::MISSING_INCLUDE,
                        format!("included file `{written}` does not exist"),
                    )
                    .with_help(format!("resolved to `{target}`"))
                } else {
                    report(INCLUDE_CODE, format!("cannot include `{written}`: {err}"))
                };
                self.diagnostics.push(diagnostic);
                return None;
            }
        };
        if let Some(code) = &spec.code {
            return self.code(host, &spec, code, &text);
        }
        // TODO(M7.7): content of a URL has no path; its links resolve against the URL.
        let path = target.file().map(Path::to_path_buf);
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

        let text = self.select(host, &spec, &included)?;
        let text = if spec.dedent() { dedent(&text) } else { text };
        let text = self.adjust(host, &spec, &included, text);
        let text = trim_blank(&text);
        let front = included.frontmatter();
        Some(match spec.frontmatter {
            FrontmatterMode::Strip => Included {
                text,
                frontmatter: None,
            },
            FrontmatterMode::Keep => {
                let block = front.map_or(String::new(), |front| {
                    format!(
                        "{}\n",
                        &included.text()[front.span().start..front.span().end]
                    )
                });
                Included {
                    text: block + &text,
                    frontmatter: None,
                }
            }
            FrontmatterMode::Merge => Included {
                text,
                frontmatter: front
                    .and_then(Frontmatter::value)
                    .and_then(Value::as_object)
                    .cloned(),
            },
        })
    }

    /// The part of `included` that `spec`'s `section` and `select` keep, else its body.
    fn select(&mut self, host: &Document, spec: &Spec<'_>, included: &Document) -> Option<String> {
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
            sectioned = Some(piece(included.text(), span));
        }
        let Some((query, attr)) = &spec.select else {
            return Some(sectioned.unwrap_or_else(|| {
                let body = included.frontmatter().map_or(0, Frontmatter::body_start);
                included.text()[body..].to_owned()
            }));
        };
        let scope = match sectioned {
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
        Some(pieces.join("\n\n"))
    }

    /// `text`, a part of `included`, with its headings shifted and its links rebased for
    /// `host`.
    fn adjust(
        &mut self,
        host: &Document,
        spec: &Spec<'_>,
        included: &Document,
        text: String,
    ) -> String {
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
        let Some(from) = included.path() else {
            return doc.text().to_owned();
        };
        let to = host
            .path()
            .map_or_else(|| self.includer.root.join("-"), Path::to_path_buf);
        let rewrite = LinkRewrite::new(from, to).root(&self.includer.root);
        let edits = rewrite_links(&doc, ByteRange::new(0, doc.text().len()), &rewrite);
        apply_edits(doc.text(), &edits).expect("link edits do not overlap")
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

/// The edit replacing the region spanning `span` in `src` by `content`: lines after the
/// first get the container prefix of the start marker's line, in the marker line's line
/// ending. An empty `content` removes the markers' lines if nothing else is on them.
fn replace_region(src: &str, span: Span, content: &str) -> TextEdit {
    let line_start = src[..span.start].rfind('\n').map_or(0, |i| i + 1);
    let lead = &src[line_start..span.start];
    let container = |c: char| matches!(c, '>' | ' ' | '\t');
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
            && lead.chars().all(container)
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
    let prefix: String = lead
        .chars()
        .map(|c| if container(c) { c } else { ' ' })
        .collect();
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
    TextEdit {
        range: ByteRange::new(span.start, span.end),
        replacement,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::symbol::{Symbol, SymbolKind};
    use crate::transform::HEADING_CLAMP_CODE;

    /// Files in memory, by path as resolved; URLs as [`FileLoader`] has them.
    struct Files(HashMap<PathBuf, String>);

    impl Loader for Files {
        fn load(&self, target: &IncludeTarget) -> io::Result<String> {
            let Some(path) = target.file() else {
                return FileLoader.load(target);
            };
            self.0
                .get(path)
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
    fn later_kinds_of_include_are_not_supported_yet() {
        let host = "<!-- mrq:include path=https://example.com/a.md -->\n\n\
                    <!-- mrq:include path=a.md sha256=00 -->\n";
        let expanded = includer(&[("a.md", "A\n")]).expand(&doc("host.md", host));
        assert_eq!(expanded.text, host);
        assert_eq!(
            codes_and_messages(&expanded),
            [
                (
                    INCLUDE_CODE,
                    "cannot include `https://example.com/a.md`: remote includes are not supported"
                ),
                (
                    INCLUDE_CODE,
                    "`sha256` is not supported yet; the include is skipped"
                ),
            ]
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
