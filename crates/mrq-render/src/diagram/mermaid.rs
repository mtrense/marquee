//! [`MermaidRenderer`]: Mermaid through merman, in pure Rust, without Node.js.

use std::fmt::Write as _;

use merman::ascii::{AsciiError, AsciiRenderOptions};
use merman::svg::{SvgPipeline, SvgRenderOptions};
use merman::{
    AsciiRequest, OperationControl, ParseOptions, RenderError, RenderOutput, RenderRequest,
    Renderer, SvgRequest,
};
use ring::digest::{SHA256, digest};

use super::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer};

/// The merman version, for the fingerprint; keep it in step with Cargo.toml.
const MERMAN: &str = "0.9.3";

/// A [`DiagramRenderer`] for `mermaid` with merman: SVG, text (Unicode box drawing, or
/// plain ASCII with [`with_ascii`](Self::with_ascii)) and, with the `png` feature, PNG
/// (resvg-safe SVG [rasterized](super::rasterize)). Text measurement is merman's
/// deterministic one, so the output does not depend on the installed fonts.
///
/// Each SVG gets the id `mrq-` and a hash of its source, so several diagrams can sit
/// inline in one HTML page without their ids and scoped styles clashing. The SVG uses
/// Mermaid's HTML labels (`<foreignObject>`), which browsers show but resvg and Typst do
/// not; [`with_resvg_safe_svg`](Self::with_resvg_safe_svg) draws labels as SVG text
/// instead. Parsing is strict: a syntax error [fails](DiagramError::Failed), and so does
/// a source that is no Mermaid diagram.
#[derive(Debug, Clone)]
pub struct MermaidRenderer {
    languages: Vec<String>,
    formats: Vec<DiagramFormat>,
    ascii: bool,
    resvg_safe: bool,
    renderer: Renderer,
}

impl MermaidRenderer {
    pub fn new() -> Self {
        let mut formats = vec![DiagramFormat::Svg];
        if cfg!(feature = "png") {
            formats.push(DiagramFormat::Png);
        }
        formats.push(DiagramFormat::Text);
        Self {
            languages: vec!["mermaid".to_owned()],
            formats,
            ascii: false,
            resvg_safe: false,
            renderer: Renderer::new().with_parse_options(ParseOptions::strict()),
        }
    }

    /// Text with ASCII characters only (`+`, `-`, `|`) instead of box drawing.
    #[must_use]
    pub const fn with_ascii(mut self, ascii: bool) -> Self {
        self.ascii = ascii;
        self
    }

    /// SVG without `<foreignObject>` labels, for consumers that are not browsers, such as
    /// resvg and Typst. PNG always uses it.
    #[must_use]
    pub const fn with_resvg_safe_svg(mut self, resvg_safe: bool) -> Self {
        self.resvg_safe = resvg_safe;
        self
    }

    /// `src` as SVG, resvg-safe if asked for.
    fn svg(&self, src: &str, resvg_safe: bool) -> Result<String, DiagramError> {
        let request = SvgRequest {
            options: SvgRenderOptions {
                diagram_id: Some(diagram_id(src)),
                ..SvgRenderOptions::default()
            },
            pipeline: resvg_safe.then(SvgPipeline::resvg_safe),
            ..SvgRequest::default()
        };
        let output = self
            .renderer
            .render(RenderRequest::svg(src, OperationControl::new(), request))
            .map_err(|e| error(&e))?;
        match output {
            RenderOutput::Svg(Some(svg)) => Ok(svg.into_parts().0),
            _ => Err(no_diagram()),
        }
    }

    /// `src` as text.
    fn text(&self, src: &str) -> Result<String, DiagramError> {
        let options = if self.ascii {
            AsciiRenderOptions::ascii()
        } else {
            AsciiRenderOptions::unicode()
        };
        let request = AsciiRequest {
            options,
            ..AsciiRequest::default()
        };
        let output = self
            .renderer
            .render(RenderRequest::ascii(src, OperationControl::new(), request))
            .map_err(|e| error(&e))?;
        match output {
            RenderOutput::Ascii(Some(ascii)) => Ok(ascii.text),
            _ => Err(no_diagram()),
        }
    }
}

impl Default for MermaidRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagramRenderer for MermaidRenderer {
    fn id(&self) -> &'static str {
        "builtin:mermaid"
    }

    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn formats(&self, _lang: &str) -> &[DiagramFormat] {
        &self.formats
    }

    fn fingerprint(&self) -> String {
        format!(
            "{} merman {MERMAN} mrq {} ascii={} resvg-safe={}",
            self.id(),
            env!("CARGO_PKG_VERSION"),
            self.ascii,
            self.resvg_safe
        )
    }

    fn render(
        &self,
        _lang: &str,
        src: &str,
        format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError> {
        let bytes = match format {
            DiagramFormat::Svg => self.svg(src, self.resvg_safe)?.into_bytes(),
            #[cfg(feature = "png")]
            DiagramFormat::Png => super::rasterize(&self.svg(src, true)?, 1.0)?,
            #[cfg(not(feature = "png"))]
            DiagramFormat::Png => {
                return Err(DiagramError::Unavailable(
                    "mrq was built without the `png` feature".to_owned(),
                ));
            }
            DiagramFormat::Text => self.text(src)?.into_bytes(),
        };
        DiagramOutput::from_bytes(format, bytes).map_err(DiagramError::Failed)
    }
}

/// `mrq-` and the first 12 hex digits of the SHA-256 of `src`.
fn diagram_id(src: &str) -> String {
    let mut id = "mrq-".to_owned();
    for byte in &digest(&SHA256, src.as_bytes()).as_ref()[..6] {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

fn no_diagram() -> DiagramError {
    DiagramError::Failed("no Mermaid diagram detected".to_owned())
}

/// A merman error as a diagram error: a diagram type that has no text rendering is
/// unavailable, anything else failed.
fn error(error: &RenderError) -> DiagramError {
    match error {
        RenderError::Ascii(
            AsciiError::UnsupportedDiagram { .. } | AsciiError::UnsupportedFeature { .. },
        )
        | RenderError::UnsupportedTarget(_) => DiagramError::Unavailable(error.to_string()),
        _ => DiagramError::Failed(error.to_string()),
    }
}
