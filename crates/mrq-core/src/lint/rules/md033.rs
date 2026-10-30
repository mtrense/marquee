//! MD033 `no-inline-html`.

use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{message, raw_html};

/// Inline HTML.
///
/// Every opening HTML tag is reported, inline or in an HTML block (whose text
/// markdownlint parses again as inline content), unless its element is in
/// `allowed_elements` (`table_allowed_elements` in tables, which defaults to
/// `allowed_elements`), ignoring case. Comments and closing tags are fine. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md033;

/// Options of MD033 `no-inline-html`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md033Options {
    /// Allowed elements.
    pub allowed_elements: Vec<String>,
    /// Allowed elements in tables; defaults to `allowed_elements`.
    pub table_allowed_elements: Option<Vec<String>>,
}

/// markdownlint's `getHtmlTagInfo` name pattern.
static TAG_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^<([^!>][^/\s>]*)").expect("valid regex"));

impl Rule for Md033 {
    type Options = Md033Options;

    const META: RuleMeta = RuleMeta {
        id: "MD033",
        aliases: &["no-inline-html"],
        tags: &["html"],
        description: "Inline HTML",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md033Options) -> Vec<Diagnostic> {
        let lower = |names: &[String]| -> Vec<String> {
            names.iter().map(|name| name.to_lowercase()).collect()
        };
        let allowed = lower(&options.allowed_elements);
        let table_allowed = options
            .table_allowed_elements
            .as_deref()
            .map_or_else(|| allowed.clone(), lower);
        let ast = doc.ast();
        let text = doc.text();
        // Each opening tag as `(range, in a table)`.
        let mut tags = Vec::new();
        for id in ast.descendants(ast.root()) {
            let span = ast.span(id);
            match ast.kind(id) {
                NodeKind::HtmlInline(_) => {
                    let in_table = ast
                        .ancestors(id)
                        .any(|id| matches!(ast.kind(id), NodeKind::Table(_)));
                    tags.push((ByteRange::new(span.start, span.end), in_table));
                }
                NodeKind::HtmlBlock(_) => {
                    let block = &text[span.start..span.end];
                    let mut pos = 0;
                    while let Some(found) = block[pos..].find('<') {
                        let start = pos + found;
                        match raw_html(&block[start..]) {
                            Some(len) => {
                                tags.push((
                                    ByteRange::new(span.start + start, span.start + start + len),
                                    false,
                                ));
                                pos = start + len;
                            }
                            None => pos = start + 1,
                        }
                    }
                }
                _ => {}
            }
        }
        let mut diagnostics = Vec::new();
        for (range, in_table) in tags {
            let html = &text[range.start..range.end];
            let Some(name) = TAG_NAME.captures(html).and_then(|captures| captures.get(1)) else {
                continue;
            };
            let name = name.as_str();
            if name.starts_with('/') {
                continue;
            }
            let allowed = if in_table { &table_allowed } else { &allowed };
            if allowed.contains(&name.to_lowercase()) {
                continue;
            }
            // markdownlint highlights the tag's first line.
            let first_line = html.find(['\r', '\n']).unwrap_or(html.len());
            let highlight = ByteRange::new(range.start, range.start + first_line);
            let detail = format!("Element: {name}");
            diagnostics.push(Self::META.diagnostic(
                doc,
                highlight,
                message(&Self::META, Some(&detail), None),
            ));
        }
        diagnostics
    }
}
