//! [`KrokiRenderer`] (feature `kroki`): diagrams from a Kroki server.

use std::error::Error;
use std::time::Duration;

use reqwest::Client;
use reqwest::header::CONTENT_TYPE;

use super::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer, KROKI_LANGUAGES};

/// The diagram types Kroki renders to PNG; every type renders to SVG.
const PNG: &[&str] = &[
    "actdiag",
    "blockdiag",
    "c4plantuml",
    "ditaa",
    "dot",
    "erd",
    "graphviz",
    "mermaid",
    "nwdiag",
    "packetdiag",
    "plantuml",
    "rackdiag",
    "seqdiag",
    "structurizr",
    "tikz",
    "umlet",
    "vega",
    "vegalite",
    "wireviz",
];

/// The diagram types Kroki renders to text (ASCII art).
const TEXT: &[&str] = &["c4plantuml", "plantuml"];

/// A [`DiagramRenderer`] for the diagram types of Kroki (<https://kroki.io>): it POSTs
/// the source as `text/plain` to `<url>/<type>/<svg|png|txt>`, with `dot` sent as
/// `graphviz`. A response that is not a success fails with its status and body (Kroki
/// explains syntax errors there). Its fingerprint is the URL.
#[derive(Debug)]
pub struct KrokiRenderer {
    url: String,
    languages: Vec<String>,
    runtime: tokio::runtime::Runtime,
    client: Client,
}

impl KrokiRenderer {
    /// How long one request may take by default.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// A renderer for the Kroki server at `url` (e.g. `https://kroki.io`); the reason
    /// if its HTTP client cannot be set up.
    pub fn new(url: &str) -> Result<Self, String> {
        Self::with_timeout(url, Self::DEFAULT_TIMEOUT)
    }

    /// [`new`](Self::new) with a timeout for each request.
    pub fn with_timeout(url: &str, timeout: Duration) -> Result<Self, String> {
        // reqwest takes the process-wide rustls provider; ring, unless one is installed.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| err.to_string())?;
        let client = Client::builder()
            .user_agent(concat!("mrq/", env!("CARGO_PKG_VERSION")))
            .timeout(timeout)
            .build()
            .map_err(|err| describe(&err))?;
        Ok(Self {
            url: url.trim_end_matches('/').to_owned(),
            languages: KROKI_LANGUAGES
                .iter()
                .map(|&lang| lang.to_owned())
                .collect(),
            runtime,
            client,
        })
    }
}

impl DiagramRenderer for KrokiRenderer {
    fn id(&self) -> &'static str {
        "kroki"
    }

    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn formats(&self, lang: &str) -> &[DiagramFormat] {
        match (PNG.contains(&lang), TEXT.contains(&lang)) {
            (true, true) => &[DiagramFormat::Svg, DiagramFormat::Png, DiagramFormat::Text],
            (true, false) => &[DiagramFormat::Svg, DiagramFormat::Png],
            (false, true) => &[DiagramFormat::Svg, DiagramFormat::Text],
            (false, false) => &[DiagramFormat::Svg],
        }
    }

    fn fingerprint(&self) -> String {
        format!("kroki\0{}", self.url)
    }

    fn render(
        &self,
        lang: &str,
        src: &str,
        format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError> {
        let kind = if lang == "dot" { "graphviz" } else { lang };
        let suffix = match format {
            DiagramFormat::Svg => "svg",
            DiagramFormat::Png => "png",
            DiagramFormat::Text => "txt",
        };
        let url = format!("{}/{kind}/{suffix}", self.url);
        let request = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(src.to_owned());
        let (status, body) = self
            .runtime
            .block_on(async move {
                let response = request.send().await?;
                let status = response.status();
                Ok::<_, reqwest::Error>((status, response.bytes().await?))
            })
            .map_err(|err| DiagramError::Failed(format!("{url}: {}", describe(&err))))?;
        if !status.is_success() {
            let body = String::from_utf8_lossy(&body);
            let body: String = body.trim().chars().take(500).collect();
            return Err(DiagramError::Failed(if body.is_empty() {
                format!("{url}: {status}")
            } else {
                format!("{url}: {status}: {body}")
            }));
        }
        DiagramOutput::from_bytes(format, body.to_vec())
            .map_err(|reason| DiagramError::Failed(format!("{url}: {reason}")))
    }
}

/// A short reason for `err`.
fn describe(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "timed out".to_owned();
    }
    let mut cause: &dyn Error = err;
    while let Some(source) = cause.source() {
        cause = source;
    }
    if err.is_connect() {
        format!("connection failed: {cause}")
    } else {
        cause.to_string()
    }
}
