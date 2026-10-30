//! The syntax inside a directive comment: `mrq:<name>` followed by attributes.
//!
//! Offsets are byte offsets into the comment's literal (the text between `<!--` and
//! `-->`); the caller maps them to the source.

use std::ops::Range;

/// Which directive a comment names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Name {
    Include,
    Toc,
    End,
}

/// A well-formed directive comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Syntax {
    pub name: Name,
    pub attributes: Vec<RawAttribute>,
}

/// One attribute, with its range in the literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawAttribute {
    pub name: String,
    pub value: String,
    pub range: Range<usize>,
}

/// Why a directive comment is malformed, and where in the literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SyntaxError {
    pub range: Range<usize>,
    pub message: String,
}

/// The prefix that makes a comment a directive (after leading whitespace).
const PREFIX: &str = "mrq:";

/// Parses a comment literal. `None` when it is not a directive (does not start with
/// `mrq:` after whitespace); an error when it is one but malformed.
pub(super) fn parse(literal: &str) -> Option<Result<Syntax, SyntaxError>> {
    let start = literal.len() - literal.trim_start().len();
    literal[start..].starts_with(PREFIX).then(|| {
        let end = token_end(literal, start);
        let name = match &literal[start + PREFIX.len()..end] {
            "include" => Name::Include,
            "toc" => Name::Toc,
            "end" => Name::End,
            "" => return Err(error(start..end, "expected a directive name after `mrq:`")),
            other => {
                return Err(error(
                    start..end,
                    format!(
                        "unknown directive `mrq:{other}` (expected `mrq:include`, `mrq:toc` \
                         or `mrq:end`)"
                    ),
                ));
            }
        };
        let attributes = Attributes { literal, at: end }.collect::<Result<_, _>>()?;
        Ok(Syntax { name, attributes })
    })
}

/// The end of the token starting at `start`: the next whitespace or the end.
fn token_end(literal: &str, start: usize) -> usize {
    literal[start..]
        .find(char::is_whitespace)
        .map_or(literal.len(), |n| start + n)
}

fn error(range: Range<usize>, message: impl Into<String>) -> SyntaxError {
    SyntaxError {
        range,
        message: message.into(),
    }
}

const fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Iterates over the attributes of a literal from `at`, stopping after the first error.
///
/// An attribute is `name=value`, `name="value"`, `name='value'` or a bare `name` (whose
/// value is `true`). Names are ASCII letters, digits, `_` and `-`. Unquoted values run to
/// the next whitespace. In quoted values `\"`, `\'` and `\\` stand for the character
/// after the backslash; any other backslash is kept.
struct Attributes<'a> {
    literal: &'a str,
    at: usize,
}

impl Attributes<'_> {
    fn next_attribute(&mut self) -> Result<RawAttribute, SyntaxError> {
        let literal = self.literal;
        let bytes = literal.as_bytes();
        let start = self.at;
        let mut i = start;
        while i < bytes.len() && is_name_byte(bytes[i]) {
            i += 1;
        }
        let name = &literal[start..i];
        let after_name = literal[i..].chars().next();
        if name.is_empty() {
            let c = after_name.expect("called before the end");
            let range = i..i + c.len_utf8();
            return Err(if c == '=' {
                error(range, "expected an attribute name before `=`")
            } else {
                error(range, format!("unexpected `{c}` in directive attributes"))
            });
        }
        match after_name {
            None => return Ok(self.finish(name, "true".into(), start, i)),
            Some(c) if c.is_whitespace() => {
                return Ok(self.finish(name, "true".into(), start, i));
            }
            Some('=') => i += 1,
            Some(c) => {
                return Err(error(
                    i..i + c.len_utf8(),
                    format!("unexpected `{c}` in attribute name `{name}`"),
                ));
            }
        }
        let (value, end) = match literal[i..].chars().next() {
            None => return Err(error(start..i, format!("missing value for `{name}`"))),
            Some(c) if c.is_whitespace() => {
                return Err(error(start..i, format!("missing value for `{name}`")));
            }
            Some(quote @ ('"' | '\'')) => quoted(literal, i, quote)?,
            Some(_) => {
                let end = token_end(literal, i);
                (literal[i..end].to_owned(), end)
            }
        };
        if let Some(c) = literal[end..].chars().next().filter(|c| !c.is_whitespace()) {
            return Err(error(
                end..end + c.len_utf8(),
                format!("expected whitespace after the value of `{name}`, found `{c}`"),
            ));
        }
        Ok(self.finish(name, value, start, end))
    }

    fn finish(&mut self, name: &str, value: String, start: usize, end: usize) -> RawAttribute {
        self.at = end;
        RawAttribute {
            name: name.to_owned(),
            value,
            range: start..end,
        }
    }
}

