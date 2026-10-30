//! Document outlines: the headings of [`Document::toc`] rendered as indented text
//! ([`outline`]), as a nested Markdown list of anchor links ([`link_list`]), or as JSON
//! ([`JsonToc`]), and the edits that refresh `<!-- mrq:toc -->` regions
//! ([`refresh_edits`]).
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
use crate::ast::{Ast, Directive, NodeId, NodeKind, Span};
use crate::diag::{Diagnostic, Severity};
use crate::directive::{DIRECTIVE_ERROR_CODE, DirectiveKind, DirectiveRegion};
use crate::document::Document;
use crate::flavor;
use crate::source::{ByteRange, TextEdit};

/// What closes a directive region.
const END_MARKER: &str = "<!-- mrq:end -->";

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

/// The edits [`refresh_edits`] computed for a document, and what it could not refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TocRefresh {
    /// Edits against [`Document::text`]; empty when every region is up to date. Apply
    /// them with [`apply_edits`](crate::source::apply_edits).
    pub edits: Vec<TextEdit>,
    /// Problems with individual regions (code [`DIRECTIVE_ERROR_CODE`]); those regions
    /// get no edits.
    pub diagnostics: Vec<Diagnostic>,
}

/// Text edits that regenerate the body of every `<!-- mrq:toc -->` region of `doc`, as
/// `mrq toc --insert` does.
///
/// The body becomes the [`link_list`] of the document's headings, framed by blank lines;
/// no headings give an empty body. A region's `depth=N` attribute (1–6) limits the levels
/// listed and overrides `depth`; an invalid value is reported and its region left alone.
/// Headings inside a region's old body are not listed (nor counted when slugs are
/// deduplicated), so a refresh is idempotent: refreshing the result produces no edits.
///
/// Only the bytes between the end of the start marker's line and the start of the end
/// marker's line are replaced; a bare start marker gets the list and an `<!-- mrq:end
/// -->` inserted after its line. Inserted lines take the file's line ending and the
/// start marker's container prefix (`> ` in a blockquote, the item's indentation in a
/// list). Regions that are already up to date get no edit.
///
/// ```
/// use mrq_core::{Document, source::apply_edits, toc};
///
/// let src = "<!-- mrq:toc -->\n<!-- mrq:end -->\n\n# Guide\n";
/// let doc = Document::parse(src);
/// let refresh = toc::refresh_edits(&doc, None);
/// assert_eq!(
///     apply_edits(src, &refresh.edits).unwrap(),
///     "<!-- mrq:toc -->\n\n- [Guide](#guide)\n\n<!-- mrq:end -->\n\n# Guide\n"
/// );
/// ```
pub fn refresh_edits(doc: &Document, depth: Option<u8>) -> TocRefresh {
    let ast = doc.ast();
    let src = doc.text();
    let regions: Vec<&DirectiveRegion> = doc
        .directives()
        .iter()
        .filter(|region| region.kind == DirectiveKind::Toc)
        .collect();
    let bodies: Vec<Span> = regions.iter().map(|region| region.content).collect();
    let inside_body = |span: Span| {
        bodies
            .iter()
            .any(|body| !body.is_empty() && body.start <= span.start && span.end <= body.end)
    };
    let entries = live_entries(doc, &inside_body);
    let mut refresh = TocRefresh::default();
    for region in regions {
        let marker = ast.span(region.start);
        // A marker in the old body of another region disappears with that body.
        if inside_body(marker) {
            continue;
        }
        let depth = match region_depth(region.directive(ast), doc) {
            Ok(own) => own.or(depth),
            Err(diagnostic) => {
                refresh.diagnostics.push(*diagnostic);
                continue;
            }
        };
        let listed: Vec<TocEntry> = entries
            .iter()
            .filter(|entry| depth.is_none_or(|depth| entry.level <= depth))
            .cloned()
            .collect();
        if let Some(edit) = region_edit(src, region, marker, ast, &listed) {
            refresh.edits.push(edit);
        }
    }
    refresh
}

/// The document's headings outside old region bodies, slugged as if those were absent.
fn live_entries(doc: &Document, inside_body: &impl Fn(Span) -> bool) -> Vec<TocEntry> {
    let ast = doc.ast();
    let mut slugger = doc.flavor().slugger();
    flavor::heading_anchors_where(ast, &mut *slugger, |id| !inside_body(ast.span(id)))
        .into_iter()
        .filter_map(|(node, slug)| match ast.kind(node) {
            NodeKind::Heading(heading) => Some(TocEntry {
                node,
                level: heading.level,
                text: ast.text(node),
                slug,
                span: ast.span(node),
            }),
            _ => None,
        })
        .collect()
}

/// The region's `depth` attribute, if any.
fn region_depth(directive: &Directive, doc: &Document) -> Result<Option<u8>, Box<Diagnostic>> {
    let Some(attr) = directive
        .attributes
        .iter()
        .find(|attr| attr.name == "depth")
    else {
        return Ok(None);
    };
    match attr.value.parse::<u8>() {
        Ok(depth @ 1..=6) => Ok(Some(depth)),
        _ => Err(Box::new(
            Diagnostic::new(
                DIRECTIVE_ERROR_CODE,
                Severity::Error,
                doc.name(),
                attr.span,
                format!(
                    "`depth` must be a heading level from 1 to 6, not `{}`; the table of \
                     contents is not refreshed",
                    attr.value
                ),
            )
            .with_help("write e.g. `<!-- mrq:toc depth=3 -->`"),
        )),
    }
}

/// The edit making `region` list `entries`, or `None` if it already does.
fn region_edit(
    src: &str,
    region: &DirectiveRegion,
    marker: Span,
    ast: &Ast,
    entries: &[TocEntry],
) -> Option<TextEdit> {
    let line_start = |at: usize| src[..at].rfind('\n').map_or(0, |i| i + 1);
    let prefix: String = src[line_start(marker.start)..marker.start]
        .chars()
        .map(|c| {
            if matches!(c, '>' | ' ' | '\t') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let newline = src[marker.end..].find('\n').map(|i| marker.end + i);
    // The marker line's ending; the file's first one if it is the unterminated last line.
    let crlf = match newline {
        Some(i) => src[..i].ends_with('\r'),
        None => src.contains("\r\n"),
    };
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut body = String::new();
    if !entries.is_empty() {
        let blank = format!("{}{eol}", prefix.trim_end());
        body.push_str(&blank);
        for line in link_list(entries).lines() {
            let _ = write!(body, "{prefix}{line}{eol}");
        }
        body.push_str(&blank);
    }
    let body_start = newline.map_or(src.len(), |i| i + 1);
    let (range, replacement) = if let Some(end) = region.end {
        let end_line = line_start(ast.span(end).start);
        if end_line < body_start {
            // Both markers on one line: nothing can go between them.
            return None;
        }
        (ByteRange::new(body_start, end_line), body)
    } else {
        let lead = if newline.is_none() { eol } else { "" };
        let replacement = format!("{lead}{body}{prefix}{END_MARKER}{eol}");
        (ByteRange::empty(body_start), replacement)
    };
    (src[range.start..range.end] != replacement).then_some(TextEdit { range, replacement })
}
