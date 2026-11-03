//! The PDF backend (feature `pdf`): the mrq AST as Typst markup, compiled in-process by
//! Typst to PDF with the fonts bundled in the binary (`mrq render -o pdf -s paper`).
//!
//! # Two steps
//!
//! 1. [`PdfRenderer::typst`] writes a [`TypstDocument`]: the Typst source (a preamble
//!    from the theme, then the body after a `// --- body` line), the files it names
//!    (local images and rendered diagrams, read at this point) and the warnings so far.
//!    It is plain text, so tests snapshot it.
//! 2. [`TypstDocument::compile`] compiles it with Typst and exports the PDF.
//!
//! [`PdfRenderer::render`] and [`render_with_diagnostics`](PdfRenderer::render_with_diagnostics)
//! do both.
//!
//! # Offline and reproducible
//!
//! Typst sees only the generated source, the files it names (mounted under `/assets/`),
//! the `overrides.pdf` template's directory (under `/template/`) and the fonts bundled
//! with Typst ([`bundled_font_families`]: Libertinus Serif, New Computer Modern, New
//! Computer Modern Math and `DejaVu Sans Mono`), which are embedded (subset) in every PDF.
//! System fonts are not used, and `@preview` packages cannot be imported, so a render
//! never needs the network and gives the same bytes on every machine. The PDF has no
//! creation date; `datetime.today()` is [`PdfOptions::today`].
//!
//! # Theme
//!
//! The preamble maps the [`Theme`]: page fill, body font, size and color, the leading
//! from the line height, heading fonts, colors and per-level size, weight, italic and
//! underline (a rule under the heading), link and code colors, table and rule strokes,
//! muted blockquotes and footnotes, alert colors. Font lists keep only the bundled
//! families, then fall back to Libertinus Serif (body, headings) or `DejaVu Sans Mono`
//! (code). A list that names no bundled family and no generic one (`serif`,
//! `system-ui`, `ui-monospace`, …) is reported as [`codes::FONT`].
//!
//! ## The `overrides.pdf` template
//!
//! The theme's `overrides.pdf` is a Typst file that must define a function `template`
//! taking the document body. It is applied as `#show: template` after the theme's
//! rules, so its own set and show rules win. The file is mounted at
//! `/template/<name>`, so it can `read`, `image` and `#import` files next to it (and
//! below); nothing outside that directory is reachable. `sys.inputs.title` is the
//! document title (frontmatter `title`, else the first H1, else the file stem) and
//! `sys.inputs.source` the document's path (absent for stdin).
//!
//! ```typst
//! #let template(body) = {
//!   set page(header: align(right, sys.inputs.at("title", default: "")))
//!   body
//! }
//! ```
//!
//! # Blocks
//!
//! - **Headings** are `#heading` with the slugger's anchor as label, so `[text](#anchor)`
//!   links to them; links to missing anchors are plain text.
//! - **Code blocks** are highlighted with the code theme ([`Highlighter`]): each span
//!   keeps its color, bold, italic and underline (span backgrounds are dropped) on the
//!   block's background; plain code uses `colors.code_*`. Diagrams go through the
//!   [`Registry`] ([`PdfRenderer::with_diagrams`]) as SVG or PNG; a failure shows the
//!   code with a [`codes::DIAGRAM`](crate::diagram::codes::DIAGRAM) warning. Register
//!   Mermaid as `MermaidRenderer::new().with_resvg_safe_svg(true)`: Typst cannot draw
//!   `<foreignObject>` labels.
//! - **Lists** are `#list`/`#enum` (start and `1)` numbering kept); task items start
//!   with ☐ or ☑ (a list of only tasks has no bullets). **Tables** are `#table` with the
//!   column alignments and a bold header row. **Definition lists** are `#terms`.
//! - **Block quotes** are indented with a bar; **alerts, callouts and admonitions** have
//!   a bar and a bold title in the kind's alert color (the kind capitalized by default;
//!   an Obsidian title is Markdown; an mkdocs `""` title has none); folds always show
//!   their body.
//! - **Footnotes** are Typst footnotes (numbered by first reference, at the page bottom);
//!   unreferenced definitions are left out, undefined references show as `[^label]`.
//! - Raw HTML, comments, frontmatter, link and abbreviation definitions and `mrq:`
//!   directives render nothing. Thematic breaks are a rule.
//!
//! # Inlines
//!
//! Text is escaped (every ASCII punctuation character that could be markup gets a
//! backslash, so quotes stay straight and URLs in text are not linked); formatting uses
//! function calls (`#emph`, `#strong`, `#strike`, `#highlight`, `#super`, `#sub`), code
//! `#raw("…")` with an escaped string. Images are local files next to the document
//! (PNG, JPEG, GIF, WebP, SVG), at most as wide as the text; remote or missing images
//! show `[alt]` with a [`codes::IMAGE`] warning. Obsidian image embeds are images, other
//! embeds and wikilinks their text (a wikilink to a heading of the same note links to
//! it), tags `#tag` in the accent color.
//!
//! **Math** is converted from LaTeX to Typst math ([`tex_to_typst`]) for the common
//! subset; anything else shows its TeX source as code with a [`codes::MATH`] warning.

