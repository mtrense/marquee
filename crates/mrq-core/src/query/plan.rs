//! The query IR: [`QueryPlan`] with its [`Selector`]s, [`Predicate`]s, [`Attribute`]s
//! and [`TextMatcher`]s.

use std::fmt;

use regex::Regex;

use super::QueryError;
use crate::ast::{NodeType, TaskState};

/// What a query selects, as a tree of selections and combinators. Every front-end
/// compiles to a plan; [`evaluate`](Self::evaluate) runs one over a document.
///
/// A plan is evaluated against a *context*: the whole document at the top, then each
/// result of the left side of a combinator. Results are nodes or
/// [sections](super::Section), deduplicated and in document order (a section comes
/// right before its heading).
///
/// "Inside" a context means: in a node's subtree, or in a section's heading and content
/// (including subsections). Selections at the left edge of a plan also consider the
/// context itself (`Pipe`), unless a combinator asks for strict nesting (`Descendant`,
/// `Child`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueryPlan {
    /// Nodes inside the context that the [`Selector`] matches.
    Select(Selector),
    /// [Sections](super::Section) inside the context whose heading text matches.
    Section(TextMatcher),
    /// Results of the plan that satisfy every predicate, e.g. sections that `Has` a code
    /// block (CSS `section:has(code)`).
    Filter(Box<QueryPlan>, Vec<Predicate>),
    /// The right side within each result of the left, strictly inside it (CSS `a b`).
    Descendant(Box<QueryPlan>, Box<QueryPlan>),
    /// Results of the right side whose parent is a result of the left (CSS `a > b`). The
    /// parent of a node directly in a section's content (or of its heading) is the
    /// innermost such section; the parent of a section is the enclosing section, else
    /// the container of its heading.
    Child(Box<QueryPlan>, Box<QueryPlan>),
    /// The right side within each result of the left, the result itself included (the
    /// selector DSL's `a | b`).
    Pipe(Box<QueryPlan>, Box<QueryPlan>),
    /// The results of every plan (CSS `a, b`).
    Union(Vec<QueryPlan>),
}

impl QueryPlan {
    /// [`Select`](Self::Select).
    pub const fn select(selector: Selector) -> Self {
        Self::Select(selector)
    }

    /// [`Section`](Self::Section).
    pub const fn section(heading: TextMatcher) -> Self {
        Self::Section(heading)
    }

    /// [`Union`](Self::Union).
    pub fn union(plans: impl IntoIterator<Item = Self>) -> Self {
        Self::Union(plans.into_iter().collect())
    }

    /// `self | inner`: [`Pipe`](Self::Pipe).
    #[must_use]
    pub fn pipe(self, inner: Self) -> Self {
        Self::Pipe(Box::new(self), Box::new(inner))
    }

    /// `self inner`: [`Descendant`](Self::Descendant).
    #[must_use]
    pub fn descendant(self, inner: Self) -> Self {
        Self::Descendant(Box::new(self), Box::new(inner))
    }

    /// `self > inner`: [`Child`](Self::Child).
    #[must_use]
    pub fn child(self, inner: Self) -> Self {
        Self::Child(Box::new(self), Box::new(inner))
    }

    /// Adds `predicate` to a [`Filter`](Self::Filter) (wrapping `self` in one if needed).
    #[must_use]
    pub fn filter(self, predicate: Predicate) -> Self {
        match self {
            Self::Filter(plan, mut predicates) => {
                predicates.push(predicate);
                Self::Filter(plan, predicates)
            }
            plan => Self::Filter(Box::new(plan), vec![predicate]),
        }
    }
}

/// A node test: an optional [`NodeType`] plus predicates that must all hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// The type nodes must have; `None` for any (CSS `*`).
    pub node_type: Option<NodeType>,
    /// Further conditions, all of which must hold.
    pub predicates: Vec<Predicate>,
}

