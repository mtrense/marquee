//! CSS-style selectors (`section[heading="Install"] code[lang=rust]`), compiled to a
//! [`QueryPlan`].
//!
//! ```text
//! list       = complex ( "," complex )*                      union
//! complex    = compound ( combinator compound )*
//! combinator = whitespace                                    descendant
//!            | ">"                                           child
//! compound   = ( type | "*" ) part* | part+
//! part       = "[" name ( op value flag? )? "]"              attribute
//!            | ":has(" relative ( "," relative )* ")"        has a match inside
//!            | ":not(" list ")" | ":is(" list ")"
//!            | ":nth-child(" an+b ")" | ":first-child"
//! relative   = ">"? complex                                  `>`: a direct child
//! op         = "=" | "^=" | "$=" | "*=" | "~="               exact, prefix, suffix,
//!                                                            substring, word
//! value      = ident | '"' quoted '"' | "'" quoted "'"
//! flag       = "i" | "s"                                     ignore case, case-sensitive
//! an+b       = "odd" | "even" | integer | integer? "n" ( ("+" | "-") integer )?
//! ```
//!
//! Type, attribute and pseudo-class names ignore case, and `-` equals `_` in type names.
//! Values are case-sensitive unless flagged `i`; a backslash in a quoted value escapes
//! the next character (CSS hex escapes are not supported).

use winnow::combinator::{alt, cut_err, opt};
use winnow::error::{ContextError, ErrMode, FromExternalError, ModalResult, StrContext};
use winnow::prelude::*;
use winnow::token::{any, one_of, take_till, take_while};

use super::selector::{expected, message, ws};
use super::{Attribute, Predicate, QueryPlan, Selector, SyntaxError, TextMatcher};
use crate::ast::NodeType;

/// What the parser expects where a compound selector is missing.
const SELECTOR: &str = "a selector (a type, `*`, `[attribute]` or `:pseudo-class`)";

/// Parses a CSS selector list.
pub(super) fn parse(source: &str) -> Result<QueryPlan, SyntaxError> {
    query
        .parse(source)
        .map_err(|err| SyntaxError::new(source, err.offset(), message(err.inner())))
}

fn query(i: &mut &str) -> ModalResult<QueryPlan> {
    let selectors = list(i, false)?;
    ws(i)?;
    if !i.is_empty() {
        return fail(expected("a combinator, `,` or end of query"));
    }
    Ok(union(selectors))
}

// --- Selector tree -------------------------------------------------------------------

/// What a compound selects before its predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Nodes of any type (`*` or no type).
    Any,
    Node(NodeType),
    /// Sections; a `heading` attribute becomes the section matcher.
    Section,
}

/// A type and the predicates of its attributes and pseudo-classes.
#[derive(Debug)]
struct Compound {
    kind: Kind,
    predicates: Vec<Predicate>,
}

#[derive(Debug, Clone, Copy)]
enum Combinator {
    Descendant,
    Child,
}

/// Compounds joined by combinators; `child_of_scope` for a relative `> …` in `:has()`.
#[derive(Debug)]
struct Complex {
    child_of_scope: bool,
    first: Compound,
    rest: Vec<(Combinator, Compound)>,
}

impl Compound {
    fn plan(self) -> QueryPlan {
        match self.kind {
            Kind::Section => {
                let mut predicates = self.predicates;
                // The first `heading` attribute becomes the section matcher.
                let first = predicates
                    .iter()
                    .position(|p| matches!(p, Predicate::Heading(_)));
                let heading = match first.map(|index| predicates.remove(index)) {
                    Some(Predicate::Heading(m)) => m,
                    _ => TextMatcher::Any,
                };
                let section = QueryPlan::section(heading);
                if predicates.is_empty() {
                    section
                } else {
                    QueryPlan::Filter(Box::new(section), predicates)
                }
            }
            Kind::Any | Kind::Node(_) => QueryPlan::select(self.selector()),
        }
    }

    /// The node test of a non-section compound.
    fn selector(self) -> Selector {
        Selector {
            node_type: match self.kind {
                Kind::Node(ty) => Some(ty),
                Kind::Any | Kind::Section => None,
            },
            predicates: self.predicates,
        }
    }
}

impl Complex {
    fn plan(self) -> QueryPlan {
        let first = self.first.plan();
        let first = if self.child_of_scope {
            QueryPlan::Scope.child(first)
        } else {
            first
        };
        self.rest
            .into_iter()
            .fold(first, |plan, (combinator, compound)| match combinator {
                Combinator::Descendant => plan.descendant(compound.plan()),
                Combinator::Child => plan.child(compound.plan()),
            })
    }
}

/// The plan of a selector list: its only selector, or their union.
fn union(selectors: Vec<Complex>) -> QueryPlan {
    let mut plans: Vec<QueryPlan> = selectors.into_iter().map(Complex::plan).collect();
    if plans.len() == 1 {
        plans.remove(0)
    } else {
        QueryPlan::union(plans)
    }
}

// --- Parsers -------------------------------------------------------------------------