mod escape;
mod markup;
mod math;
mod preamble;
mod world;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mrq_core::Document;
use mrq_core::Slashed as _;
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::source::ByteRange;

use crate::diagram::Registry;
use crate::highlight::{HighlightError, Highlighter};
use crate::theme::Theme;

pub use math::tex_to_typst;
pub use world::bundled_font_families;

use world::{Asset, MrqWorld};

/// The codes of the PDF backend's diagnostics (diagrams use
/// [`diagram::codes::DIAGRAM`](crate::diagram::codes::DIAGRAM)).
pub mod codes {
    /// A theme font list names no bundled font; the bundled fallback is used.
    pub const FONT: &str = "pdf-font";
    /// An image is remote, missing or not a supported format; its alt text is shown.
    pub const IMAGE: &str = "pdf-image";
    /// Math that the LaTeX-to-Typst converter does not handle; its TeX is shown.
    pub const MATH: &str = "pdf-math";
    /// A warning from the Typst compiler.
    pub const TYPST: &str = "pdf-typst";
}

/// How [`PdfRenderer`] renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOptions {
    /// The Typst paper size, e.g. `a4`, `us-letter`.
    pub paper: String,
    /// Highlight code blocks with a known language.
    pub highlight: bool,
    /// What `datetime.today()` gives in a template: year, month, day. `None` makes it an
    /// error, so output never depends on the clock unless asked to.
    pub today: Option<(i32, u8, u8)>,
}

/// A4, highlighting on, no date.
impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            paper: "a4".to_owned(),
            highlight: true,
            today: None,
        }
    }
}

/// A [`PdfRenderer`] could not be set up, or a document did not compile.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// The code theme does not load.
    #[error(transparent)]
    Highlight(#[from] HighlightError),
    /// The `overrides.pdf` template cannot be read.
    #[error("cannot read the PDF override {}: {source}", path.slashed())]
    Override {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Typst reported errors (in the template, or a file it could not use).
    #[error("Typst failed to compile the document: {}", messages.join("; "))]
    Compile { messages: Vec<String> },
}

/// The `overrides.pdf` template, read once.
#[derive(Debug, Clone)]
struct Template {
    /// Its virtual path, `/template/<file name>`.
    vpath: String,
    /// Its directory on disk.
    dir: PathBuf,
}

/// Renders documents to PDF with one theme. See the [module docs](self).
#[derive(Clone)]
pub struct PdfRenderer {
    options: PdfOptions,
    theme: Theme,
    highlighter: Highlighter,
    template: Option<Template>,
    diagrams: Option<Arc<Registry>>,
}

impl fmt::Debug for PdfRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PdfRenderer")
            .field("options", &self.options)
            .field("template", &self.template)
            .finish_non_exhaustive()
    }
}

/// A document as Typst markup, ready to compile; see the [module docs](self#two-steps).
#[derive(Debug, Clone)]
pub struct TypstDocument {
    /// The Typst source: the theme preamble, `// --- body`, the body.
    pub source: String,
    /// Warnings so far: fonts, images, math, diagrams.
    pub diagnostics: Vec<Diagnostic>,
    /// The diagnostics' file name for the document.
    file: PathBuf,
    /// Virtual path → content, for images and diagrams.
    assets: HashMap<String, Asset>,
    template_dir: Option<PathBuf>,
    inputs: Vec<(&'static str, String)>,
    today: Option<(i32, u8, u8)>,
}

/// What [`PdfRenderer::render_with_diagnostics`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOutput {
    /// The PDF file.
    pub pdf: Vec<u8>,
    /// Warnings of the markup step ([`TypstDocument::diagnostics`]) and of Typst
    /// ([`codes::TYPST`]), in that order.
    pub diagnostics: Vec<Diagnostic>,
}