impl Selector {
    /// Nodes of type `node_type`.
    pub const fn new(node_type: NodeType) -> Self {
        Self {
            node_type: Some(node_type),
            predicates: Vec::new(),
        }
    }

    /// Nodes of any type.
    pub const fn any() -> Self {
        Self {
            node_type: None,
            predicates: Vec::new(),
        }
    }

    /// Adds a predicate.
    #[must_use]
    pub fn with(mut self, predicate: Predicate) -> Self {
        self.predicates.push(predicate);
        self
    }
}

/// A condition on a node or section.
///
/// For a section, attributes and the level are its heading's, the text is the whole
/// section's, and the position (`NthChild`) is its heading's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Predicate {
    /// The plain text ([`Ast::text`](crate::ast::Ast::text)) matches.
    Text(TextMatcher),
    /// Some value of the attribute matches; nodes without the attribute never match.
    Attr(Attribute, TextMatcher),
    /// A heading (or section) of this level.
    Level(u8),
    /// A task list item in this state; `None` for any task.
    Task(Option<TaskState>),
    /// An ordered (`true`) or bullet (`false`) list, or an item of one.
    Ordered(bool),
    /// The plan finds something strictly inside (CSS `:has()`).
    Has(Box<QueryPlan>),
    /// The selector does not match (CSS `:not()`). A section never matches a selector
    /// with a type.
    Not(Box<Selector>),
    /// The 1-based position `n` among the parent's children is `a·k + b` for some
    /// `k ≥ 0` (CSS `:nth-child(an+b)`).
    NthChild {
        /// Step.
        a: i64,
        /// Offset.
        b: i64,
    },
}

/// A named property of a node, for [`Predicate::Attr`] (CSS attribute selectors).
///
/// Values come from the node kind's payload; a section has its heading's attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Attribute {
    /// Code block language, the first word of the info string.
    Lang,
    /// Code block info string.
    Info,
    /// Link or image destination.
    Url,
    /// Link or image title; admonition or callout title.
    Title,
    /// Heading level, in decimal.
    Level,
    /// Admonition or callout type (`note`, `WARNING`, …).
    Kind,
    /// Wikilink or embed target.
    Target,
    /// Wikilink or embed anchor.
    Anchor,
    /// Wikilink or embed alias.
    Alias,
    /// Footnote definition or reference label, link definition label.
    Label,
    /// Tag name without `#`.
    Name,
    /// Heading `{#id}`.
    Id,
    /// Heading `{.class}`es and admonition classes (multi-valued).
    Class,
}

impl Attribute {
    /// Every attribute.
    pub const ALL: &'static [Self] = &[
        Self::Lang,
        Self::Info,
        Self::Url,
        Self::Title,
        Self::Level,
        Self::Kind,
        Self::Target,
        Self::Anchor,
        Self::Alias,
        Self::Label,
        Self::Name,
        Self::Id,
        Self::Class,
    ];

    /// Stable lowercase name, as used in CSS attribute selectors.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lang => "lang",
            Self::Info => "info",
            Self::Url => "url",
            Self::Title => "title",
            Self::Level => "level",
            Self::Kind => "kind",
            Self::Target => "target",
            Self::Anchor => "anchor",
            Self::Alias => "alias",
            Self::Label => "label",
            Self::Name => "name",
            Self::Id => "id",
            Self::Class => "class",
        }
    }

    /// Looks an attribute up by its [`as_str`](Self::as_str) name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|attr| attr.as_str() == name)
    }
}

impl fmt::Display for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a text (heading text, plain text, attribute value) is matched.
///
/// The literal constructors are case-sensitive; [`ignore_case`](Self::ignore_case) folds
/// case. The selector DSL's bare `word` is `substring(word).ignore_case()`, `"exact"`
/// is `exact`, `^anchored$` is `prefix`/`suffix`/`exact`; CSS `*=`, `=`, `^=`, `$=`,
/// `~=` map to `substring`, `exact`, `prefix`, `suffix`, `word`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextMatcher {
    /// Anything, including the empty text (`*`).
    Any,
    /// Contains the literal.
    Substring(Literal),
    /// Equals the literal.
    Exact(Literal),
    /// Starts with the literal (`^text`).
    Prefix(Literal),
    /// Ends with the literal (`text$`).
    Suffix(Literal),
    /// One of the whitespace-separated words equals the literal (CSS `~=`).
    Word(Literal),
    /// The regular expression matches somewhere (`/regex/`).
    Regex(Pattern),
}

