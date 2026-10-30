//! MD059 `descriptive-link-text`.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{NodeKind, Span};
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};

use super::{ellipsify, first_line_span, link_syntax, message, one_line};

/// Link text should be descriptive.
///
/// A link (not an image or autolink) whose text, normalized (ASCII punctuation, `_` and
/// non-ASCII characters as spaces, whitespace collapsed, lowercase), is one of
/// `prohibited_texts` is reported, unless the text holds a code span or HTML. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md059;

/// Options of MD059 `descriptive-link-text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md059Options {
    /// Prohibited link texts.
    pub prohibited_texts: Vec<String>,
}

impl Default for Md059Options {
    fn default() -> Self {
        Self {
            prohibited_texts: ["click here", "here", "link", "more"]
                .map(str::to_owned)
                .to_vec(),
        }
    }
}

impl Rule for Md059 {
    type Options = Md059Options;

    const META: RuleMeta = RuleMeta {
        id: "MD059",
        aliases: &["descriptive-link-text"],
        tags: &["accessibility", "links"],
        description: "Link text should be descriptive",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md059Options) -> Vec<Diagnostic> {
        let prohibited: HashSet<String> = options
            .prohibited_texts
            .iter()
            .map(|prohibited| normalize(prohibited))
            .collect();
        if prohibited.is_empty() {
            return Vec::new();
        }
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Link(_)) {
                continue;
            }
            let Some(syntax) = link_syntax(doc, id) else {
                continue;
            };
            let has_code_or_html = ast.children(id).iter().any(|&child| {
                matches!(
                    ast.kind(child),
                    NodeKind::Code(_) | NodeKind::HtmlInline(_) | NodeKind::Comment(_)
                )
            });
            let label = &text[syntax.text.start..syntax.text.end];
            if has_code_or_html || !prohibited.contains(&normalize(label)) {
                continue;
            }
            let range = first_line_span(doc, Span::new(syntax.text.start, syntax.text.end));
            let brackets = &text[syntax.text.start - 1..=syntax.text.end];
            let shown = ellipsify(&one_line(brackets), false, false);
            diagnostics.push(Self::META.diagnostic(
                doc,
                range,
                message(&Self::META, None, Some(&shown)),
            ));
        }
        diagnostics
    }
}

/// markdownlint's `normalize`: runs of ASCII non-word characters as one space, then
/// whitespace collapsed, lowercase and trimmed.
fn normalize(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
