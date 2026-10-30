//! Queries over a [`Document`](crate::Document): the [`QueryPlan`] IR, its evaluation
//! into [`Match`]es, and [sections](Section).
//!
//! The selector DSL and CSS front-ends compile to a [`QueryPlan`]: selections
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
//! [`Query`] holds a query in one of the front-end syntaxes, compiled (to its plan, or to
//! a jq program) when it is created; [`Document::select`](crate::Document::select)
//! evaluates it to [`Match`]es and
//! [`Document::select_json`](crate::Document::select_json) to [`JsonOutput`]s.
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
//!
//! # CSS selectors
//!
//! [`Query::css`] parses CSS-style selectors. A section counts as an element whose
//! children are its heading, its content blocks and its direct subsections; `*` and
//! attribute-only compounds match nodes, never sections.
//!
//! | Selector | Matches |
//! | --- | --- |
//! | `section` | [sections](Section) |
//! | `heading`, `h1`–`h6` | headings (of a level) |
//! | `p`, `code`, `inline-code`, `link`, `image`, `table`, `blockquote`, `frontmatter`, `hr` | paragraphs, code blocks, inline code, … |
//! | `list`, `ul`, `ol`, `item`/`li`, `task` | lists (bullet, ordered), list items, task items |
//! | any [`NodeType`](crate::ast::NodeType) name | that type, e.g. `code_block`, `wiki_link` (`-` or `_`) |
//! | `*` | any node |
//! | `[attr]`, `[attr=v]`, `^=`, `$=`, `*=`, `~=` | an [`Attribute`] that exists, equals, starts with, ends with, contains, has the word `v`; ` i` after the value ignores case |
//! | `[heading=v]` / `[text=v]` | the heading text of a section or heading / the plain text |
//! | `a b`, `a > b` | `b` inside / a direct child of `a` |
//! | `a, b` | either |
//! | `:has(b)`, `:has(> b)` | something matching `b` inside / as a direct child |
//! | `:not(a, …)`, `:is(a, …)` | not / any of the selectors (combinators allowed) |
//! | `:nth-child(an+b)`, `odd`, `even`, `:first-child` | by position among the parent's children |
//!
//! Values are case-sensitive, bare (`lang=rust`) or quoted (`"…"`, `'…'`):
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::query::Query;
//!
//! let doc = Document::parse("# Install\n\n```rust\nmain()\n```\n\n# Use\n\n```rust\nrun()\n```\n");
//! let query = Query::css(r#"section[heading="Install"] code[lang=rust]"#)?;
//! let found = doc.select(&query)?;
//! assert_eq!(found.len(), 1);
//! assert_eq!(found[0].text(), "main()");
//! # Ok::<(), mrq_core::query::QueryError>(())
//! ```
//!
//! # jq
//!
//! [`Query::jq`] compiles a jq program (with [jaq](https://github.com/01mf02/jaq) and
//! its standard library) that runs on the [JSON projection](crate::ast::json) of the
//! document. Its outputs are arbitrary JSON values, so jq has no [`QueryPlan`]:
//! [`Document::select_json`](crate::Document::select_json) returns them as
//! [`JsonOutput`]s, whose [`span`](JsonOutput::span) and
//! [`to_match`](JsonOutput::to_match) lead back to the source when an output is a node
//! or section; [`Document::select`](crate::Document::select) requires every output to be
//! one. mrq adds these definitions:
//!
//! | Definition | Outputs |
//! | --- | --- |
//! | `nodes` | the input and every node inside it, in document order (unlike `..`, without frontmatter values and other fields) |
//! | `links` | the `link` nodes inside the input |
//! | `code`, `code($lang)` | the `code` blocks inside the input (with language `$lang`, ignoring case) |
//! | `sections` | the sections inside the input node or section as `section` objects (see [`JsonOutput`]) |
//! | `text` | the plain text of the input node or section ([`Node::text`](crate::ast::Node::text), [`Section::text`]); a string as is |
//!
//! Parse errors and undefined names are [`QueryError::Syntax`] at the offending token;
//! errors raised while the program runs are [`QueryError::Runtime`]:
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::query::{Query, QueryError};
//!
//! let doc = Document::parse("# Install\n\nSee [docs](./docs.md).\n\n# Use\n");
//! let urls = doc.select_json(&Query::jq(r#".. | select(.type? == "link") | .url"#)?)?;
//! assert_eq!(urls[0].value(), "./docs.md");
//! let found = doc.select(&Query::jq(r#"sections | select(.heading | text == "Use")"#)?)?;
//! assert_eq!(found[0].source(doc.text()), "# Use");
//!
//! let Err(QueryError::Syntax(err)) = Query::jq("links | txt") else { panic!() };
//! assert_eq!(err.render(), "links | txt\n        ^ undefined filter `txt/0`");
//! # Ok::<(), QueryError>(())
//! ```

mod css;
mod engine;
mod error;
mod jq;
mod output;
mod plan;
mod section;
mod selector;

pub use engine::Match;
pub use error::{QueryError, RuntimeError, SyntaxError};
pub use output::JsonOutput;
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

/// A query in one of the front-end [`Syntax`]es, compiled. Queries are equal when their
/// syntax and source are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    syntax: Syntax,
    source: String,
    plan: Option<QueryPlan>,
    jq: Option<jq::Program>,
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

    /// A CSS-style selector query, compiled to a plan; see the [module
    /// docs](self#css-selectors) for the syntax.
    ///
    /// # Errors
    ///
    /// [`QueryError::Syntax`] if `source` does not parse (an unknown type, attribute or
    /// pseudo-class included).
    pub fn css(source: &str) -> Result<Self, QueryError> {
        let plan = css::parse(source)?;
        Ok(Self {
            plan: Some(plan),
            ..Self::new(Syntax::Css, source)
        })
    }

    /// A jq program over the JSON projection, compiled; see the [module docs](self#jq).
    ///
    /// # Errors
    ///
    /// [`QueryError::Syntax`] if `source` does not parse or uses an undefined filter,
    /// variable or label.
    pub fn jq(source: &str) -> Result<Self, QueryError> {
        let program = jq::compile(source)?;
        Ok(Self {
            jq: Some(program),
            ..Self::new(Syntax::Jq, source)
        })
    }

    /// A query that evaluates `plan`; its [`source`](Self::source) is empty.
    pub const fn from_plan(plan: QueryPlan) -> Self {
        Self {
            syntax: Syntax::Plan,
            source: String::new(),
            plan: Some(plan),
            jq: None,
        }
    }

    fn new(syntax: Syntax, source: &str) -> Self {
        Self {
            syntax,
            source: source.to_owned(),
            plan: None,
            jq: None,
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

    /// The compiled plan; `None` for jq, which has no plan.
    pub const fn plan(&self) -> Option<&QueryPlan> {
        self.plan.as_ref()
    }

    /// The outputs of the jq program on `doc`; nothing for other syntaxes.
    pub(crate) fn run_jq(&self, doc: &crate::Document) -> Result<Vec<JsonOutput>, QueryError> {
        let Some(program) = &self.jq else {
            return Ok(Vec::new());
        };
        let values = program.run(&self.source, doc)?;
        Ok(values.into_iter().map(JsonOutput::new).collect())
    }
}
