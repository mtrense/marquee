//! Queries over a [`Document`](crate::Document): the [`QueryPlan`] IR, its evaluation
//! into [`Match`]es, and [sections](Section).
//!
//! Every front-end (the selector DSL, CSS, jq) compiles to a [`QueryPlan`]: selections
//! ([`Select`](QueryPlan::Select) with a [`Selector`] of a node type and
//! [`Predicate`]s, [`Section`](QueryPlan::Section) with a [`TextMatcher`] for the
//! heading) joined by combinators ([`Pipe`](QueryPlan::Pipe),
//! [`Descendant`](QueryPlan::Descendant), [`Child`](QueryPlan::Child),
//! [`Union`](QueryPlan::Union), [`Filter`](QueryPlan::Filter)).
//! [`QueryPlan::evaluate`] returns the matching nodes and sections in document order.
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::ast::NodeType;
//! use mrq_core::query::{Attribute, Predicate, QueryPlan, Selector, TextMatcher};
//!
//! let doc = Document::parse("# Install\n\n```sh\nmake\n```\n\n# Use\n\n```sh\nrun\n```\n");
//! // The selector DSL's `# install | ```sh`.
//! let plan = QueryPlan::section(TextMatcher::substring("install").ignore_case()).pipe(
//!     QueryPlan::select(
//!         Selector::new(NodeType::CodeBlock)
//!             .with(Predicate::Attr(Attribute::Lang, TextMatcher::exact("sh"))),
//!     ),
//! );
//! let found = plan.evaluate(&doc);
//! assert_eq!(found.len(), 1);
//! assert_eq!(found[0].text(), "make");
//! ```
//!
//! [`Query`] holds a query in one of the front-end syntaxes. Until the front-ends exist
//! (M3.2–M3.4) only [`Query::from_plan`] queries can be evaluated by
//! [`Document::select`](crate::Document::select); text queries fail with
//! [`QueryError::NotImplemented`].

mod engine;
mod plan;
mod section;

pub use engine::Match;
pub use plan::{Attribute, Literal, Pattern, Predicate, QueryPlan, Selector, TextMatcher};
pub use section::{Section, sections};

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
    /// A hand-built [`QueryPlan`] ([`Query::from_plan`]).
    Plan,
}

/// A query in one of the front-end [`Syntax`]es.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    syntax: Syntax,
    source: String,
    plan: Option<QueryPlan>,
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

    /// A query that evaluates `plan`; its [`source`](Self::source) is empty.
    pub const fn from_plan(plan: QueryPlan) -> Self {
        Self {
            syntax: Syntax::Plan,
            source: String::new(),
            plan: Some(plan),
        }
    }

    fn new(syntax: Syntax, source: &str) -> Self {
        Self {
            syntax,
            source: source.to_owned(),
            plan: None,
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

    /// The compiled plan; `None` while the query's front-end does not exist yet.
    pub const fn plan(&self) -> Option<&QueryPlan> {
        self.plan.as_ref()
    }
}

/// Why a query could not be parsed or evaluated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryError {
    /// The query's front-end does not exist yet (M3.2–M3.4).
    #[error("queries are not implemented yet")]
    NotImplemented,
    /// A `/regex/` matcher does not compile.
    #[error("invalid regex `{pattern}`: {message}")]
    Regex {
        /// The pattern as written.
        pattern: String,
        /// What is wrong with it.
        message: String,
    },
}
