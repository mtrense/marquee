//! Diagrams: fenced code blocks whose language names a diagram type, rendered to SVG, PNG
//! or text by [`DiagramRenderer`]s that a [`Registry`] tries in order.
//!
//! # Order
//!
//! [`Registry::from_config`] sets up the order of the README; for each language the
//! first renderer that succeeds wins:
//!
//! | Language | Renderers |
//! | --- | --- |
//! | any with a `render.diagrams.commands` entry | that command first |
//! | `mermaid` | built in (M8.5) → `mmdc` → Kroki |
//! | `dot`, `graphviz` | `dot` → built in (M8.5) → Kroki |
//! | `d2` | `d2` → Kroki |
//! | `plantuml` | `plantuml` → Kroki |
//! | other Kroki types ([`KROKI_LANGUAGES`]) | Kroki |
//!
//! Kroki is only registered when `render.diagrams.kroki.enabled` is set, and it needs
//! the `kroki` feature; when the feature is missing, an enabled Kroki fails with that
//! reason. Languages match ignoring ASCII case. A command that is not installed is
//! [unavailable](DiagramError::Unavailable) and skipped like any failure.
//!
//! # Formats
//!
//! Each backend asks for the [`DiagramFormat`]s it can show, best first (HTML: SVG, PNG,
//! text; a terminal may want text first). Each renderer in turn tries the requested
//! formats it [offers](DiagramRenderer::formats), in the caller's order. Outputs are
//! checked before use: SVG must be UTF-8 and contain `<svg`, PNG must start with the PNG
//! signature, text must be UTF-8 and not empty.
//!
//! # Cache
//!
//! With a [`DiagramCache`] ([`Registry::with_cache`]) every successful render is stored
//! under a SHA-256 of the renderer's [fingerprint](DiagramRenderer::fingerprint) (its id
//! and options, such as a command line or the Kroki URL), the language, the format and
//! the source, and found there on the next render of the same diagram, before the
//! renderer runs. Failures are never cached. The default directory is
//! `<user cache>/diagrams` ([`DiagramCache::in_user_cache`]; `$MRQ_CACHE_HOME`, then
//! `$XDG_CACHE_HOME/mrq`, then `~/.cache/mrq`). Entries are never pruned.
//!
//! # Failures
//!
//! [`Registry::render`] returns `None` for a language no renderer handles (the block is
//! plain code), and a [`RenderFailure`] listing every attempt when all renderers fail.
//! Backends then show the original code block and report a [`codes::DIAGRAM`] warning
//! with the block's span: the HTML backend through [`DiagramHook`] and
//! [`HtmlRenderer::render_with_diagnostics`](crate::html::HtmlRenderer::render_with_diagnostics).

mod cache;
mod command;
mod html;
#[cfg(feature = "kroki")]
mod kroki;

use std::fmt;
use std::sync::Arc;

use mrq_core::config::{DiagramsConfig, LoadedConfig};

pub use cache::DiagramCache;
pub use command::CommandRenderer;
pub use html::DiagramHook;
#[cfg(feature = "kroki")]
pub use kroki::KrokiRenderer;

/// Diagnostic codes of the diagram framework.
pub mod codes {
    /// A diagram that no renderer could render; it is shown as its code block.
    pub const DIAGRAM: &str = "diagram";
}

/// The diagram types of Kroki (<https://kroki.io>), and `dot` for `graphviz`.
pub const KROKI_LANGUAGES: &[&str] = &[
    "actdiag",
    "blockdiag",
    "bpmn",
    "bytefield",
    "c4plantuml",
    "d2",
    "dbml",
    "ditaa",
    "dot",
    "erd",
    "excalidraw",
    "graphviz",
    "mermaid",
    "nomnoml",
    "nwdiag",
    "packetdiag",
    "pikchr",
    "plantuml",
    "rackdiag",
    "seqdiag",
    "structurizr",
    "svgbob",
    "symbolator",
    "tikz",
    "umlet",
    "vega",
    "vegalite",
    "wavedrom",
    "wireviz",
];

