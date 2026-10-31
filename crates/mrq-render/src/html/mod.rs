//! The HTML backend: mrq's own renderer over the mrq AST, so every flavor node renders
//! the same way whatever parsed it.
//!
//! # Output
//!
//! Core `CommonMark` and GFM nodes render as cmark-gfm does (the spec suites pass after
//! normalization, see `tests/html_spec.rs`), with these additions:
//!
//! - **Headings** get the `id` the flavor's slugger gives them (the document's
//!   [`anchors`](mrq_core::Document::anchors), so `{#id}` wins); mkdocs heading
//!   attributes add their classes and `key=value` attributes.
//! - **Code blocks** with a known language are highlighted with CSS classes
//!   (`<pre class="hl-code">` and `<span class="hl-…">` per scope atom, see
//!   [`Highlighter::highlight_html`]) whose colors come from the stylesheet. Plain code
//!   uses `colors.code_*`. A [`CodeBlockHook`] can replace a block (diagrams).
//! - **Alerts, callouts and admonitions** are `<div class="admonition KIND">` with a
//!   `<p class="admonition-title">`, or `<details>`/`<summary>` when foldable (`open`
//!   for `+`). The title defaults to the kind capitalized; an mkdocs `""` title has none.
//! - **Footnotes** are numbered by first reference: `<sup class="footnote-ref">` links
//!   to a closing `<section class="footnotes">` with back-references, as on GitHub.
//!   Unreferenced definitions are left out.
//! - **Task items** start with a disabled checkbox (custom Obsidian states carry
//!   `data-task`).
//! - **Math** is `MathML` from `pulldown-latex`, with the TeX source as annotation.
//! - **Obsidian** wikilinks are `<a class="wikilink">` to `target.html#anchor`, image
//!   embeds `<img>` (an alias `WxH` sets the size), other embeds links; tags are
//!   `<span class="tag">`, `%%` comments vanish.
//! - **mkdocs** definition lists are `<dl>`; abbreviations wrap whole-word occurrences
//!   in `<abbr title>`.
//! - Frontmatter, link and abbreviation definitions and `mrq:` directives render
//!   nothing; HTML comments are kept. Raw HTML passes through, with GFM's tag filter
//!   when [`HtmlOptions::tagfilter`] is set.
//!
//! # Pages and the stylesheet
//!
//! A standalone page ([`HtmlOptions::fragment`] off) is an HTML5 document with the
//! stylesheet inline and the body in `<main>`; its `<title>` is the frontmatter
//! `title`, else the first H1's text, else the file stem, else `Document`, and the
//! frontmatter `lang` sets `<html lang>`. The stylesheet ([`HtmlRenderer::stylesheet`])
//! is generated from the [`Theme`] (colors, fonts with a generic fallback, heading
//! styles, the code theme's rules) followed by the theme's `overrides.html` file.

mod css;
mod escape;
mod render;

use std::fmt;
use std::path::PathBuf;

use mrq_core::Document;
use mrq_core::ast::{CodeBlock, NodeKind};

use crate::highlight::{HighlightError, Highlighter};
use crate::theme::{Color, Theme};

use escape::push_escaped;

/// How [`HtmlRenderer`] renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HtmlOptions {
    /// Only the body HTML, without `<html>`, `<head>` and stylesheet (`--fragment`).
    pub fragment: bool,
    /// Highlight code blocks with a known language; otherwise they render as plain code.
    pub highlight: bool,
    /// GFM's tag filter: `<title>`, `<textarea>`, `<style>`, `<xmp>`, `<iframe>`,
    /// `<noembed>`, `<noframes>`, `<script>` and `<plaintext>` tags in raw HTML are
    /// escaped.
    pub tagfilter: bool,
}

/// A standalone page with highlighting and the tag filter.
impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            fragment: false,
            highlight: true,
            tagfilter: true,
        }
    }
}

