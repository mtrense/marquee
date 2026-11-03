//! The ANSI backend: documents laid out as styled lines at a terminal width, then
//! written with escape sequences. The lines are backend-neutral, so the TUI shows the
//! same layout.
//!
//! # Layers
//!
//! 1. [`AnsiRenderer::render`] walks the mrq AST into a [`StyledDocument`]: [`Line`]s
//!    of [`StyledSpan`]s (text, a [`TextStyle`], a link destination), each with the
//!    block it shows ([`BlockInfo`]: node id, source span, heading level and anchor) and
//!    maybe an image ([`Graphic`]). It depends only on [`AnsiOptions`], never on the
//!    terminal.
//! 2. [`StyledDocument::to_ansi`] writes the lines for a terminal's [`Capabilities`]:
//!    SGR styles in true colour or the 256 palette, OSC 8 hyperlinks, kitty, iTerm2 or
//!    sixel images. [`StyledDocument::plain_text`] is the same without any escapes.
//! 3. [`Capabilities::detect`] decides those from `--color`, whether output is a
//!    terminal and the environment (`NO_COLOR`, `TERM`, `COLORTERM`, `TERM_PROGRAM`, …),
//!    all passed in.
//!
//! # Layout
//!
//! Blocks are separated by a blank line (none inside tight list items). Paragraphs
//! wrap greedily at whitespace by display width (`unicode-width`), breaking words wider
//! than a line; containers draw a prefix and lay out their content in the remaining
//! width:
//!
//! - **Headings:** `#` marks, then the text in the heading style (bold unless its
//!   weight is below semibold, the heading color only when the theme's heading style
//!   sets one); `underline` adds a rule under it.
//! - **Inlines:** emphasis italic, strong bold, strikethrough, `==mark==` reversed,
//!   code in the code colors, links underlined in the accent color (their URL in
//!   parentheses after the text with [`AnsiOptions::link_urls`], unless the text is the
//!   URL), footnote references `[n]`, tags `#tag`, math as TeX, superscript `^(…)` and
//!   subscript `_(…)`, raw HTML muted; comments, directives, frontmatter and definitions
//!   show nothing. Wikilinks show their alias, `target > anchor` or target, and keep
//!   `[[target#anchor]]` as their link.
//! - **Lists:** `•`, `◦`, `▪` by depth, or numbers aligned to the widest; tasks `☐`,
//!   `☑` or `[c]`. **Blockquotes:** `│ ` and muted text. **Alerts, callouts and
//!   admonitions:** `┃ ` in the kind's alert color and a bold title (the kind
//!   capitalized by default; an Obsidian title is Markdown), with `▾`/`▸` for open and
//!   closed folds; the body always shows.
//! - **Code:** highlighted with the code theme ([`Highlighter`]), indented by two and
//!   painted in the block's background to the full width; long lines break. Diagrams go
//!   through the [`Registry`] ([`AnsiRenderer::with_diagrams`]): PNG with
//!   [`AnsiOptions::graphics`], else text (Mermaid's Unicode art); a failure shows the
//!   code with a [`codes::DIAGRAM`](crate::diagram::codes::DIAGRAM) warning.
//! - **Tables:** box drawing at the width; columns keep their natural widths when they
//!   fit, otherwise the widest gives up space and cells wrap. The header is bold.
//! - **Images:** a paragraph of only images (and Obsidian image embeds) shows them as
//!   graphics with [`AnsiOptions::graphics`], when the file is a local PNG or SVG
//!   (rasterized, feature `png`) next to the document; at most the width, never scaled
//!   up, with [`AnsiOptions::cell_size`] pixels per cell, and followed by a line per
//!   further row with [`AnsiOptions::reserve_rows`]. Otherwise, and for images in text,
//!   `[image: alt]`.
//! - **Footnotes:** numbered by first reference, listed after a short rule at the end.
//! - Definition lists: the term bold, details indented by 4. Thematic breaks: a rule.
//!
//! # Styles
//!
//! Every element's style comes from the [`Theme`]: accent, muted, border, code and
//! alert colors. Body text keeps the terminal's colors, since the theme's foreground was
//! chosen for a background a terminal does not paint. The theme's `overrides.ansi` file
//! then changes single elements: YAML mapping element names (`h1`–`h6`,
//! `heading_marker`, `heading_rule`, `emphasis`, `strong`, `strikethrough`,
//! `highlight`, `code`, `code_block`, `link`, `link_url`, `quote`, `quote_bar`, `rule`,
//! `table_border`, `table_header`, `list_marker`, `task`, `tag`, `footnote_ref`,
//! `footnote`, `html`, `math`, `image`, `definition_term`, `alert_note`, `alert_tip`,
//! `alert_important`, `alert_warning`, `alert_caution`) to `fg`, `bg` (hex colors),
//! `bold`, `dim`, `italic`, `underline`, `strikethrough` and `reverse`.
//!
//! # Escapes
//!
//! Each span that changes the style writes a full SGR (`0;…`), and a line ends with a
//! reset and a closed hyperlink, so lines stand alone (for `less -R`). Without colour no
//! SGR is written at all. Trailing whitespace is dropped where it shows nothing. Control
//! characters in the source become U+FFFD, so a document cannot send escapes. OSC 8
//! needs a URI: URLs with a scheme are used as written, relative files and anchors
//! become `file://` URIs next to the document (none for stdin), wikilinks get none.

mod escape;
mod graphics;
mod layout;
mod lines;
mod styles;
mod table;
mod terminal;
mod wrap;

use std::path::PathBuf;
use std::sync::Arc;

use mrq_core::Document;
use mrq_core::Slashed as _;
use mrq_core::diag::Diagnostic;

