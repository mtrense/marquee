//! Markdown flavors: which syntax is recognized, how anchors are slugged, and how flavor
//! syntax is printed.
//!
//! A [`Flavor`] is data plus hooks:
//!
//! - [`parse_options`](Flavor::parse_options): the extensions the parser enables,
//! - [`slugger`](Flavor::slugger): a fresh per-document [`Slugger`] for heading anchors,
//! - [`passes`](Flavor::passes): [`Pass`]es that rewrite the parsed tree to recognize
//!   syntax the parser does not know (admonitions, wikilinks, tags, …),
//! - [`printer_hooks`](Flavor::printer_hooks): [`PrinterHooks`] that tell the printer
//!   which text would be read as flavor syntax (and must be escaped) and which nodes to
//!   print from their source.
//!
//! [`parse`] runs the whole pipeline for one document: split off frontmatter, parse the
//! body with the flavor's options, run the passes, then recognize the flavor-independent
//! [directives](crate::directive).
//!
//! ```
//! use std::path::Path;
//! use mrq_core::flavor::{self, Registry};
//!
//! let src = "---\nmrq:\n  flavor: commonmark\n---\n# Hello, World!\n";
//! let frontmatter = mrq_core::frontmatter::Frontmatter::extract(src);
//! let resolved = Registry::builtin()
//!     .resolve(None, frontmatter.as_ref(), Some("gfm"))
//!     .unwrap();
//! assert_eq!(resolved.flavor.name(), "commonmark");
//!
//! let parsed = flavor::parse(&*resolved.flavor, src, Path::new("hello.md"));
//! let heading = parsed.ast.children(parsed.ast.root())[1];
//! let mut slugger = resolved.flavor.slugger();
//! assert_eq!(slugger.slug(&parsed.ast.text(heading)), "hello-world");
//! assert_eq!(slugger.slug("Hello, World!"), "hello-world-1");
//! ```
//!
//! # Built-in flavors
//!
//! [`Registry::builtin`] holds `commonmark` ([`Commonmark`]), `gfm` ([`Gfm`], the
//! [`DEFAULT_FLAVOR`]), `obsidian` ([`Obsidian`]) and `mkdocs` ([`Mkdocs`]).
//! `commonmark` and `gfm` slug with [`GithubSlugger`] and have no passes; `obsidian`
//! slugs with [`ObsidianSlugger`] and recognizes wikilinks, embeds, callouts, tags and
//! `%%` comments with passes; `mkdocs` slugs with [`MkdocsSlugger`] and recognizes
//! admonitions, heading attributes, abbreviations and `\(…\)` math with passes.
//!
//! # Heading anchors
//!
//! [`heading_anchors`] gives every heading its anchor: the explicit id of its
//! [`HeadingAttrs`](crate::ast::HeadingAttrs) (mkdocs `{#id}`) if it has one, else the
//! flavor's slug of its text. Use it rather than slugging headings one by one, so
//! explicit ids are honoured and reserved.
//!
//! # Resolution
//!
//! [`Registry::resolve`] picks the flavor from `--flavor`, then the frontmatter key
//! `mrq.flavor`, then the configuration, then [`DEFAULT_FLAVOR`]. Frontmatter is
//! detected independently of the flavor, so it can be extracted first.

mod builtin;
mod mkdocs;
mod obsidian;
mod pass;
mod registry;
mod slug;
mod text;

use std::fmt;
use std::path::Path;

pub use builtin::{Commonmark, Gfm};
pub use mkdocs::Mkdocs;
pub use obsidian::Obsidian;
pub use pass::{Pass, PassContext};
pub use registry::{
    FLAVOR_ERROR_CODE, FRONTMATTER_KEY, FlavorError, FlavorOrigin, Registry, Resolved,
};
pub use slug::{
    GithubSlugger, MkdocsSlugger, ObsidianSlugger, Slugger, github_slug, mkdocs_slug,
    obsidian_anchor,
};
pub(crate) use text::repeated_prefix;

use crate::ast::{Ast, NodeId, NodeKind};
use crate::diag::Diagnostic;
use crate::directive::{self, DirectiveRegion};
use crate::frontmatter::{self, Frontmatter};
use crate::parse::{ComrakParser, ParseOptions};

/// The flavor used when nothing chooses one.
pub const DEFAULT_FLAVOR: &str = "gfm";

/// A Markdown dialect. See the [module docs](self).
///
/// Implementations are usually unit structs; a registry shares them as
/// `Arc<dyn Flavor>`. Methods with defaults may be added over time.
pub trait Flavor: fmt::Debug + Send + Sync {
    /// The registry name, e.g. `gfm`, as used by `--flavor`, `mrq.flavor` and config.
    fn name(&self) -> &'static str;

    /// The syntax extensions the parser recognizes.
    fn parse_options(&self) -> ParseOptions;

    /// A fresh slugger for one document's heading anchors.
    fn slugger(&self) -> Box<dyn Slugger>;

    /// Whether links match heading anchors regardless of case (Obsidian's do). `false`
    /// by default.
    fn case_insensitive_anchors(&self) -> bool {
        false
    }

    /// Passes run, in order, over every parsed tree (and over fragments parsed with
    /// [`PassContext::parse_fragment`]). None by default.
    fn passes(&self) -> &[&dyn Pass] {
        &[]
    }

    /// How the printer (`mrq fmt`, `render -o md`) handles this flavor's syntax; `None`
    /// (the default) prints with the generic rules only.
    fn printer_hooks(&self) -> Option<&dyn PrinterHooks> {
        None
    }

    /// The parser for [`parse_options`](Self::parse_options).
    fn parser(&self) -> ComrakParser {
        ComrakParser::new(self.parse_options())
    }
}

