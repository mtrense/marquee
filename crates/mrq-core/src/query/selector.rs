//! The Markdown-native selector DSL (`# Install | ```sh`), compiled to a [`QueryPlan`].
//!
//! ```text
//! query    = selector ( "|" selector )*
//! selector = "#" matcher                         section by heading text
//!          | "-" task? matcher                   bullet list item
//!          | "1." task? matcher                  ordered list item
//!          | "[" matcher "](" matcher ")"        link by text and url
//!          | "![" matcher "](" matcher ")"       image by alt text and url
//!          | ">" matcher                         block quote
//!          | "```" lang? matcher                 code block by language and content
//!          | ":-:" matcher                       table
//!          | "</>" matcher                       HTML (blocks, inline, comments)
//!          | "+++" format? matcher               frontmatter (`yaml`/`toml`) by content
//!          | "P:" matcher                        paragraph
//! task     = "[ ]" | "[x]" | "[X]" | "[?]"       open, done, any
//! matcher  = "*" | "/" regex "/" | "^"? ( '"' quoted '"' "$"? | bare )
//! ```
//!
//! Whitespace is allowed between tokens, except that `lang` and `format` follow their
//! marker directly (```` ``` rust ```` is any code block containing `rust`). Bare text
//! runs up to the next `|` (`]` or `)` inside a link) and is trimmed.

use winnow::combinator::{alt, cut_err, delimited, opt};
use winnow::error::{
    ContextError, ErrMode, FromExternalError, ModalResult, StrContext, StrContextValue,
};
use winnow::prelude::*;
use winnow::token::{any, one_of, take_till, take_while};

use super::{Attribute, Predicate, QueryError, QueryPlan, Selector, SyntaxError, TextMatcher};
use crate::ast::{NodeType, TaskState};

/// What the parser expects where a selector is missing.
const SELECTOR: &str =
    "a selector (`#`, `-`, `1.`, `[]()`, `![]()`, `>`, ```, `:-:`, `</>`, `+++` or `P:`)";

/// Selector markers; longer ones before their prefixes.
const MARKERS: [&str; 11] = [
    "</>", "![", "[", "```", ":-:", "+++", "P:", "1.", "#", "-", ">",
];

/// Parses a selector query.
pub(super) fn parse(source: &str) -> Result<QueryPlan, SyntaxError> {
    query
        .parse(source)
        .map_err(|err| SyntaxError::new(source, err.offset(), message(err.inner())))
}

/// `selector ( "|" selector )*`, pipes associating to the left.
fn query(i: &mut &str) -> ModalResult<QueryPlan> {
    ws(i)?;
    let mut plan = selector(i)?;
    loop {
        ws(i)?;
        if i.is_empty() {
            return Ok(plan);
        }
        cut_err('|'.context(expected("`|` or end of query"))).parse_next(i)?;
        ws(i)?;
        plan = plan.pipe(cut_err(selector).parse_next(i)?);
    }
}

fn selector(i: &mut &str) -> ModalResult<QueryPlan> {
    let marker: &str = alt(MARKERS).context(expected(SELECTOR)).parse_next(i)?;
    match marker {
        "#" => cut_err(spaced(Bare::Substring))
            .map(QueryPlan::section)
            .parse_next(i),
        "-" | "1." => cut_err(list_item(marker == "1.")).parse_next(i),
        "[" => cut_err(link(NodeType::Link)).parse_next(i),
        "![" => cut_err(link(NodeType::Image)).parse_next(i),
        ">" => cut_err(by_text(NodeType::BlockQuote)).parse_next(i),
        "```" => cut_err(tagged(
            NodeType::CodeBlock,
            Attribute::Lang,
            Predicate::Text,
        ))
        .parse_next(i),
        ":-:" => cut_err(by_text(NodeType::Table)).parse_next(i),
        "</>" => cut_err(spaced(Bare::Substring)).map(html).parse_next(i),
        "+++" => cut_err(tagged(NodeType::FrontMatter, Attribute::Format, |m| {
            Predicate::Attr(Attribute::Raw, m)
        }))
        .parse_next(i),
        "P:" => cut_err(by_text(NodeType::Paragraph)).parse_next(i),
        _ => unreachable!("every marker has a branch"),
    }
}

/// `matcher` on the plain text of a node type.
fn by_text<'i>(ty: NodeType) -> impl Parser<&'i str, QueryPlan, ErrMode<ContextError>> {
    spaced(Bare::Substring).map(move |m| select(ty, [text(m)]))
}