use crate::diagram::Registry;
use crate::highlight::{HighlightError, Highlighter};
use crate::theme::Theme;

pub use escape::palette;
pub use lines::{BlockInfo, Graphic, HeadingInfo, Image, Line, StyledSpan, TextStyle};
pub use terminal::{Capabilities, ColorChoice, ColorLevel, GraphicsProtocol};

use styles::Styles;

/// How [`AnsiRenderer`] lays out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent switches of the layout"
)]
pub struct AnsiOptions {
    /// The terminal width in columns (at least 1).
    pub width: usize,
    /// Show each link's URL after its text, for output without hyperlinks.
    pub link_urls: bool,
    /// Show images and PNG diagrams as [`Graphic`]s.
    pub graphics: bool,
    /// Highlight code blocks with a known language.
    pub highlight: bool,
    /// The pixel size of a terminal cell (width, height), for sizing images.
    pub cell_size: (u32, u32),
    /// Follow each graphic's line with a line for each further row it takes, with only
    /// the prefix of its container: for a viewer that draws the images over the lines
    /// (the TUI), so that every line is one row. [`StyledDocument::to_ansi`] leaves them
    /// blank.
    pub reserve_rows: bool,
}

/// 80 columns, link URLs shown, no graphics, highlighting on, 10×20 pixel cells, no
/// reserved rows.
impl Default for AnsiOptions {
    fn default() -> Self {
        Self {
            width: 80,
            link_urls: true,
            graphics: false,
            highlight: true,
            cell_size: (10, 20),
            reserve_rows: false,
        }
    }
}

impl AnsiOptions {
    /// The options for a terminal `width` columns wide with `caps`: URLs shown without
    /// hyperlinks, graphics when the terminal has a protocol this build can write.
    pub fn for_terminal(caps: &Capabilities, width: usize) -> Self {
        Self {
            width,
            link_urls: !caps.hyperlinks,
            graphics: caps
                .graphics
                .is_some_and(|p| p != GraphicsProtocol::Sixel || cfg!(feature = "png")),
            ..Self::default()
        }
    }
}

/// An [`AnsiRenderer`] could not be set up from its theme.
#[derive(Debug, thiserror::Error)]
pub enum AnsiError {
    /// The code theme does not load.
    #[error(transparent)]
    Highlight(#[from] HighlightError),
    /// The `overrides.ansi` file cannot be read or is invalid.
    #[error("cannot load the ANSI override {}: {message}", path.slashed())]
    Override { path: PathBuf, message: String },
}

/// A laid-out document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledDocument {
    pub lines: Vec<Line>,
    /// Warnings, such as diagrams shown as code, in document order.
    pub diagnostics: Vec<Diagnostic>,
    /// The document's path, for resolving relative links.
    pub source: Option<PathBuf>,
}

impl StyledDocument {
    /// The lines with escape sequences for a terminal with `caps`; see the
    /// [module docs](self#escapes).
    pub fn to_ansi(&self, caps: &Capabilities) -> String {
        escape::write(&self.lines, *caps, self.source.as_deref())
    }

    /// The lines as plain text, images as their alt text.
    pub fn plain_text(&self) -> String {
        self.to_ansi(&Capabilities::plain())
    }
}

/// Lays documents out as styled lines with one theme. See the [module docs](self).
#[derive(Debug, Clone)]
pub struct AnsiRenderer {
    options: AnsiOptions,
    highlighter: Highlighter,
    styles: Styles,
    /// Per heading level: whether the theme draws a line under it.
    heading_underline: [bool; 6],
    diagrams: Option<Arc<Registry>>,
}

impl AnsiRenderer {
    /// A renderer for `theme`: loads its code theme and its `overrides.ansi`.
    pub fn new(theme: &Theme, options: AnsiOptions) -> Result<Self, AnsiError> {
        let highlighter = Highlighter::new(&theme.code)?;
        let mut styles = Styles::from_theme(theme);
        if let Some(path) = &theme.overrides.ansi {
            let error = |message: String| AnsiError::Override {
                path: path.clone(),
                message,
            };
            let yaml = std::fs::read_to_string(path).map_err(|e| error(e.to_string()))?;
            styles
                .apply_overrides(&yaml)
                .map_err(|e| error(e.to_string()))?;
        }
        let heading_underline =
            [1, 2, 3, 4, 5, 6].map(|level| theme.headings.level(level).underline == Some(true));
        Ok(Self {
            options,
            highlighter,
            styles,
            heading_underline,
            diagrams: None,
        })
    }

    /// Renders diagrams with `registry`, which may be shared with other backends.
    #[must_use]
    pub fn with_diagrams(mut self, registry: impl Into<Arc<Registry>>) -> Self {
        self.diagrams = Some(registry.into());
        self
    }

    /// Lays out for `width` columns (at least 1) instead, e.g. after a terminal resize.
    #[must_use]
    pub fn with_width(mut self, width: usize) -> Self {
        self.options.width = width.max(1);
        self
    }

    /// Lays out with `options` instead (the width at least 1), e.g. with graphics for a
    /// terminal whose cell size is known only once it runs.
    #[must_use]
    pub fn with_options(mut self, options: AnsiOptions) -> Self {
        self.options = options;
        self.options.width = options.width.max(1);
        self
    }

    /// The options.
    pub const fn options(&self) -> &AnsiOptions {
        &self.options
    }

    /// `doc` as styled lines.
    pub fn render(&self, doc: &Document) -> StyledDocument {
        let (lines, diagnostics) = layout::document(self, doc);
        StyledDocument {
            lines,
            diagnostics,
            source: doc.path().map(PathBuf::from),
        }
    }
}