/// What a diagram is rendered to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagramFormat {
    /// An SVG document (inline in HTML, embedded in PDF, rasterized for terminals).
    Svg,
    /// A PNG image.
    Png,
    /// Text, such as an ASCII-art diagram, for terminals and plain output.
    Text,
}

impl DiagramFormat {
    /// `svg`, `png` or `text`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Text => "text",
        }
    }

    /// The file extension of cache entries: `svg`, `png` or `txt`.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Text => "txt",
        }
    }
}

impl fmt::Display for DiagramFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A rendered diagram.
#[derive(Clone, PartialEq, Eq)]
pub enum DiagramOutput {
    Svg(String),
    Png(Vec<u8>),
    Text(String),
}

impl DiagramOutput {
    /// The format of the output.
    pub const fn format(&self) -> DiagramFormat {
        match self {
            Self::Svg(_) => DiagramFormat::Svg,
            Self::Png(_) => DiagramFormat::Png,
            Self::Text(_) => DiagramFormat::Text,
        }
    }

    /// The output as bytes, as stored in the cache.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Svg(text) | Self::Text(text) => text.as_bytes(),
            Self::Png(bytes) => bytes,
        }
    }

    /// `bytes` as output in `format`, if they look like it (see the
    /// [module docs](self#formats)); the reason if not.
    pub fn from_bytes(format: DiagramFormat, bytes: Vec<u8>) -> Result<Self, String> {
        const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
        let text = |bytes: Vec<u8>| {
            String::from_utf8(bytes).map_err(|_| format!("the {format} output is not UTF-8"))
        };
        match format {
            DiagramFormat::Svg => {
                let svg = text(bytes)?;
                if svg.contains("<svg") {
                    Ok(Self::Svg(svg))
                } else {
                    Err("the output is not SVG".to_owned())
                }
            }
            DiagramFormat::Png => {
                if bytes.starts_with(PNG_SIGNATURE) {
                    Ok(Self::Png(bytes))
                } else {
                    Err("the output is not PNG".to_owned())
                }
            }
            DiagramFormat::Text => {
                let text = text(bytes)?;
                if text.trim().is_empty() {
                    Err("the output is empty".to_owned())
                } else {
                    Ok(Self::Text(text))
                }
            }
        }
    }
}

/// PNG bytes would flood test failures and logs; their length is enough.
impl fmt::Debug for DiagramOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Svg(svg) => f.debug_tuple("Svg").field(svg).finish(),
            Self::Png(bytes) => write!(f, "Png({} bytes)", bytes.len()),
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
        }
    }
}

/// Why a renderer produced no diagram.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiagramError {
    /// The renderer cannot run here: a command that is not installed, a format it does
    /// not offer, a feature that was not built.
    #[error("{0}")]
    Unavailable(String),
    /// It ran and failed, e.g. on a syntax error in the source.
    #[error("{0}")]
    Failed(String),
}

/// Renders diagrams of some languages to some formats.
///
/// Built-in renderers, external commands ([`CommandRenderer`]) and Kroki
/// (`KrokiRenderer`, feature `kroki`) implement it; a [`Registry`] tries them in order.
pub trait DiagramRenderer: Send + Sync {
    /// A short name for messages, such as `command:dot` or `kroki`.
    fn id(&self) -> &str;

    /// The languages it renders, in ASCII lowercase.
    fn languages(&self) -> &[String];

    /// The formats it can render `lang` (lowercase) to.
    fn formats(&self, lang: &str) -> &[DiagramFormat];

    /// What its output depends on besides the language, format and source, for the
    /// cache key: the id and options (a command line, a server URL, a version). The id
    /// by default.
    fn fingerprint(&self) -> String {
        self.id().to_owned()
    }