/// `task? matcher` after `-` or `1.`.
fn list_item<'i>(ordered: bool) -> impl Parser<&'i str, QueryPlan, ErrMode<ContextError>> {
    move |i: &mut &'i str| {
        ws(i)?;
        let task = opt(delimited('[', one_of([' ', 'x', 'X', '?']), ']')).parse_next(i)?;
        let task = task.map(|marker| match marker {
            ' ' => Some(TaskState::Open),
            'x' | 'X' => Some(TaskState::Done),
            _ => None,
        });
        let m = spaced(Bare::Substring).parse_next(i)?;
        Ok(select(
            NodeType::ListItem,
            [
                Some(Predicate::Ordered(ordered)),
                task.map(Predicate::Task),
                text(m),
            ],
        ))
    }
}

/// `matcher "](" matcher ")"` after `[` or `![`.
fn link<'i>(ty: NodeType) -> impl Parser<&'i str, QueryPlan, ErrMode<ContextError>> {
    move |i: &mut &'i str| {
        let label = spaced_until(i, Bare::Substring, |c| c == ']')?;
        ']'.context(expected("`]`")).parse_next(i)?;
        '('.context(expected("`(`")).parse_next(i)?;
        let url = spaced_until(i, Bare::Substring, |c| c == ')')?;
        ')'.context(expected("`)`")).parse_next(i)?;
        Ok(select(ty, [text(label), attr(Attribute::Url, url)]))
    }
}

/// `tag? matcher` after ```` ``` ```` or `+++`: an attribute matcher written right after
/// the marker (bare words match exactly, ignoring case), then a content matcher.
fn tagged<'i>(
    ty: NodeType,
    tag: Attribute,
    content: fn(TextMatcher) -> Predicate,
) -> impl Parser<&'i str, QueryPlan, ErrMode<ContextError>> {
    move |i: &mut &'i str| {
        let tag_matcher = if i.starts_with(|c: char| !ends_tag(c)) {
            matcher(i, Bare::Exact, ends_tag)?
        } else {
            TextMatcher::Any
        };
        let m = spaced(Bare::Substring).parse_next(i)?;
        Ok(select(
            ty,
            [attr(tag, tag_matcher), (!is_any(&m)).then(|| content(m))],
        ))
    }
}

fn ends_tag(c: char) -> bool {
    c.is_whitespace() || c == '|'
}

/// HTML blocks, inline HTML and comments (directives included), by their raw content.
fn html(m: TextMatcher) -> QueryPlan {
    let plan = QueryPlan::union(
        [
            NodeType::HtmlBlock,
            NodeType::HtmlInline,
            NodeType::Comment,
            NodeType::IncludeDirective,
            NodeType::TocDirective,
            NodeType::DirectiveEnd,
        ]
        .map(|ty| QueryPlan::select(Selector::new(ty))),
    );
    match attr(Attribute::Raw, m) {
        Some(raw) => plan.filter(raw),
        None => plan,
    }
}

// --- Text matchers -------------------------------------------------------------------

/// What unanchored bare text means.
#[derive(Debug, Clone, Copy)]
enum Bare {
    /// Contained anywhere, ignoring case (headings, content).
    Substring,
    /// The whole text, ignoring case (code languages, frontmatter formats).
    Exact,
}

/// Whitespace, then a matcher whose bare text runs up to the next `|`.
fn spaced<'i>(bare: Bare) -> impl Parser<&'i str, TextMatcher, ErrMode<ContextError>> {
    move |i: &mut &'i str| spaced_until(i, bare, |c| c == '|')
}

/// Whitespace, a matcher whose bare text stops at `stop`, whitespace.
fn spaced_until(i: &mut &str, bare: Bare, stop: fn(char) -> bool) -> ModalResult<TextMatcher> {
    ws(i)?;
    let m = matcher(i, bare, stop)?;
    ws(i)?;
    Ok(m)
}

/// `"*" | "/" regex "/" | "^"? ( '"' quoted '"' "$"? | bare )`. Bare text stops at a
/// `stop` character and is trimmed; empty bare text without anchors matches anything.
fn matcher(i: &mut &str, bare: Bare, stop: fn(char) -> bool) -> ModalResult<TextMatcher> {
    if opt('*').parse_next(i)?.is_some() {
        return Ok(TextMatcher::Any);
    }
    if i.starts_with('/') {
        return regex(i);
    }
    let start = opt('^').parse_next(i)?.is_some();
    if i.starts_with('"') {
        let literal = quoted(i)?;
        let end = opt('$').parse_next(i)?.is_some();
        return Ok(anchored(literal, start, end, Bare::Exact));
    }
    let raw = take_till(0.., stop).parse_next(i)?.trim();
    let (raw, end) = raw
        .strip_suffix('$')
        .map_or((raw, false), |stripped| (stripped, true));
    if raw.is_empty() && !start && !end {
        return Ok(TextMatcher::Any);
    }
    Ok(anchored(raw.to_owned(), start, end, bare).ignore_case())
}

