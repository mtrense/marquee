//! Query errors: [`QueryError`], the positioned [`SyntaxError`] of the front-ends and the
//! [`RuntimeError`] of jq programs.

use std::path::PathBuf;

use crate::diag::{Diagnostic, Severity};
use crate::source::ByteRange;

/// Why a query could not be parsed or evaluated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryError {
    /// A `/regex/` matcher does not compile.
    #[error("invalid regex `{pattern}`: {message}")]
    Regex {
        /// The pattern as written.
        pattern: String,
        /// What is wrong with it.
        message: String,
    },
    /// The query text does not parse.
    #[error(transparent)]
    Syntax(#[from] SyntaxError),
    /// A jq program failed while it ran, e.g. `error("…")` or adding a string to a number.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// A jq output that [`Document::select`](crate::Document::select) cannot turn into a
    /// [`Match`](super::Match): it is not an unchanged node or section of the document.
    #[error("jq output {value} is not a node or section of the document")]
    NotANode {
        /// The output as compact JSON.
        value: String,
    },
}

/// A query that does not parse: a message about the byte at [`offset`](Self::offset) of
/// the [query text](Self::query).
///
/// [`render`](Self::render) shows the query with a caret under the offending character;
/// [`to_diagnostic`](Self::to_diagnostic) turns the error into a [`Diagnostic`] for a
/// query written inside a file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid query at offset {offset}: {message}")]
pub struct SyntaxError {
    query: String,
    offset: usize,
    message: String,
}

impl SyntaxError {
    /// An error about byte `offset` of `query`; `offset` is clamped to the query length.
    pub fn new(query: impl Into<String>, offset: usize, message: impl Into<String>) -> Self {
        let query = query.into();
        let offset = offset.min(query.len());
        Self {
            query,
            offset,
            message: message.into(),
        }
    }

    /// The query text as given.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Byte offset of the problem in the [query](Self::query); its length at the end.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// What is wrong, e.g. ``expected `)` ``.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The offending character in the query, or the empty range at its end.
    pub fn range(&self) -> ByteRange {
        let len = self.query[self.offset..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        ByteRange::new(self.offset, self.offset + len)
    }

    /// The line of the query that contains the offset, then a caret under the offending
    /// character followed by the message:
    ///
    /// ```text
    /// # A | ?
    ///       ^ expected a selector
    /// ```
    ///
    /// The caret column counts characters, so it lines up for non-ASCII queries in a
    /// monospace terminal (wide characters aside).
    pub fn render(&self) -> String {
        let line_start = self.query[..self.offset].rfind('\n').map_or(0, |i| i + 1);
        let line_end = self.query[self.offset..]
            .find('\n')
            .map_or(self.query.len(), |i| self.offset + i);
        let column = self.query[line_start..self.offset].chars().count();
        format!(
            "{}\n{}^ {}",
            &self.query[line_start..line_end],
            " ".repeat(column),
            self.message
        )
    }

    /// A [`Diagnostic`] (code `query`, error) for a query whose text starts at byte
    /// `origin` of `file`, e.g. the `select="…"` value of an include directive.
    pub fn to_diagnostic(&self, file: impl Into<PathBuf>, origin: usize) -> Diagnostic {
        let range = self.range();
        Diagnostic::new(
            "query",
            Severity::Error,
            file,
            ByteRange::new(origin + range.start, origin + range.end),
            format!("invalid query: {}", self.message),
        )
    }
}

/// A jq program that failed while it ran: the [message](Self::message) of the error it
/// raised, about the whole [query text](Self::query).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("jq error: {message}")]
pub struct RuntimeError {
    query: String,
    message: String,
}

impl RuntimeError {
    /// An error with `message` raised by the program `query`.
    pub fn new(query: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            message: message.into(),
        }
    }

    /// The query text as given.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// What went wrong: the value given to `error` (strings as is, other values as
    /// JSON) or jaq's description, e.g. `cannot use "heading" (string) as number`.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// A [`Diagnostic`] (code `query`, error) covering the whole query, for a query whose
    /// text starts at byte `origin` of `file`.
    pub fn to_diagnostic(&self, file: impl Into<PathBuf>, origin: usize) -> Diagnostic {
        Diagnostic::new(
            "query",
            Severity::Error,
            file,
            ByteRange::new(origin, origin + self.query.len()),
            self.to_string(),
        )
    }
}
