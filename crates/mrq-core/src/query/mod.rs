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
//!             .with(Predicate::Attr(Attribute::Lang, TextMatcher::exact("sh").ignore_case())),
//!     ),
//! );
//! let found = plan.evaluate(&doc);
//! assert_eq!(found.len(), 1);
//! assert_eq!(found[0].text(), "make");
//! ```
//!
//! [`Query`] holds a query in one of the front-end syntaxes, compiled to its plan when it
//! is created; [`Document::select`](crate::Document::select) evaluates it. Until the CSS
//! and jq front-ends exist (M3.3–M3.4) those queries fail with
//! [`QueryError::NotImplemented`].
//!
//! # Selector DSL
//!
//! [`Query::selector`] parses mrq's Markdown-native selectors (mdq-style). Each selector
//! is a marker optionally followed by a text matcher; `a | b` selects `b` within each
//! result of `a`, the result itself included ([`Pipe`](QueryPlan::Pipe)).
//!
//! | Selector | Matches |
//! | --- | --- |
//! | `# text` | sections whose heading text matches |
//! | `- text` / `1. text` | bullet / ordered list items |
//! | `- [ ] text` / `- [x]` / `- [?]` | open / done / any tasks (`1. [ ]` in ordered lists) |
//! | `[text](url)` / `![alt](url)` | links / images |
//! | `> text` | block quotes |
//! | ```` ```lang text ```` | code blocks; `lang` follows the backticks directly |
//! | `:-: text` | tables |
//! | `</> text` | raw HTML (blocks and inline), comments and `mrq:` directives |
//! | `+++format text` | frontmatter (`+++yaml`, `+++toml`), matching its raw content |
//! | `P: text` | paragraphs |
//!
//! Text matchers: bare `words` (a case-insensitive substring, up to the next `|`),
//! `"exact"` (case-sensitive, with `\"` and `\\` escapes), `/regex/`, `^anchored$` (on
//! bare or quoted text), `*` or nothing (anything). A bare language or frontmatter
//! format is matched exactly, ignoring case, so ```` ```sh ```` does not match `bash`.
//! Errors are [`QueryError::Syntax`] with the byte offset of the problem:
//!
//! ```
//! use mrq_core::query::{Query, QueryError};
//!
//! let Err(QueryError::Syntax(err)) = Query::selector("# Install | [text](url") else {
//!     panic!("expected a syntax error");
//! };
//! assert_eq!(err.offset(), 22);
//! assert_eq!(err.render(), "# Install | [text](url\n                      ^ expected `)`");
//! ```

mod engine;
mod error;
mod plan;
mod section;
mod selector;

pub use engine::Match;
pub use error::{QueryError, SyntaxError};
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
    /// A Markdown-native selector query, compiled to a plan; see the [module
    /// docs](self#selector-dsl) for the syntax.
    ///
    /// # Errors
    ///
    /// [`QueryError::Syntax`] if `source` does not parse (an invalid `/regex/`
    /// included).
    pub fn selector(source: &str) -> Result<Self, QueryError> {
        let plan = selector::parse(source)?;
        Ok(Self {
            plan: Some(plan),
            ..Self::new(Syntax::Selector, source)
        })
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
