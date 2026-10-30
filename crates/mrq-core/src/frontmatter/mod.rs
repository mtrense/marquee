//! Frontmatter: detecting, parsing and locating YAML and TOML metadata blocks.
//!
//! A document may start with a metadata block between delimiter lines: `---` for YAML,
//! `+++` for TOML. [`Frontmatter::extract`] finds it, parses it into a
//! [`serde_json::Value`] and indexes the source spans of every key and value, so later
//! stages can read metadata ([`Frontmatter::get`]) and point diagnostics at it
//! ([`Frontmatter::entry`], [`Frontmatter::entry_at_pointer`]).
//!
//! ```
//! use mrq_core::frontmatter::Frontmatter;
//!
//! let src = "---\ntitle: Hello\nmrq:\n  flavor: obsidian\n---\n# Body\n";
//! let fm = Frontmatter::extract(src).unwrap();
//! assert_eq!(fm.get_str(&["mrq", "flavor"]), Some("obsidian"));
//! let entry = fm.entry(&["mrq", "flavor"]).unwrap();
//! assert_eq!(&src[entry.key.unwrap().start..entry.value.end], "flavor: obsidian");
//! assert_eq!(&src[fm.body_start()..], "# Body\n");
//! ```
//!
//! # Detection
//!
//! The opening delimiter must be the first line, after an optional byte order mark, and the
//! closing delimiter is the next line made of the same three characters. Trailing spaces
//! and tabs are allowed on both lines; `\n` and `\r\n` line endings are both recognized.
//! Without a closing line there is no frontmatter (and `---` is a thematic break).
//! Detection does not depend on the flavor: the flavor itself can be chosen in the
//! frontmatter (`mrq.flavor`).
//!
//! # Values
//!
//! YAML is read with serde-saphyr (duplicate keys are errors), TOML with `toml_edit`.
//! TOML datetimes become strings, and so do non-finite floats (`nan`, `inf`, `-inf`),
//! which JSON cannot represent. Empty frontmatter, or YAML with only comments, is an empty
//! mapping. A block that does not parse, or whose top level is not a mapping, is still
//! detected (its body starts after it) but has no value; its [`FrontmatterError`] turns
//! into a [`Diagnostic`] with code [`FRONTMATTER_ERROR_CODE`].
//!
//! # Spans
//!
//! All spans are byte ranges into the full source passed to [`Frontmatter::extract`].
//! Entries are addressed by key path: mapping keys as written, array items by their
//! decimal index; the empty path is the whole mapping. An [`EntrySpan`] has the span of
//! the key (for mapping entries) and of the value:
//!
//! - Scalars span their source text, quotes included; block scalars (`|`, `>`) span their
//!   content lines, without the indicator.
//! - YAML collections span from their first token to their last, closing bracket included
//!   for flow collections. YAML aliases span the alias (`*name`); keys pulled in by merge
//!   keys (`<<`) have no entry.
//! - TOML tables declared with a header span the header (`[a]`, `[[a]]`), implicit tables
//!   (`a` in `a.b = 1`) span their key.
//!
//! # Integration with the tree
//!
//! The Markdown parser never sees the frontmatter. [`parse_document`] parses the body on
//! its own and assembles a tree whose first child is a
//! [`FrontMatter`](crate::ast::NodeKind::FrontMatter) node, with every span relative to
//! the full source.

mod detect;
mod toml;
pub(crate) mod yaml;

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::ast::{Ast, FrontMatter, FrontMatterFormat, NodeKind, Span};
use crate::diag::{Diagnostic, Severity};
use crate::parse::Parser;

/// Diagnostic code of malformed frontmatter.
pub const FRONTMATTER_ERROR_CODE: &str = "frontmatter";

/// Source spans of one frontmatter entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntrySpan {
    /// The key, for mapping entries; `None` for array items and the root.
    pub key: Option<Span>,
    /// The value.
    pub value: Span,
}

impl EntrySpan {
    const fn new(key: Option<Span>, value: Span) -> Self {
        Self { key, value }
    }

