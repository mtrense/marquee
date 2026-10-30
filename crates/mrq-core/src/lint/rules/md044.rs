//! MD044 `proper-names`.

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{LinkForm, NodeKind, Span};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, expected_actual, message, span_lines};

/// Proper names should have the correct capitalization.
///
/// Each of `names` found in other capitalization (ignoring case, as a whole word unless
/// the name starts or ends with a non-word character, underscores allowed around it) is
/// reported, longer names first; a match overlapping an earlier one is skipped. Text,
/// image alt text and link text are searched, code blocks and spans with `code_blocks`,
/// HTML (comments included) with `html_elements`; link destinations, autolinks, info
/// strings and front matter are not. Like markdownlint's text tokens, text is split at
/// runs of `_`. The fix writes the name as configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md044;

/// Options of MD044 `proper-names`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md044Options {
    /// List of proper names.
    pub names: Vec<String>,
    /// Include code blocks.
    pub code_blocks: bool,
    /// Include HTML elements.
    pub html_elements: bool,
}

impl Default for Md044Options {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            code_blocks: true,
            html_elements: true,
        }
    }
}

impl Rule for Md044 {
    type Options = Md044Options;

    const META: RuleMeta = RuleMeta {
        id: "MD044",
        aliases: &["proper-names"],
        tags: &["spelling"],
        description: "Proper names should have the correct capitalization",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md044Options) -> Vec<Diagnostic> {
        let mut names: Vec<&str> = options.names.iter().map(String::as_str).collect();
        names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        if names.is_empty() {
            return Vec::new();
        }
        let text = doc.text();
        let tokens = tokens(doc, options);
        let mut seen: Vec<ByteRange> = Vec::new();
        let mut diagnostics = Vec::new();
        for &name in &names {
            let Some(pattern) = name_pattern(name) else {
                continue;
            };
            for token in &tokens {
                let haystack = &text[token.start..token.end];
                for captures in pattern.captures_iter(haystack) {
                    let Some(found) = captures.get(2) else {
                        continue;
                    };
                    let range =
                        ByteRange::new(token.start + found.start(), token.start + found.end());
                    let overlaps = seen
                        .iter()
                        .any(|other| other.start < range.end && range.start < other.end);
                    if !names.contains(&found.as_str()) && !overlaps {
                        let detail = expected_actual(name, found.as_str());
                        diagnostics.push(
                            Self::META
                                .diagnostic(doc, range, message(&Self::META, Some(&detail), None))
                                .with_fix(Fix::new(
                                    format!("Write {name}"),
                                    vec![edit(range, name)],
                                )),
                        );
                    }
                    seen.push(range);
                }
            }
        }
        diagnostics
    }
}

/// markdownlint's name pattern: the name ignoring case, with `_`s and an (ASCII) word
/// boundary on each side that is a word character.
fn name_pattern(name: &str) -> Option<Regex> {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let start = if word(name.chars().next()) {
        r"(?-u:\b)_*"
    } else {
        ""
    };
    let end = if word(name.chars().last()) {
        r"_*(?-u:\b)"
    } else {
        ""
    };
    Regex::new(&format!("(?i)({start})({}){end}", regex::escape(name))).ok()
}

/// The source ranges markdownlint searches (its `data`, code and HTML tokens), each on
/// one line, in document order.
fn tokens(doc: &Document, options: &Md044Options) -> Vec<ByteRange> {
    let ast = doc.ast();
    let text = doc.text();
    let source = doc.source();
    let mut tokens = Vec::new();
    // `start..end` split into lines.
    let lines_of = |tokens: &mut Vec<ByteRange>, start: usize, end: usize| {
        let (first, last) = span_lines(doc, Span::new(start, end));
        for line in first..=last {
            if let Some(range) = source.line_range(line) {
                let from = range.start.max(start);
                let to = range.end.min(end);
                if from < to {
                    tokens.push(ByteRange::new(from, to));
                }
            }
        }
    };
    for id in ast.descendants(ast.root()) {
        let span = ast.span(id);
        match ast.kind(id) {
            NodeKind::Text(_) => {
                let autolink = ast.ancestors(id).any(|id| {
                    matches!(ast.kind(id), NodeKind::Link(link)
                        if matches!(link.form, LinkForm::Autolink | LinkForm::Bare))
                });
                if autolink {
                    continue;
                }
                // Runs of `_` are tokens of their own.
                let mut start = span.start;
                for (at, c) in text[span.start..span.end].char_indices() {
                    let at = span.start + at;
                    let underscore = c == '_';
                    let previous = text[..at].ends_with('_');
                    if at > start && underscore != previous {
                        tokens.push(ByteRange::new(start, at));
                        start = at;
                    }
                }
                lines_of(&mut tokens, start, span.end);
            }
            NodeKind::Code(_) if options.code_blocks => {
                let source = &text[span.start..span.end];
                let ticks = source.len() - source.trim_start_matches('`').len();
                if source.len() >= 2 * ticks {
                    lines_of(&mut tokens, span.start + ticks, span.end - ticks);
                }
            }
            NodeKind::CodeBlock(code) if options.code_blocks => {
                let (first, last) = span_lines(doc, span);
                let first = first + usize::from(code.fence.is_some());
                for line in first..=last {
                    let Some(range) = source.line_range(line) else {
                        continue;
                    };
                    let content = text[range.start..range.end].trim_start_matches([' ', '>']);
                    let closing = code.fence.is_some_and(|fence| {
                        line == last
                            && content.trim_end().len() >= fence.len
                            && content.trim_end().chars().all(|c| c == fence.char)
                    });
                    if !closing {
                        tokens.push(range);
                    }
                }
            }
            NodeKind::HtmlInline(_)
            | NodeKind::HtmlBlock(_)
            | NodeKind::Comment(_)
            | NodeKind::IncludeDirective(_)
            | NodeKind::TocDirective(_)
            | NodeKind::DirectiveEnd
                if options.html_elements =>
            {
                lines_of(&mut tokens, span.start, span.end);
            }
            _ => {}
        }
    }
    tokens
}