/// `complex ( "," complex )*`; `relative` allows a leading `>` on each.
fn list(i: &mut &str, relative: bool) -> ModalResult<Vec<Complex>> {
    let mut selectors = Vec::new();
    loop {
        ws(i)?;
        let child_of_scope = relative && opt('>').parse_next(i)?.is_some();
        ws(i)?;
        let mut selector = complex(i)?;
        selector.child_of_scope = child_of_scope;
        selectors.push(selector);
        ws(i)?;
        if opt(',').parse_next(i)?.is_none() {
            return Ok(selectors);
        }
    }
}

/// `compound ( combinator compound )*`; stops before `,`, `)` or the end.
fn complex(i: &mut &str) -> ModalResult<Complex> {
    let first = compound(i)?;
    let mut rest = Vec::new();
    loop {
        let before = *i;
        ws(i)?;
        let combinator = if opt('>').parse_next(i)?.is_some() {
            ws(i)?;
            Combinator::Child
        } else if i.is_empty() || i.starts_with([',', ')']) || i.len() == before.len() {
            *i = before;
            break;
        } else {
            Combinator::Descendant
        };
        rest.push((combinator, compound(i)?));
    }
    Ok(Complex {
        child_of_scope: false,
        first,
        rest,
    })
}

fn compound(i: &mut &str) -> ModalResult<Compound> {
    let (kind, mut predicates) = if opt('*').parse_next(i)?.is_some() {
        (Some(Kind::Any), Vec::new())
    } else if i.starts_with(is_name_char) {
        let start = *i;
        let name = name(i)?;
        let Some((kind, predicates)) = type_selector(name) else {
            return invalid(i, start, format!("unknown type `{name}`"));
        };
        (Some(kind), predicates)
    } else {
        (None, Vec::new())
    };
    loop {
        if i.starts_with('[') {
            predicates.push(attribute(i)?);
        } else if i.starts_with(':') {
            predicates.push(pseudo_class(i)?);
        } else {
            break;
        }
    }
    if kind.is_none() && predicates.is_empty() {
        return fail(expected(SELECTOR));
    }
    Ok(Compound {
        kind: kind.unwrap_or(Kind::Any),
        predicates,
    })
}

/// The kind and implied predicates of a type name.
fn type_selector(name: &str) -> Option<(Kind, Vec<Predicate>)> {
    let name = name.to_ascii_lowercase().replace('-', "_");
    let node = |ty| Some((Kind::Node(ty), Vec::new()));
    let with = |ty, predicate| Some((Kind::Node(ty), vec![predicate]));
    match name.as_str() {
        "section" => Some((Kind::Section, Vec::new())),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => with(
            NodeType::Heading,
            Predicate::Level(name.as_bytes()[1] - b'0'),
        ),
        "p" => node(NodeType::Paragraph),
        "code" => node(NodeType::CodeBlock),
        "inline_code" => node(NodeType::Code),
        "item" | "li" => node(NodeType::ListItem),
        "task" => with(NodeType::ListItem, Predicate::Task(None)),
        "ul" => with(NodeType::List, Predicate::Ordered(false)),
        "ol" => with(NodeType::List, Predicate::Ordered(true)),
        "blockquote" => node(NodeType::BlockQuote),
        "frontmatter" => node(NodeType::FrontMatter),
        "hr" => node(NodeType::ThematicBreak),
        _ => NodeType::from_name(&name).and_then(node),
    }
}

/// `"[" name ( op value flag? )? "]"`.
fn attribute(i: &mut &str) -> ModalResult<Predicate> {
    '['.parse_next(i)?;
    ws(i)?;
    let start = *i;
    let name = take_while(0.., is_name_char).parse_next(i)?;
    if name.is_empty() {
        return fail(expected("an attribute name"));
    }
    let lowered = name.to_ascii_lowercase();
    let predicate: Box<dyn Fn(TextMatcher) -> Predicate> = match lowered.as_str() {
        "heading" => Box::new(Predicate::Heading),
        "text" => Box::new(Predicate::Text),
        other => match Attribute::from_name(other) {
            Some(attr) => Box::new(move |m| Predicate::Attr(attr, m)),
            None => return invalid(i, start, format!("unknown attribute `{name}`")),
        },
    };
    ws(i)?;
    if opt(']').parse_next(i)?.is_some() {
        return Ok(predicate(TextMatcher::Any));
    }
    let op = cut_err(
        alt(("=", "^=", "$=", "*=", "~="))
            .context(expected("an operator (`=`, `^=`, `$=`, `*=`, `~=`) or `]`")),
    )
    .parse_next(i)?;
    ws(i)?;
    let value = value(i)?;
    let mut m = match op {
        "=" => TextMatcher::exact(value),
        "^=" => TextMatcher::prefix(value),
        "$=" => TextMatcher::suffix(value),
        "*=" => TextMatcher::substring(value),
        "~=" => TextMatcher::word(value),
        _ => unreachable!("every operator has a branch"),
    };
    ws(i)?;
    if let Some(flag) = opt(one_of(['i', 'I', 's', 'S'])).parse_next(i)? {
        if flag.eq_ignore_ascii_case(&'i') {
            m = m.ignore_case();
        }
        ws(i)?;
    }
    cut_err(']'.context(expected("`]`"))).parse_next(i)?;
    Ok(predicate(m))
}