impl PdfRenderer {
    /// A renderer for `theme`: loads its code theme and checks that its `overrides.pdf`
    /// can be read.
    pub fn new(theme: &Theme, options: PdfOptions) -> Result<Self, PdfError> {
        let highlighter = Highlighter::new(&theme.code)?;
        let template = match &theme.overrides.pdf {
            Some(path) => {
                std::fs::metadata(path)
                    .and_then(|meta| {
                        if meta.is_file() {
                            Ok(())
                        } else {
                            Err(std::io::Error::other("not a file"))
                        }
                    })
                    .map_err(|source| PdfError::Override {
                        path: path.clone(),
                        source,
                    })?;
                let name = path
                    .file_name()
                    .map_or_else(|| "template.typ".into(), |n| n.to_string_lossy());
                Some(Template {
                    vpath: format!("{}/{name}", world::TEMPLATE_DIR),
                    dir: path
                        .parent()
                        .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
                })
            }
            None => None,
        };
        Ok(Self {
            options,
            theme: theme.clone(),
            highlighter,
            template,
            diagrams: None,
        })
    }

    /// Renders diagrams with `registry`, which may be shared with other backends.
    #[must_use]
    pub fn with_diagrams(mut self, registry: impl Into<Arc<Registry>>) -> Self {
        self.diagrams = Some(registry.into());
        self
    }

    /// The options.
    pub const fn options(&self) -> &PdfOptions {
        &self.options
    }

    /// `doc` as Typst markup, with the files it names; see the
    /// [module docs](self#two-steps).
    pub fn typst(&self, doc: &Document) -> TypstDocument {
        markup::document(self, doc)
    }

    /// `doc` as a PDF file. Warnings are dropped;
    /// [`render_with_diagnostics`](Self::render_with_diagnostics) keeps them.
    pub fn render(&self, doc: &Document) -> Result<Vec<u8>, PdfError> {
        Ok(self.render_with_diagnostics(doc)?.pdf)
    }

    /// `doc` as a PDF file, like [`render`](Self::render), and the warnings.
    pub fn render_with_diagnostics(&self, doc: &Document) -> Result<PdfOutput, PdfError> {
        self.typst(doc).compile()
    }
}

impl TypstDocument {
    /// Compiles the source with Typst and exports the PDF.
    pub fn compile(&self) -> Result<PdfOutput, PdfError> {
        let world = MrqWorld::new(
            self.source.clone(),
            self.assets.clone(),
            self.template_dir.clone(),
            self.inputs.clone(),
            self.today,
        );
        let warned = typst::compile::<typst_layout::PagedDocument>(&world);
        let messages = |errors: &[typst::diag::SourceDiagnostic]| -> Vec<String> {
            errors.iter().map(message).collect()
        };
        let document = warned.output.map_err(|errors| PdfError::Compile {
            messages: messages(&errors),
        })?;
        let options = typst_pdf::PdfOptions {
            ident: typst::foundations::Smart::Custom("mrq".to_owned()),
            ..typst_pdf::PdfOptions::default()
        };
        let pdf = typst_pdf::pdf(&document, &options).map_err(|errors| PdfError::Compile {
            messages: messages(&errors),
        })?;
        let mut diagnostics = self.diagnostics.clone();
        for warning in &warned.warnings {
            diagnostics.push(Diagnostic::new(
                codes::TYPST,
                Severity::Warning,
                &self.file,
                ByteRange::new(0, 0),
                message(warning),
            ));
        }
        Ok(PdfOutput { pdf, diagnostics })
    }
}

/// A Typst diagnostic's message with its hints.
fn message(diagnostic: &typst::diag::SourceDiagnostic) -> String {
    let mut message = diagnostic.message.to_string();
    for hint in &diagnostic.hints {
        message.push_str(" (hint: ");
        message.push_str(&hint.v);
        message.push(')');
    }
    message
}