    const fn offset(self, by: usize) -> Self {
        Self {
            key: match self.key {
                Some(key) => Some(shift(key, by)),
                None => None,
            },
            value: shift(self.value, by),
        }
    }
}

/// Why a frontmatter block has no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterError {
    /// What is wrong, e.g. `invalid YAML frontmatter: unclosed bracket '['`.
    pub message: String,
    /// The offending token, or the whole content when the top level is not a mapping.
    pub range: Span,
}

impl FrontmatterError {
    /// This error as a diagnostic about `file`.
    pub fn to_diagnostic(&self, file: impl Into<PathBuf>) -> Diagnostic {
        Diagnostic::new(
            FRONTMATTER_ERROR_CODE,
            Severity::Error,
            file,
            self.range,
            self.message.clone(),
        )
    }
}

/// Key path → entry spans.
type Index = HashMap<Vec<String>, EntrySpan>;

/// A parsed block: its value and the spans of its entries.
#[derive(Debug, Clone)]
struct Parsed {
    value: Value,
    entries: Index,
}

/// A frontmatter block found at the start of a document.
#[derive(Debug, Clone)]
pub struct Frontmatter {
    format: FrontMatterFormat,
    span: Span,
    content: Span,
    body_start: usize,
    data: Result<Parsed, FrontmatterError>,
}

impl Frontmatter {
    /// Finds and parses the frontmatter at the start of `src`; `None` if there is none.
    /// Malformed frontmatter is still returned, with an [`error`](Self::error).
    pub fn extract(src: &str) -> Option<Self> {
        let block = detect::detect(src)?;
        let text = &src[block.content.start..block.content.end];
        let parsed = match block.format {
            FrontMatterFormat::Toml => toml::parse(text),
            _ => yaml::parse(text),
        };
        let data = parsed.and_then(|parsed| {
            if parsed.value.is_object() {
                Ok(parsed)
            } else {
                Err(FrontmatterError {
                    message: "frontmatter must be a mapping of keys to values".to_owned(),
                    range: trimmed(text, Span::new(0, text.len())),
                })
            }
        });
        let offset = block.content.start;
        let data = match data {
            Ok(Parsed { value, entries }) => Ok(Parsed {
                value,
                entries: entries
                    .into_iter()
                    .map(|(path, entry)| (path, entry.offset(offset)))
                    .collect(),
            }),
            Err(err) => Err(FrontmatterError {
                range: shift(err.range, offset),
                ..err
            }),
        };
        Some(Self {
            format: block.format,
            span: block.span,
            content: block.content,
            body_start: block.body_start,
            data,
        })
    }

    /// YAML or TOML.
    pub const fn format(&self) -> FrontMatterFormat {
        self.format
    }

    /// The whole block, from the opening delimiter to the closing one (without the line
    /// ending after it). Starts after a byte order mark.
    pub const fn span(&self) -> Span {
        self.span
    }

    /// The text between the delimiter lines, including its final line ending.
    pub const fn content(&self) -> Span {
        self.content
    }

    /// Offset where the Markdown body starts: after the closing delimiter's line ending.
    pub const fn body_start(&self) -> usize {
        self.body_start
    }

    /// The parsed mapping; `None` if the frontmatter is malformed.
    pub fn value(&self) -> Option<&Value> {
        self.data.as_ref().ok().map(|parsed| &parsed.value)
    }

    /// Why the frontmatter has no value.
    pub fn error(&self) -> Option<&FrontmatterError> {
        self.data.as_ref().err()
    }

    /// The [`error`](Self::error) as a diagnostic about `file`.
    pub fn diagnostic(&self, file: impl Into<PathBuf>) -> Option<Diagnostic> {
        self.error().map(|err| err.to_diagnostic(file))
    }

    /// The value at a key path (array items by decimal index).
    pub fn get<S: AsRef<str>>(&self, path: &[S]) -> Option<&Value> {
        path.iter().try_fold(self.value()?, |value, segment| {
            let segment = segment.as_ref();
            match value {
                Value::Object(map) => map.get(segment),
                Value::Array(items) => items.get(segment.parse::<usize>().ok()?),
                _ => None,
            }
        })
    }

