//! MD013 `line-length`.

use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;
use crate::validate::reference_definitions;

use super::{expected_actual, js_len, lines, mark_lines, message};

/// Line length.
///
/// Lines longer than `line_length` (`heading_line_length` in headings,
/// `code_block_line_length` in code blocks) are reported, counted in UTF-16 units like
/// markdownlint. Unless `strict`, the last word may cross the limit if it starts within
/// it, and lines holding only a link or image (outside paragraph text) are exempt;
/// `stern` reports long lines unless they have no whitespace to wrap at. Reference
/// definitions are always exempt. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md013;

/// Options of MD013 `line-length`.
#[allow(clippy::struct_excessive_bools)] // markdownlint's option names
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md013Options {
    /// Number of characters.
    pub line_length: usize,
    /// Number of characters for headings; defaults to `line_length`.
    pub heading_line_length: Option<usize>,
    /// Number of characters for code blocks; defaults to `line_length`.
    pub code_block_line_length: Option<usize>,
    /// Include code blocks.
    pub code_blocks: bool,
    /// Include tables.
    pub tables: bool,
    /// Include headings.
    pub headings: bool,
    /// Strict length checking.
    pub strict: bool,
    /// Stern length checking.
    pub stern: bool,
}

impl Default for Md013Options {
    fn default() -> Self {
        Self {
            line_length: 80,
            heading_line_length: None,
            code_block_line_length: None,
            code_blocks: true,
            tables: true,
            headings: true,
            strict: false,
            stern: false,
        }
    }
}

/// A line without whitespace to wrap at (after a heading or quote prefix).
static NOT_WRAPPABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[#>\s]*\s)?\S*$").expect("valid regex"));

impl Rule for Md013 {
    type Options = Md013Options;

    const META: RuleMeta = RuleMeta {
        id: "MD013",
        aliases: &["line-length"],
        tags: &["line_length"],
        description: "Line length",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md013Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let count = doc.source().line_count();
        let mut headings = vec![false; count];
        let mut code = vec![false; count];
        let mut tables = vec![false; count];
        let mut links = vec![false; count];
        let mut paragraph_text = vec![false; count];
        let mut definitions = vec![false; count];
        for id in ast.descendants(ast.root()) {
            let span = ast.span(id);
            match ast.kind(id) {
                NodeKind::Heading(_) => mark_lines(doc, &mut headings, span),
                NodeKind::CodeBlock(_) => mark_lines(doc, &mut code, span),
                NodeKind::Table(_) => mark_lines(doc, &mut tables, span),
                NodeKind::Link(_) | NodeKind::Image(_) => mark_lines(doc, &mut links, span),
                NodeKind::Text(_)
                    if ast
                        .parent(id)
                        .is_some_and(|parent| matches!(ast.kind(parent), NodeKind::Paragraph)) =>
                {
                    mark_lines(doc, &mut paragraph_text, span);
                }
                _ => {}
            }
        }
        for definition in reference_definitions(doc) {
            mark_lines(doc, &mut definitions, definition.span);
        }
        let heading_length = options.heading_line_length.unwrap_or(options.line_length);
        let code_length = options
            .code_block_line_length
            .unwrap_or(options.line_length);
        let mut diagnostics = Vec::new();
        for (line, range) in lines(doc) {
            let content = &text[range.start..range.end];
            let maximum = if code[line] {
                code_length
            } else if headings[line] {
                heading_length
            } else {
                options.line_length
            };
            let link_only = links[line] && !paragraph_text[line];
            // Unless strict or stern, the last word may cross the limit.
            let length = if options.strict || options.stern {
                js_len(content)
            } else {
                let last_word =
                    content.len() - content.trim_end_matches(|c: char| !c.is_whitespace()).len();
                js_len(&content[..content.len() - last_word]) + 1
            };
            if maximum == 0
                || length <= maximum
                || (!options.code_blocks && code[line])
                || (!options.tables && tables[line])
                || (!options.headings && headings[line])
                || definitions[line]
                || (!options.strict
                    && ((options.stern && NOT_WRAPPABLE.is_match(content)) || link_only))
            {
                continue;
            }
            let start = range.start + byte_offset(content, maximum);
            let detail = expected_actual(maximum, js_len(content));
            diagnostics.push(Self::META.diagnostic(
                doc,
                ByteRange::new(start, range.end),
                message(&Self::META, Some(&detail), None),
            ));
        }
        diagnostics
    }
}

/// Byte offset of the character at UTF-16 index `index` in `text` (or the end).
fn byte_offset(text: &str, index: usize) -> usize {
    let mut units = 0;
    for (offset, c) in text.char_indices() {
        if units >= index {
            return offset;
        }
        units += c.len_utf16();
    }
    text.len()
}