impl Iterator for Attributes<'_> {
    type Item = Result<RawAttribute, SyntaxError>;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = &self.literal[self.at..];
        self.at += rest.len() - rest.trim_start().len();
        if self.at >= self.literal.len() {
            return None;
        }
        let item = self.next_attribute();
        if item.is_err() {
            self.at = self.literal.len();
        }
        Some(item)
    }
}

/// The unescaped value of the quoted string opening at `open`, and the offset after its
/// closing quote.
fn quoted(literal: &str, open: usize, quote: char) -> Result<(String, usize), SyntaxError> {
    let mut value = String::new();
    let mut chars = literal[open + 1..].char_indices();
    while let Some((n, c)) = chars.next() {
        match c {
            '\\' => match chars.clone().next() {
                Some((_, next)) if next == quote || next == '\\' => {
                    value.push(next);
                    chars.next();
                }
                _ => value.push('\\'),
            },
            c if c == quote => return Ok((value, open + 1 + n + 1)),
            c => value.push(c),
        }
    }
    Err(error(
        open..literal.len(),
        format!("unterminated quote `{quote}`"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attributes(literal: &str) -> Vec<(String, String, &str)> {
        let syntax = parse(literal).unwrap().unwrap();
        syntax
            .attributes
            .into_iter()
            .map(|a| (a.name, a.value, &literal[a.range]))
            .collect()
    }

    fn err(literal: &str) -> (String, &str) {
        let err = parse(literal).unwrap().unwrap_err();
        (err.message, &literal[err.range])
    }

    #[test]
    fn only_mrq_comments_are_directives() {
        assert_eq!(parse(" note "), None);
        assert_eq!(parse(" MRQ:toc "), None);
        assert_eq!(parse(" x mrq:toc "), None);
        let toc = parse("\n mrq:toc").unwrap().unwrap();
        assert_eq!((toc.name, toc.attributes), (Name::Toc, vec![]));
        assert_eq!(parse("mrq:end").unwrap().unwrap().name, Name::End);
        assert_eq!(parse(" mrq:include ").unwrap().unwrap().name, Name::Include);
    }

    #[test]
    fn values_unescape_only_quotes_and_backslashes() {
        assert_eq!(
            attributes(r#" mrq:toc a="x\"y" b='\'\n' c="é ü" d=é"#),
            [
                ("a".into(), "x\"y".into(), r#"a="x\"y""#),
                ("b".into(), r"'\n".into(), r"b='\'\n'"),
                ("c".into(), "é ü".into(), "c=\"é ü\""),
                ("d".into(), "é".into(), "d=é"),
            ]
        );
        // A quote of the other kind needs no escape; a trailing backslash is kept.
        assert_eq!(
            attributes(r#"mrq:toc a="it's" b='\\' c="\"""#),
            [
                ("a".into(), "it's".into(), r#"a="it's""#),
                ("b".into(), r"\".into(), r"b='\\'"),
                ("c".into(), "\"".into(), r#"c="\"""#),
            ]
        );
    }

    #[test]
    fn errors_point_at_the_offending_text() {
        assert_eq!(
            err(" mrq:toc a=\"x\\\""),
            ("unterminated quote `\"`".into(), "\"x\\\"")
        );
        assert_eq!(
            err(" mrq:toc a é"),
            ("unexpected `é` in directive attributes".into(), "é")
        );
        assert_eq!(
            err(" mrq:toc a:b=1"),
            ("unexpected `:` in attribute name `a`".into(), ":")
        );
        assert_eq!(err(" mrq:toc a="), ("missing value for `a`".into(), "a="));
        assert_eq!(
            err(" mrq:toc a='b'c"),
            (
                "expected whitespace after the value of `a`, found `c`".into(),
                "c"
            )
        );
        assert_eq!(
            err(" mrq:tocx "),
            (
                "unknown directive `mrq:tocx` (expected `mrq:include`, `mrq:toc` or `mrq:end`)"
                    .into(),
                "mrq:tocx"
            )
        );
    }
}
