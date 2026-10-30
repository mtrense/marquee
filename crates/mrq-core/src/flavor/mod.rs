//! Markdown flavors: which syntax is recognized, how anchors are slugged, and (later) how
//! flavor syntax is printed.
//!
//! A [`Flavor`] is data plus hooks:
//!
//! - [`parse_options`](Flavor::parse_options): the extensions the parser enables,
//! - [`slugger`](Flavor::slugger): a fresh per-document [`Slugger`] for heading anchors,
//! - [`passes`](Flavor::passes): [`Pass`]es that rewrite the parsed tree to recognize
//!   syntax the parser does not know (admonitions, wikilinks, tags, …),
//! - [`printer_hooks`](Flavor::printer_hooks): flavor-specific printing (placeholder).
//!
//! [`parse`] runs the whole pipeline for one document: split off frontmatter, parse the
//! body with the flavor's options, run the passes.
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
//! [`Registry::builtin`] holds `commonmark` ([`Commonmark`]) and `gfm` ([`Gfm`], the
//! [`DEFAULT_FLAVOR`]); `obsidian` and `mkdocs` join them in later milestones. Until then
//! resolving them is an [`FlavorError::Unknown`], like any other unregistered name. Both
//! built-ins slug with [`GithubSlugger`] and have no passes.
//!
//! # Resolution
//!
//! [`Registry::resolve`] picks the flavor from `--flavor`, then the frontmatter key
//! `mrq.flavor`, then the configuration, then [`DEFAULT_FLAVOR`]. Frontmatter is
//! detected independently of the flavor, so it can be extracted first.

mod builtin;
mod pass;
mod registry;
mod slug;

use std::fmt;
use std::path::Path;

pub use builtin::{Commonmark, Gfm};
pub use pass::{Pass, PassContext};
pub use registry::{
    FLAVOR_ERROR_CODE, FRONTMATTER_KEY, FlavorError, FlavorOrigin, Registry, Resolved,
};
pub use slug::{GithubSlugger, Slugger, github_slug};

use crate::ast::Ast;
use crate::diag::Diagnostic;
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

/// Flavor-specific printing rules for the Markdown printer.
///
/// A placeholder: it has no methods yet. The printer (M6) and the flavor printers (M6.5)
/// add methods here, each with a default that defers to the generic printer, so existing
/// implementations keep compiling.
pub trait PrinterHooks: fmt::Debug + Send + Sync {}

/// One document parsed by [`parse`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Parsed {
    /// The tree after all passes; a frontmatter node first if there is frontmatter.
    pub ast: Ast,
    pub frontmatter: Option<Frontmatter>,
    /// What the passes reported. Frontmatter errors are not included; see
    /// [`Frontmatter::diagnostic`].
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses `src` with `flavor`: splits off frontmatter
/// ([`frontmatter::parse_document`]), parses the body with [`Flavor::parser`] and runs
/// [`Flavor::passes`]. `file` names the document in diagnostics.
pub fn parse(flavor: &dyn Flavor, src: &str, file: &Path) -> Parsed {
    let (mut ast, frontmatter) = frontmatter::parse_document(src, &flavor.parser());
    let mut cx = PassContext::new(src, file, flavor);
    cx.run_passes(&mut ast);
    Parsed {
        ast,
        frontmatter,
        diagnostics: cx.into_diagnostics(),
    }
}