    /// `src`, a diagram in `lang` (lowercase), rendered to `format`.
    fn render(
        &self,
        lang: &str,
        src: &str,
        format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError>;
}

/// A successful [`Registry::render`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub output: DiagramOutput,
    /// The [`id`](DiagramRenderer::id) of the renderer that made it.
    pub renderer: String,
    /// Whether it came from the cache.
    pub cached: bool,
}

/// One renderer's failure in a [`RenderFailure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The [`id`](DiagramRenderer::id) of the renderer.
    pub renderer: String,
    pub error: DiagramError,
}

/// Every renderer of a language failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFailure {
    /// The language, lowercase.
    pub lang: String,
    /// What each renderer said, in order.
    pub attempts: Vec<Attempt>,
}

impl fmt::Display for RenderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, attempt) in self.attempts.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{}: {}", attempt.renderer, attempt.error)?;
        }
        Ok(())
    }
}

impl std::error::Error for RenderFailure {}

/// The diagram renderers, in the order they are tried, and an optional cache. See the
/// [module docs](self).
#[derive(Default, Clone)]
pub struct Registry {
    renderers: Vec<Arc<dyn DiagramRenderer>>,
    cache: Option<DiagramCache>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ids: Vec<&str> = self.renderers.iter().map(|r| r.id()).collect();
        f.debug_struct("Registry")
            .field("renderers", &ids)
            .field("cache", &self.cache)
            .finish()
    }
}

impl Registry {
    /// No renderers and no cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The renderers of the README's order (see the [module docs](self#order)) for
    /// `config`: its commands, the built-in renderers, the well-known commands and Kroki
    /// when enabled. Relative program paths are relative to the current directory;
    /// [`from_loaded`](Self::from_loaded) resolves them against their config file.
    pub fn from_config(config: &DiagramsConfig) -> Self {
        let mut registry = Self::new();
        for (lang, argv) in &config.commands {
            registry = registry.with_renderer(CommandRenderer::configured(lang, argv.clone()));
        }
        // M8.5: the built-in Mermaid renderer (merman, feature `mermaid`) goes here.
        registry = registry.with_renderer(CommandRenderer::mmdc());
        registry = registry.with_renderer(CommandRenderer::dot());
        // M8.5: the built-in DOT renderer (layout-rs) goes here.
        registry = registry.with_renderer(CommandRenderer::d2());
        registry = registry.with_renderer(CommandRenderer::plantuml());
        if config.kroki.enabled {
            registry.renderers.push(kroki(&config.kroki.url));
        }
        registry
    }

    /// [`from_config`](Self::from_config) for `render.diagrams` of `loaded`, with a
    /// relative program path that names a directory (`./tools/bob.sh`, not `svgbob`)
    /// resolved against the directory of the config file that declares it.
    pub fn from_loaded(loaded: &LoadedConfig) -> Self {
        let mut config = loaded.config().render.diagrams.clone();
        for (lang, argv) in &mut config.commands {
            mrq_core::process::resolve_program(
                loaded,
                &["render", "diagrams", "commands", lang.as_str()],
                argv,
            );
        }
        Self::from_config(&config)
    }

    /// Adds `renderer` after the others.
    #[must_use]
    pub fn with_renderer(mut self, renderer: impl DiagramRenderer + 'static) -> Self {
        self.renderers.push(Arc::new(renderer));
        self
    }