/// A quoted string or a bare value up to whitespace or `]`.
fn value(i: &mut &str) -> ModalResult<String> {
    if let Some(quote) = opt(one_of(['"', '\''])).parse_next(i)? {
        return quoted(i, quote);
    }
    let bare = take_while(0.., |c: char| {
        !c.is_whitespace() && !matches!(c, ']' | '"' | '\'')
    })
    .parse_next(i)?;
    if bare.is_empty() {
        return fail(expected("a value"));
    }
    Ok(bare.to_owned())
}

/// The rest of a string opened by `quote`, a backslash escaping any character.
fn quoted(i: &mut &str, quote: char) -> ModalResult<String> {
    let closing = if quote == '"' { "`\"`" } else { "`'`" };
    let mut text = String::new();
    loop {
        text.push_str(take_till(0.., [quote, '\\']).parse_next(i)?);
        if opt('\\').parse_next(i)?.is_some() {
            text.push(cut_err(any.context(expected("an escaped character"))).parse_next(i)?);
        } else {
            cut_err(quote.context(expected(closing))).parse_next(i)?;
            return Ok(text);
        }
    }
}

/// `":" name ( "(" … ")" )?`.
fn pseudo_class(i: &mut &str) -> ModalResult<Predicate> {
    ':'.parse_next(i)?;
    let start = *i;
    let name = name(i)?;
    let predicate = match name.to_ascii_lowercase().as_str() {
        "first-child" => return Ok(Predicate::NthChild { a: 0, b: 1 }),
        "has" => {
            let selectors = arguments(i, |i| list(i, true))?;
            Predicate::Has(Box::new(union(selectors)))
        }
        "is" => {
            let selectors = arguments(i, |i| list(i, false))?;
            Predicate::Is(Box::new(union(selectors)))
        }
        "not" => {
            let selectors = arguments(i, |i| list(i, false))?;
            Predicate::Not(Box::new(negated(selectors)))
        }
        "nth-child" => arguments(i, nth_child)?,
        _ => return invalid(i, start, format!("unknown pseudo-class `{name}`")),
    };
    Ok(predicate)
}

/// The selector `:not()` negates: a plain compound as is, anything else (combinators,
/// sections, several selectors) through [`Predicate::Is`].
fn negated(mut selectors: Vec<Complex>) -> Selector {
    if let [only] = selectors.as_slice()
        && only.rest.is_empty()
        && only.first.kind != Kind::Section
    {
        return selectors.remove(0).first.selector();
    }
    Selector::any().with(Predicate::Is(Box::new(union(selectors))))
}

/// `"(" inner ")"`, whitespace allowed inside.
fn arguments<T>(
    i: &mut &str,
    mut inner: impl FnMut(&mut &str) -> ModalResult<T>,
) -> ModalResult<T> {
    cut_err('('.context(expected("`(`"))).parse_next(i)?;
    ws(i)?;
    let value = inner(i)?;
    ws(i)?;
    cut_err(')'.context(expected("`)`"))).parse_next(i)?;
    Ok(value)
}

/// `an+b`, `odd` or `even` up to the closing parenthesis.
fn nth_child(i: &mut &str) -> ModalResult<Predicate> {
    let start = *i;
    let arg = take_till(0.., ')').parse_next(i)?;
    let compact: String = arg
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let Some((a, b)) = an_plus_b(&compact) else {
        *i = start;
        return fail(expected("`an+b`, `odd` or `even`"));
    };
    Ok(Predicate::NthChild { a, b })
}

fn an_plus_b(arg: &str) -> Option<(i64, i64)> {
    match arg {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    let Some((a, b)) = arg.split_once('n') else {
        return Some((0, arg.parse().ok()?));
    };
    let a = match a {
        "" | "+" => 1,
        "-" => -1,
        a => a.parse().ok()?,
    };
    let b = if b.is_empty() {
        0
    } else if b.starts_with(['+', '-']) {
        b.parse().ok()?
    } else {
        return None;
    };
    Some((a, b))
}

// --- Helpers -------------------------------------------------------------------------

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

fn name<'i>(i: &mut &'i str) -> ModalResult<&'i str> {
    take_while(1.., is_name_char)
        .context(expected("a name"))
        .parse_next(i)
}

/// A cut error expecting `what` at the current position.
fn fail<T>(what: StrContext) -> ModalResult<T> {
    let mut err = ContextError::new();
    err.push(what);
    Err(ErrMode::Cut(err))
}

/// A cut error with `message`, pointing at `start`.
fn invalid<'i, T>(i: &mut &'i str, start: &'i str, message: String) -> ModalResult<T> {
    *i = start;
    Err(ErrMode::Cut(ContextError::from_external_error(
        i,
        Invalid(message),
    )))
}

/// A semantic error (unknown name), carried as the cause of a winnow error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Invalid(String);