/// The matcher for `literal` with `^` (`start`) and `$` (`end`) anchors.
fn anchored(literal: String, start: bool, end: bool, unanchored: Bare) -> TextMatcher {
    match (start, end, unanchored) {
        (true, true, _) | (false, false, Bare::Exact) => TextMatcher::exact(literal),
        (true, false, _) => TextMatcher::prefix(literal),
        (false, true, _) => TextMatcher::suffix(literal),
        (false, false, Bare::Substring) => TextMatcher::substring(literal),
    }
}

/// `"…"` with `\"` and `\\` escapes (a backslash escapes any character).
fn quoted(i: &mut &str) -> ModalResult<String> {
    '"'.parse_next(i)?;
    let mut text = String::new();
    loop {
        text.push_str(take_till(0.., ['"', '\\']).parse_next(i)?);
        if opt('\\').parse_next(i)?.is_some() {
            text.push(cut_err(any.context(expected("an escaped character"))).parse_next(i)?);
        } else {
            cut_err('"'.context(StrContext::Expected('"'.into()))).parse_next(i)?;
            return Ok(text);
        }
    }
}

/// `/…/`, where `\/` is a literal slash and other escapes go to the regex unchanged.
fn regex(i: &mut &str) -> ModalResult<TextMatcher> {
    let start = *i;
    '/'.parse_next(i)?;
    let mut pattern = String::new();
    loop {
        pattern.push_str(take_till(0.., ['/', '\\']).parse_next(i)?);
        if opt('\\').parse_next(i)?.is_some() {
            match cut_err(any.context(expected("an escaped character"))).parse_next(i)? {
                '/' => pattern.push('/'),
                c => {
                    pattern.push('\\');
                    pattern.push(c);
                }
            }
        } else {
            cut_err('/'.context(StrContext::Expected('/'.into()))).parse_next(i)?;
            break;
        }
    }
    TextMatcher::regex(&pattern).map_err(|err| {
        // Point at the opening slash.
        *i = start;
        ErrMode::Cut(ContextError::from_external_error(i, err))
    })
}

// --- Helpers -------------------------------------------------------------------------

fn ws<'i>(i: &mut &'i str) -> ModalResult<&'i str> {
    take_while(0.., char::is_whitespace).parse_next(i)
}

const fn expected(what: &'static str) -> StrContext {
    StrContext::Expected(StrContextValue::Description(what))
}

fn select<const N: usize>(ty: NodeType, predicates: [Option<Predicate>; N]) -> QueryPlan {
    QueryPlan::select(Selector {
        node_type: Some(ty),
        predicates: predicates.into_iter().flatten().collect(),
    })
}

const fn is_any(m: &TextMatcher) -> bool {
    matches!(m, TextMatcher::Any)
}

/// A text predicate, left out when it matches anything.
fn text(m: TextMatcher) -> Option<Predicate> {
    (!is_any(&m)).then_some(Predicate::Text(m))
}

/// An attribute predicate, left out when it matches anything (an absent attribute too).
fn attr(attribute: Attribute, m: TextMatcher) -> Option<Predicate> {
    (!is_any(&m)).then_some(Predicate::Attr(attribute, m))
}

/// A one-line message for a winnow error: the regex error that caused it, or what was
/// expected.
fn message(err: &ContextError) -> String {
    if let Some(cause) = err.cause() {
        return match cause.downcast_ref::<QueryError>() {
            // The regex crate's message is a multi-line report; its last line says
            // what is wrong.
            Some(QueryError::Regex { message, .. }) => {
                let reason = message
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or_default()
                    .trim();
                format!(
                    "invalid regex: {}",
                    reason.strip_prefix("error: ").unwrap_or(reason)
                )
            }
            _ => cause.to_string(),
        };
    }
    let expected: Vec<String> = err
        .context()
        .filter_map(|context| match context {
            StrContext::Expected(value) => Some(value.to_string()),
            _ => None,
        })
        .collect();
    match expected.as_slice() {
        [] => "unexpected input".to_owned(),
        [one] => format!("expected {one}"),
        [init @ .., last] => format!("expected {} or {last}", init.join(", ")),
    }
}