    /// Caches renders in `cache`.
    #[must_use]
    pub fn with_cache(mut self, cache: DiagramCache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// The cache, if any.
    pub const fn cache(&self) -> Option<&DiagramCache> {
        self.cache.as_ref()
    }

    /// The ids of the renderers for `lang`, in the order they are tried.
    pub fn renderers_for(&self, lang: &str) -> Vec<&str> {
        let lang = lang.to_ascii_lowercase();
        self.candidates(&lang)
            .into_iter()
            .map(DiagramRenderer::id)
            .collect()
    }

    /// Whether `lang` is a diagram language: some renderer handles it.
    pub fn handles(&self, lang: &str) -> bool {
        let lang = lang.to_ascii_lowercase();
        !self.candidates(&lang).is_empty()
    }

    /// `src`, a diagram in `lang`, rendered to the first format of `accept` that the
    /// first renderer able to render it offers (see the [module docs](self)); `None`
    /// when no renderer handles `lang`.
    pub fn render(
        &self,
        lang: &str,
        src: &str,
        accept: &[DiagramFormat],
    ) -> Option<Result<Rendered, RenderFailure>> {
        let lang = lang.to_ascii_lowercase();
        let candidates = self.candidates(&lang);
        if candidates.is_empty() {
            return None;
        }
        let mut attempts = Vec::new();
        for renderer in candidates {
            let offered = renderer.formats(&lang);
            let formats: Vec<DiagramFormat> = accept
                .iter()
                .copied()
                .filter(|format| offered.contains(format))
                .collect();
            if formats.is_empty() {
                let wanted: Vec<&str> = accept.iter().map(|format| format.as_str()).collect();
                attempts.push(Attempt {
                    renderer: renderer.id().to_owned(),
                    error: DiagramError::Unavailable(format!("renders no {}", wanted.join(" or "))),
                });
                continue;
            }
            for format in formats {
                let key = self
                    .cache
                    .as_ref()
                    .map(|_| cache::key(&renderer.fingerprint(), &lang, format, src));
                if let (Some(cache), Some(key)) = (&self.cache, &key)
                    && let Some(output) = cache.get(key, format)
                {
                    return Some(Ok(Rendered {
                        output,
                        renderer: renderer.id().to_owned(),
                        cached: true,
                    }));
                }
                match renderer.render(&lang, src, format) {
                    Ok(output) => {
                        if let (Some(cache), Some(key)) = (&self.cache, &key) {
                            cache.put(key, &output);
                        }
                        return Some(Ok(Rendered {
                            output,
                            renderer: renderer.id().to_owned(),
                            cached: false,
                        }));
                    }
                    Err(error) => attempts.push(Attempt {
                        renderer: renderer.id().to_owned(),
                        error,
                    }),
                }
            }
        }
        Some(Err(RenderFailure { lang, attempts }))
    }

    /// The renderers for `lang` (lowercase), in order.
    fn candidates(&self, lang: &str) -> Vec<&dyn DiagramRenderer> {
        self.renderers
            .iter()
            .map(|renderer| &**renderer)
            .filter(|renderer| renderer.languages().iter().any(|l| l == lang))
            .collect()
    }
}

/// The Kroki renderer at `url`, or one that says why there is none.
fn kroki(url: &str) -> Arc<dyn DiagramRenderer> {
    #[cfg(feature = "kroki")]
    let reason = match KrokiRenderer::new(url) {
        Ok(renderer) => return Arc::new(renderer),
        Err(reason) => reason,
    };
    #[cfg(not(feature = "kroki"))]
    let reason = {
        let _ = url;
        "mrq was built without the `kroki` feature".to_owned()
    };
    Arc::new(Unavailable {
        id: "kroki".to_owned(),
        languages: KROKI_LANGUAGES
            .iter()
            .map(|&lang| lang.to_owned())
            .collect(),
        reason,
    })
}

/// A renderer that cannot run, so that its failure explains why.
struct Unavailable {
    id: String,
    languages: Vec<String>,
    reason: String,
}

impl DiagramRenderer for Unavailable {
    fn id(&self) -> &str {
        &self.id
    }

    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn formats(&self, _lang: &str) -> &[DiagramFormat] {
        &[DiagramFormat::Svg, DiagramFormat::Png, DiagramFormat::Text]
    }

    fn render(
        &self,
        _lang: &str,
        _src: &str,
        _format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError> {
        Err(DiagramError::Unavailable(self.reason.clone()))
    }
}