impl TextMatcher {
    /// [`Substring`](Self::Substring), case-sensitive.
    pub fn substring(text: impl Into<String>) -> Self {
        Self::Substring(Literal::new(text))
    }

    /// [`Exact`](Self::Exact), case-sensitive.
    pub fn exact(text: impl Into<String>) -> Self {
        Self::Exact(Literal::new(text))
    }

    /// [`Prefix`](Self::Prefix), case-sensitive.
    pub fn prefix(text: impl Into<String>) -> Self {
        Self::Prefix(Literal::new(text))
    }

    /// [`Suffix`](Self::Suffix), case-sensitive.
    pub fn suffix(text: impl Into<String>) -> Self {
        Self::Suffix(Literal::new(text))
    }

    /// [`Word`](Self::Word), case-sensitive.
    pub fn word(text: impl Into<String>) -> Self {
        Self::Word(Literal::new(text))
    }

    /// [`Regex`](Self::Regex); see [`Pattern::new`].
    pub fn regex(pattern: &str) -> Result<Self, QueryError> {
        Pattern::new(pattern).map(Self::Regex)
    }

    /// The same matcher, folding case. [`Any`](Self::Any) and
    /// [`Regex`](Self::Regex) are unchanged (use `(?i)` in the pattern).
    #[must_use]
    pub fn ignore_case(mut self) -> Self {
        if let Self::Substring(lit)
        | Self::Exact(lit)
        | Self::Prefix(lit)
        | Self::Suffix(lit)
        | Self::Word(lit) = &mut self
        {
            lit.ignore_case = true;
        }
        self
    }

    /// Whether `text` matches.
    pub fn matches(&self, text: &str) -> bool {
        let compare = |lit: &Literal, test: fn(&str, &str) -> bool| {
            if lit.ignore_case {
                test(&text.to_lowercase(), &lit.text.to_lowercase())
            } else {
                test(text, &lit.text)
            }
        };
        match self {
            Self::Any => true,
            Self::Substring(lit) => compare(lit, |t, l| t.contains(l)),
            Self::Exact(lit) => compare(lit, |t, l| t == l),
            Self::Prefix(lit) => compare(lit, |t, l| t.starts_with(l)),
            Self::Suffix(lit) => compare(lit, |t, l| t.ends_with(l)),
            Self::Word(lit) => compare(lit, |t, l| t.split_whitespace().any(|w| w == l)),
            Self::Regex(pattern) => pattern.is_match(text),
        }
    }
}

/// The literal of a [`TextMatcher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Literal {
    /// The text to look for.
    pub text: String,
    /// Compare lowercased texts.
    pub ignore_case: bool,
}

impl Literal {
    /// A case-sensitive literal.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ignore_case: false,
        }
    }
}

/// A compiled regular expression ([`regex`] syntax). Patterns compare equal when their
/// source text is equal.
#[derive(Debug, Clone)]
pub struct Pattern(Regex);

impl Pattern {
    /// Compiles `pattern`.
    ///
    /// # Errors
    ///
    /// [`QueryError::Regex`] if the pattern is invalid.
    pub fn new(pattern: &str) -> Result<Self, QueryError> {
        Regex::new(pattern)
            .map(Self)
            .map_err(|err| QueryError::Regex {
                pattern: pattern.to_owned(),
                message: err.to_string(),
            })
    }

    /// The pattern source.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether the pattern matches somewhere in `text`.
    pub fn is_match(&self, text: &str) -> bool {
        self.0.is_match(text)
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Pattern {}