    /// The string at a key path, e.g. `get_str(&["mrq", "flavor"])`.
    pub fn get_str<S: AsRef<str>>(&self, path: &[S]) -> Option<&str> {
        self.get(path)?.as_str()
    }

    /// The value at a JSON Pointer (RFC 6901), e.g. `/tags/0`.
    pub fn pointer(&self, pointer: &str) -> Option<&Value> {
        self.value()?.pointer(pointer)
    }

    /// Source spans of the entry at a key path; `None` for missing paths and malformed
    /// frontmatter.
    pub fn entry<S: AsRef<str>>(&self, path: &[S]) -> Option<EntrySpan> {
        let entries = &self.data.as_ref().ok()?.entries;
        let path: Vec<String> = path.iter().map(|s| s.as_ref().to_owned()).collect();
        entries.get(&path).copied()
    }

    /// Source spans of the entry at a JSON Pointer, such as a JSON Schema instance path.
    pub fn entry_at_pointer(&self, pointer: &str) -> Option<EntrySpan> {
        self.entry(&pointer_segments(pointer)?)
    }
}

/// Parses `src` with `parser`, splitting off frontmatter first.
///
/// Without frontmatter this is `parser.parse(src)`. Otherwise the body is parsed on its
/// own, and the returned tree's root (spanning all of `src`) has a
/// [`FrontMatter`](crate::ast::NodeKind::FrontMatter) node first, followed by the body's
/// blocks with spans shifted into `src`. Malformed frontmatter still gets its node.
pub fn parse_document<P: Parser + ?Sized>(src: &str, parser: &P) -> (Ast, Option<Frontmatter>) {
    let Some(frontmatter) = Frontmatter::extract(src) else {
        return (parser.parse(src), None);
    };
    let offset = frontmatter.body_start();
    let body = parser.parse(&src[offset..]);

    let mut ast = Ast::new(Span::new(0, src.len()));
    let root = ast.root();
    let content = frontmatter.content();
    let node = ast.create(
        NodeKind::FrontMatter(FrontMatter {
            format: frontmatter.format(),
            literal: src[content.start..content.end].to_owned(),
        }),
        frontmatter.span(),
    );
    ast.append_child(root, node)
        .expect("a new node is detached");
    for &child in body.children(body.root()) {
        let copy = ast.import(&body, child, |span| shift(span, offset));
        ast.append_child(root, copy)
            .expect("an imported node is detached");
    }
    (ast, Some(frontmatter))
}

/// The segments of a JSON Pointer, unescaped; `None` if it is not a pointer.
fn pointer_segments(pointer: &str) -> Option<Vec<String>> {
    if pointer.is_empty() {
        return Some(Vec::new());
    }
    let rest = pointer.strip_prefix('/')?;
    Some(
        rest.split('/')
            .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
            .collect(),
    )
}

const fn shift(span: Span, by: usize) -> Span {
    Span::new(span.start + by, span.end + by)
}

/// `span` of `text` without trailing whitespace (and line endings).
fn trimmed(text: &str, span: Span) -> Span {
    let trimmed = text[span.start..span.end].trim_end();
    Span::new(span.start, span.start + trimmed.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_segments_unescape() {
        assert_eq!(pointer_segments(""), Some(vec![]));
        assert_eq!(pointer_segments("/"), Some(vec![String::new()]));
        assert_eq!(
            pointer_segments("/a~1b/~01/0"),
            Some(vec!["a/b".to_owned(), "~1".to_owned(), "0".to_owned()])
        );
        assert_eq!(pointer_segments("a"), None);
    }

    #[test]
    fn get_walks_objects_and_arrays() {
        let fm = Frontmatter::extract("---\na:\n  - b: 1\n---\n").unwrap();
        assert_eq!(fm.get(&["a", "0", "b"]), Some(&Value::from(1)));
        assert_eq!(fm.get(&["a", "x"]), None);
        assert_eq!(fm.get(&["a", "0", "b", "c"]), None);
        assert_eq!(fm.get_str(&["a", "0", "b"]), None);
    }
}