/// Flavor-specific printing rules for the Markdown printer ([`crate::fmt`]).
///
/// The printer prints every node kind the same way in every flavor (an admonition as
/// `!!! kind`, a wikilink as `[[…]]`); what depends on the flavor is which text would
/// be read as its syntax when the printed document is parsed again, and so must be
/// escaped, and which nodes are better printed from their source. Every method has a
/// default that adds nothing to the generic rules (`CommonMark`, GFM and the enabled
/// [`ParseOptions`]).
pub trait PrinterHooks: fmt::Debug + Send + Sync {
    /// Whether the character at byte `at` of `text.literal` must be escaped (written
    /// with a `\` before it) so that it stays text; `false` by default.
    fn escapes(&self, text: &EscapeContext<'_>, at: usize) -> bool {
        let _ = (text, at);
        false
    }

    /// Whether a backslash before the ASCII punctuation `c` makes it plain text in this
    /// flavor; where it does not (mkdocs reads `\[` as the start of math), an escaped
    /// `c` is written as a character reference (`&#91;`) instead. `true` by default.
    fn backslash_escapes(&self, c: char) -> bool {
        let _ = c;
        true
    }

    /// The byte offset of the character to escape when a line of a paragraph (or the
    /// text of another block) starts with `line`, for flavor syntax that starts a block
    /// at the start of a line; `None` by default.
    fn line_start_marker(&self, line: &str) -> Option<usize> {
        let _ = line;
        None
    }

    /// Whether nodes of `kind` are printed from their source, as written (with the
    /// prefixes of the containers they are in replaced by the printed ones), instead of
    /// by the generic printer: for extension nodes whose syntax the generic printer
    /// does not write the way the flavor reads it. `false` by default.
    fn verbatim(&self, kind: &NodeKind) -> bool {
        let _ = kind;
        false
    }
}

/// A text node being escaped by the printer, for [`PrinterHooks::escapes`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct EscapeContext<'a> {
    /// The text, with escapes resolved.
    pub literal: &'a str,
    /// What is printed right before the text: `Some('\n')` at the start of a line,
    /// `None` at the start of its parent's children elsewhere (of emphasis, a link
    /// text), else the last character printed.
    pub before: Option<char>,
    /// What is printed right after it, where that is known: `Some('\n')` at the end of a
    /// line, `Some('\\')` before a hard break; `None` otherwise.
    pub after: Option<char>,
    /// The text is the last child of a heading.
    pub ends_heading: bool,
}

/// One document parsed by [`parse`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Parsed {
    /// The tree after all passes; a frontmatter node first if there is frontmatter.
    pub ast: Ast,
    pub frontmatter: Option<Frontmatter>,
    /// The `mrq:include` and `mrq:toc` regions, in document order; see
    /// [`directive`].
    pub directives: Vec<DirectiveRegion>,
    /// What the passes and directive recognition reported. Frontmatter errors are not
    /// included; see [`Frontmatter::diagnostic`].
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses `src` with `flavor`: splits off frontmatter
/// ([`frontmatter::parse_document`]), parses the body with [`Flavor::parser`] and runs
/// [`Flavor::passes`], then recognizes [directives](crate::directive) (whatever the
/// flavor). `file` names the document in diagnostics.
pub fn parse(flavor: &dyn Flavor, src: &str, file: &Path) -> Parsed {
    let (mut ast, frontmatter) = frontmatter::parse_document(src, &flavor.parser());
    let mut cx = PassContext::new(src, file, flavor);
    cx.run_passes(&mut ast);
    let mut diagnostics = cx.into_diagnostics();
    let (directives, directive_diagnostics) = directive::recognize(&mut ast, src, file);
    diagnostics.extend(directive_diagnostics);
    Parsed {
        ast,
        frontmatter,
        directives,
        diagnostics,
    }
}

/// The anchor of every heading in `ast`, in document order: the explicit id of its
/// [`HeadingAttrs`](crate::ast::HeadingAttrs) if it has one, else
/// `slugger.slug(&ast.text(heading))`.
///
/// All explicit ids are [reserved](Slugger::reserve) first, so a slug never repeats one,
/// even one further down (as in Python-Markdown's `toc`). Explicit ids themselves are
/// kept as written, duplicates included. Pass a fresh (or [reset](Slugger::reset))
/// slugger from the document's flavor.
pub fn heading_anchors(ast: &Ast, slugger: &mut dyn Slugger) -> Vec<(NodeId, String)> {
    heading_anchors_where(ast, slugger, |_| true)
}

/// [`heading_anchors`] for the headings `include` accepts, as if the others were absent.
pub(crate) fn heading_anchors_where(
    ast: &Ast,
    slugger: &mut dyn Slugger,
    include: impl Fn(NodeId) -> bool,
) -> Vec<(NodeId, String)> {
    let explicit = |id: NodeId| match ast.kind(id) {
        NodeKind::Heading(_) if !include(id) => None,
        NodeKind::Heading(heading) => {
            Some(heading.attrs.as_ref().and_then(|attrs| attrs.id.as_deref()))
        }
        _ => None,
    };
    let headings: Vec<(NodeId, Option<&str>)> = ast
        .descendants(ast.root())
        .filter_map(|id| explicit(id).map(|anchor| (id, anchor)))
        .collect();
    for &(_, anchor) in &headings {
        if let Some(anchor) = anchor {
            slugger.reserve(anchor);
        }
    }
    headings
        .into_iter()
        .map(|(id, anchor)| {
            let anchor = anchor.map_or_else(|| slugger.slug(&ast.text(id)), str::to_owned);
            (id, anchor)
        })
        .collect()
}
