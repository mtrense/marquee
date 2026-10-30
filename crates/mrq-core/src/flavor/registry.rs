//! The flavor registry and flavor resolution.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::{Commonmark, DEFAULT_FLAVOR, Flavor, Gfm};
use crate::ast::Span;
use crate::diag::{Diagnostic, Severity};
use crate::frontmatter::Frontmatter;

/// Diagnostic code of an unknown or malformed flavor choice.
pub const FLAVOR_ERROR_CODE: &str = "flavor";

/// Frontmatter key path that picks a document's flavor.
pub const FRONTMATTER_KEY: [&str; 2] = ["mrq", "flavor"];

/// Where a [resolved](Registry::resolve) flavor was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlavorOrigin {
    /// The `--flavor` flag.
    Cli,
    /// The document's frontmatter key `mrq.flavor`.
    Frontmatter,
    /// The `flavor` key of the configuration.
    Config,
    /// Nothing chose one: [`DEFAULT_FLAVOR`].
    Default,
}

/// Why no flavor could be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FlavorError {
    /// The chosen name is not registered.
    #[error("unknown flavor `{name}` (known flavors: {})", known.join(", "))]
    Unknown {
        name: String,
        /// The registered names, in registration order.
        known: Vec<&'static str>,
        origin: FlavorOrigin,
        /// The frontmatter value, when the name came from there.
        span: Option<Span>,
    },
    /// Frontmatter `mrq.flavor` is not a string.
    #[error("frontmatter `mrq.flavor` must be a string")]
    NotAString {
        /// The frontmatter value.
        span: Span,
    },
}

impl FlavorError {
    /// The frontmatter value at fault; `None` for flavors chosen outside the document.
    pub const fn span(&self) -> Option<Span> {
        match self {
            Self::Unknown { span, .. } => *span,
            Self::NotAString { span } => Some(*span),
        }
    }

    /// This error as a diagnostic about `file`, at the frontmatter value if there is one
    /// (else at the start of the file).
    pub fn to_diagnostic(&self, file: impl Into<PathBuf>) -> Diagnostic {
        Diagnostic::new(
            FLAVOR_ERROR_CODE,
            Severity::Error,
            file,
            self.span().unwrap_or(Span::new(0, 0)),
            self.to_string(),
        )
    }
}

/// A flavor picked by [`Registry::resolve`], and where it was picked.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub flavor: Arc<dyn Flavor>,
    pub origin: FlavorOrigin,
}

/// The available flavors, by name.
///
/// [`Registry::builtin`] (also the [`Default`]) holds every built-in flavor.
#[derive(Debug, Clone)]
pub struct Registry {
    flavors: Vec<Arc<dyn Flavor>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Registry {
    /// An empty registry.
    pub const fn new() -> Self {
        Self {
            flavors: Vec::new(),
        }
    }

    /// The built-in flavors: `commonmark` and `gfm`.
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(Commonmark));
        registry.register(Arc::new(Gfm));
        registry
    }

    /// Adds `flavor`, replacing (and returning) a registered flavor of the same name,
    /// which keeps its position.
    pub fn register(&mut self, flavor: Arc<dyn Flavor>) -> Option<Arc<dyn Flavor>> {
        if let Some(slot) = self.flavors.iter_mut().find(|f| f.name() == flavor.name()) {
            return Some(std::mem::replace(slot, flavor));
        }
        self.flavors.push(flavor);
        None
    }

    /// The flavor called `name`.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Flavor>> {
        self.flavors.iter().find(|f| f.name() == name).cloned()
    }

    /// The registered names, in registration order.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.flavors.iter().map(|f| f.name())
    }

    /// Picks a document's flavor: the first of `cli` (`--flavor`), the frontmatter key
    /// `mrq.flavor`, `config` (the configured `flavor`) and [`DEFAULT_FLAVOR`].
    ///
    /// A name that is chosen must be registered; later sources are not consulted then.
    /// Malformed frontmatter counts as having no `mrq.flavor`. Pass `config: None` when
    /// the configuration only holds its default, so the origin reads
    /// [`FlavorOrigin::Default`] (the flavor is the same either way). Note that a loaded
    /// [`Config`](crate::config::Config) already has `--flavor` applied; pass the flag
    /// separately as `cli` all the same, since it outranks the frontmatter and the
    /// configuration does not.
    pub fn resolve(
        &self,
        cli: Option<&str>,
        frontmatter: Option<&Frontmatter>,
        config: Option<&str>,
    ) -> Result<Resolved, FlavorError> {
        if let Some(name) = cli {
            return self.lookup(name, FlavorOrigin::Cli, None);
        }
        if let Some(fm) = frontmatter
            && let Some(value) = fm.get(&FRONTMATTER_KEY)
        {
            let span = fm
                .entry(&FRONTMATTER_KEY)
                .map_or(fm.content(), |entry| entry.value);
            return match value {
                Value::String(name) => self.lookup(name, FlavorOrigin::Frontmatter, Some(span)),
                _ => Err(FlavorError::NotAString { span }),
            };
        }
        match config {
            Some(name) => self.lookup(name, FlavorOrigin::Config, None),
            None => self.lookup(DEFAULT_FLAVOR, FlavorOrigin::Default, None),
        }
    }

    fn lookup(
        &self,
        name: &str,
        origin: FlavorOrigin,
        span: Option<Span>,
    ) -> Result<Resolved, FlavorError> {
        match self.get(name) {
            Some(flavor) => Ok(Resolved { flavor, origin }),
            None => Err(FlavorError::Unknown {
                name: name.to_owned(),
                known: self.names().collect(),
                origin,
                span,
            }),
        }
    }
}