/// An [`HtmlRenderer`] could not be set up from its theme.
#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    /// The code theme does not load.
    #[error(transparent)]
    Highlight(#[from] HighlightError),
    /// The `overrides.html` stylesheet cannot be read.
    #[error("cannot read the HTML override {}: {source}", path.display())]
    Override {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Renders code blocks itself instead of the backend, e.g. diagrams as inline SVG.
///
/// Closures `Fn(&CodeBlock) -> Option<String>` implement it.
pub trait CodeBlockHook: Send + Sync {
    /// The HTML that replaces `block`, or `None` to leave it to the next hook and then
    /// to the backend's code block.
    fn render(&self, block: &CodeBlock) -> Option<String>;
}

impl<F: Fn(&CodeBlock) -> Option<String> + Send + Sync> CodeBlockHook for F {
    fn render(&self, block: &CodeBlock) -> Option<String> {
        self(block)
    }
}

/// Renders documents to HTML with one theme. See the [module docs](self).
pub struct HtmlRenderer {
    options: HtmlOptions,
    highlighter: Highlighter,
    stylesheet: String,
    /// `colors.alerts.caution`, for TeX errors.
    theme_caution: Color,
    code_hooks: Vec<Box<dyn CodeBlockHook>>,
}

impl fmt::Debug for HtmlRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HtmlRenderer")
            .field("options", &self.options)
            .field("code_hooks", &self.code_hooks.len())
            .finish_non_exhaustive()
    }
}

impl HtmlRenderer {
    /// A renderer for `theme`: loads its code theme and reads its `overrides.html`.
    pub fn new(theme: &Theme, options: HtmlOptions) -> Result<Self, HtmlError> {
        let highlighter = Highlighter::new(&theme.code)?;
        let mut stylesheet = css::stylesheet(theme, &highlighter);
        if let Some(path) = &theme.overrides.html {
            let extra = std::fs::read_to_string(path).map_err(|source| HtmlError::Override {
                path: path.clone(),
                source,
            })?;
            if !stylesheet.ends_with('\n') {
                stylesheet.push('\n');
            }
            stylesheet.push_str(&extra);
        }
        Ok(Self {
            options,
            highlighter,
            stylesheet,
            theme_caution: theme.colors.alerts.caution,
            code_hooks: Vec::new(),
        })
    }

    /// Adds a hook that may render code blocks; hooks are asked in the order added.
    #[must_use]
    pub fn with_code_hook(mut self, hook: impl CodeBlockHook + 'static) -> Self {
        self.code_hooks.push(Box::new(hook));
        self
    }

    /// The options.
    pub const fn options(&self) -> &HtmlOptions {
        &self.options
    }

    /// The stylesheet: generated from the theme, then `overrides.html`. Standalone pages
    /// inline it; fragments leave it to the caller.
    pub fn stylesheet(&self) -> &str {
        &self.stylesheet
    }

    /// `doc` as HTML: a fragment or a standalone page, per [`HtmlOptions::fragment`].
    pub fn render(&self, doc: &Document) -> String {
        let body = render::body(self, doc);
        if self.options.fragment {
            return body;
        }
        let mut page = String::from("<!DOCTYPE html>\n");
        match doc.frontmatter().and_then(|fm| fm.get_str(&["lang"])) {
            Some(lang) => {
                page.push_str("<html lang=\"");
                push_escaped(&mut page, lang);
                page.push_str("\">\n");
            }
            None => page.push_str("<html>\n"),
        }
        page.push_str("<head>\n<meta charset=\"utf-8\" />\n");
        page.push_str(
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n",
        );
        page.push_str("<title>");
        push_escaped(&mut page, &title(doc));
        page.push_str("</title>\n<style>\n");
        page.push_str(&self.stylesheet);
        if !page.ends_with('\n') {
            page.push('\n');
        }
        page.push_str("</style>\n</head>\n<body>\n<main>\n");
        page.push_str(&body);
        page.push_str("</main>\n</body>\n</html>\n");
        page
    }
}

/// The page title: frontmatter `title`, the first H1, the file stem or `Document`.
fn title(doc: &Document) -> String {
    if let Some(title) = doc.frontmatter().and_then(|fm| fm.get_str(&["title"])) {
        return title.to_owned();
    }
    let ast = doc.ast();
    let h1 = ast
        .descendants(ast.root())
        .find(|&id| matches!(ast.kind(id), NodeKind::Heading(h) if h.level == 1));
    if let Some(h1) = h1 {
        return ast.text(h1);
    }
    doc.path().and_then(|path| path.file_stem()).map_or_else(
        || "Document".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}
