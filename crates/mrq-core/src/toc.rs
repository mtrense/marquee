//! Document outlines: the headings of [`Document::toc`] rendered as indented text
//! ([`outline`]), as a nested Markdown list of anchor links ([`link_list`]), or as JSON
//! ([`JsonToc`]).
//!
//! ```
//! use mrq_core::{Document, toc};
//!
//! let doc = Document::parse("# Guide\n\n## Install\n\n## Use it\n");
//! assert_eq!(toc::outline(&doc.toc()), "Guide\n  Install\n  Use it\n");
//! assert_eq!(
//!     toc::link_list(&doc.toc()),
//!     "- [Guide](#guide)\n  - [Install](#install)\n  - [Use it](#use-it)\n"
//! );
//! ```
//!
//! # Nesting
//!
//! An entry nests one step under the closest preceding entry of a lower level, whatever
//! the gap in between: `####` directly under `#` is indented once, not three times (the
//! deeper indentation would turn a Markdown list item into an indented code block).
//! Entries with no lower-level entry before them are at the top, so an outline may start
//! at any level. Limit the depth by filtering entries on [`TocEntry::level`].

use std::fmt::Write as _;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ast::json::{self, Position};
use crate::ast::{NodeId, Span};
use crate::document::Document;

/// Indentation per nesting step, in both renderings.
const INDENT: &str = "  ";

/// One heading of a document's [table of contents](Document::toc).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TocEntry {
    /// The heading node.
    pub node: NodeId,
    /// 1–6.
    pub level: u8,
    /// Plain text ([`Ast::text`](crate::ast::Ast::text)).
    pub text: String,
    /// The anchor, from the [`AnchorIndex`](crate::document::AnchorIndex).
    pub slug: String,
    /// The heading's source span.
    pub span: Span,
}

/// The entries' texts, one per line, indented two spaces per [nesting](self#nesting)
/// step. Empty for no entries.
pub fn outline(entries: &[TocEntry]) -> String {
    render(entries, |entry| entry.text.clone())
}

/// A nested Markdown bullet list linking each entry's text to `#slug`, two spaces of
/// indentation per [nesting](self#nesting) step. Link text is escaped so it reads back
/// as the heading's plain text; the anchor is percent-encoded where Markdown or URLs
/// need it. Empty for no entries.
pub fn link_list(entries: &[TocEntry]) -> String {
    render(entries, |entry| {
        format!(
            "- [{}](#{})",
            escape_link_text(&entry.text),
            encode_anchor(&entry.slug)
        )
    })
}

/// One line per entry, indented by nesting depth.
fn render(entries: &[TocEntry], line: impl Fn(&TocEntry) -> String) -> String {
    let mut out = String::new();
    // Levels of the open ancestors of the current entry.
    let mut open: Vec<u8> = Vec::new();
    for entry in entries {
        while open.last().is_some_and(|&level| level >= entry.level) {
            open.pop();
        }
        out.push_str(&INDENT.repeat(open.len()));
        out.push_str(&line(entry));
        out.push('\n');
        open.push(entry.level);
    }
    out
}

/// Backslash-escapes what would otherwise start inline syntax in link text: brackets,
/// backslashes, code spans, emphasis, autolinks/HTML, and `_` unless it is inside a word
/// (where it cannot open or close emphasis).
fn escape_link_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let escape = match c {
            '\\' | '[' | ']' | '`' | '*' | '<' => true,
            '_' => {
                let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
                !(i > 0 && word(chars.get(i - 1)) && word(chars.get(i + 1)))
            }
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Percent-encodes the characters that cannot appear verbatim in an inline link
/// destination (whitespace, controls, parentheses, angle brackets, backslash) plus `%`.
fn encode_anchor(slug: &str) -> String {
    let mut out = String::with_capacity(slug.len());
    for c in slug.chars() {
        if c.is_whitespace() || c.is_control() || matches!(c, '(' | ')' | '<' | '>' | '\\' | '%') {
            let mut buf = [0; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{byte:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The JSON form of one document's outline, as printed by `mrq toc --json` (an array of
/// these, one per input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct JsonToc {
    /// The document's path as given; `null` for stdin and in-memory text.
    pub file: Option<String>,
    /// Name of the flavor the document was parsed with, whose slugger made the slugs.
    pub flavor: String,
    /// The headings in document order.
    pub headings: Vec<JsonHeading>,
}

/// One heading in a [`JsonToc`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct JsonHeading {
    /// 1–6.
    pub level: u8,
    /// Plain text.
    pub text: String,
    /// The anchor.
    pub slug: String,
    /// The heading's source range, as in the [AST projection](crate::ast::json#positions).
    pub position: Position,
}

impl JsonToc {
    /// The outline `entries` of `doc` (usually [`Document::toc`], possibly filtered).
    pub fn new(doc: &Document, entries: &[TocEntry]) -> Self {
        Self {
            file: doc.path().map(|path| path.display().to_string()),
            flavor: doc.flavor().name().to_owned(),
            headings: entries
                .iter()
                .map(|entry| JsonHeading {
                    level: entry.level,
                    text: entry.text.clone(),
                    slug: entry.slug.clone(),
                    position: json::position(doc.source(), entry.span),
                })
                .collect(),
        }
    }
}
