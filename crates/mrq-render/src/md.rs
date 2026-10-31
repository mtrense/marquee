//! The md backend: the document as `mrq fmt` prints it, optionally converted to another
//! flavor (`-o md --flavor commonmark`).
//!
//! Without a target, or with the document's own flavor, the output is
//! [`fmt::format`]'s. With another target, [`fmt::format_as`] first rewrites the syntax
//! the target cannot express (wikilinks become links, callouts and admonitions alerts
//! or block quotes with a bold title, and so on; see the
//! [rules](mrq_core::fmt#down-conversion)) and prints the result with the target's
//! escaping, so it parses back with the target flavor. Fenced code goes through the
//! `fmt.code_formatters` of the configuration, as with `mrq fmt`.

use std::fmt as stdfmt;
use std::sync::Arc;

use mrq_core::Document;
use mrq_core::config::FmtConfig;
use mrq_core::diag::Diagnostic;
use mrq_core::flavor::Flavor;
use mrq_core::fmt::{self, CommandFormatter};

/// What [`MdRenderer::render_with_diagnostics`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdOutput {
    /// The Markdown.
    pub markdown: String,
    /// Warnings about parts printed as written (code formatters that failed,
    /// frontmatter that cannot be normalized), in document order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Renders documents to Markdown with one `fmt` configuration and an optional target
/// flavor. See the [module docs](self).
pub struct MdRenderer {
    config: FmtConfig,
    code: CommandFormatter,
    target: Option<Arc<dyn Flavor>>,
}

impl stdfmt::Debug for MdRenderer {
    fn fmt(&self, f: &mut stdfmt::Formatter<'_>) -> stdfmt::Result {
        f.debug_struct("MdRenderer")
            .field("config", &self.config)
            .field("target", &self.target.as_ref().map(|flavor| flavor.name()))
            .finish_non_exhaustive()
    }
}

impl MdRenderer {
    /// A renderer printing as `config` says, in each document's own flavor.
    pub fn new(config: FmtConfig) -> Self {
        let code = CommandFormatter::from_config(&config);
        Self {
            config,
            code,
            target: None,
        }
    }

    /// Converts documents to `target` (`--flavor`).
    #[must_use]
    pub fn with_target(mut self, target: Arc<dyn Flavor>) -> Self {
        self.target = Some(target);
        self
    }

    /// The target flavor, if documents are converted.
    pub fn target(&self) -> Option<&dyn Flavor> {
        self.target.as_deref()
    }

    /// `doc` as Markdown. Warnings are dropped;
    /// [`render_with_diagnostics`](Self::render_with_diagnostics) keeps them.
    pub fn render(&self, doc: &Document) -> String {
        self.render_with_diagnostics(doc).markdown
    }

    /// `doc` as Markdown, like [`render`](Self::render), and the warnings of the
    /// printer.
    pub fn render_with_diagnostics(&self, doc: &Document) -> MdOutput {
        let formatted = match &self.target {
            Some(target) => fmt::format_as_with(doc, &**target, &self.config, &self.code),
            None => fmt::format_with(doc, &self.config, &self.code),
        };
        MdOutput {
            markdown: formatted.text,
            diagnostics: formatted.diagnostics,
        }
    }
}
