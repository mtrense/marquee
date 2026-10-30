//! Queries over a [`Document`](crate::Document): a placeholder until the query engine
//! (M3).
//!
//! [`Query`] already has the constructors of the three front-ends, which keep the query
//! text, and [`Document::select`](crate::Document::select) has its final signature, but
//! evaluating a query fails with [`QueryError::NotImplemented`]. Parse errors arrive
//! with the front-ends.

/// The syntax a [`Query`] is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Syntax {
    /// mrq's Markdown-native selectors (the default), e.g. `# Install | ```sh`.
    Selector,
    /// CSS-style selectors (`--css`).
    Css,
    /// jq over the JSON projection (`--jq`).
    Jq,
}

/// A query in one of the front-end [`Syntax`]es.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    syntax: Syntax,
    source: String,
}

impl Query {
    /// A Markdown-native selector query.
    pub fn selector(source: &str) -> Result<Self, QueryError> {
        Ok(Self::new(Syntax::Selector, source))
    }

    /// A CSS-style selector query.
    pub fn css(source: &str) -> Result<Self, QueryError> {
        Ok(Self::new(Syntax::Css, source))
    }

    /// A jq query.
    pub fn jq(source: &str) -> Result<Self, QueryError> {
        Ok(Self::new(Syntax::Jq, source))
    }

    fn new(syntax: Syntax, source: &str) -> Self {
        Self {
            syntax,
            source: source.to_owned(),
        }
    }

    /// The front-end syntax.
    pub const fn syntax(&self) -> Syntax {
        self.syntax
    }

    /// The query text as given.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Why a query could not be parsed or evaluated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryError {
    /// Query evaluation does not exist yet (M3).
    #[error("queries are not implemented yet")]
    NotImplemented,
}
