//! MD045 `no-alt-text`.

use std::sync::LazyLock;

use regex::Regex;

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{
    first_line_span, html_attribute, html_attribute_pattern, html_pieces, html_tag_name,
    link_syntax, message,
};

/// `alt` attributes.
static ALT: LazyLock<Regex> = LazyLock::new(|| html_attribute_pattern("alt"));
/// `aria-hidden` attributes.
static ARIA_HIDDEN: LazyLock<Regex> = LazyLock::new(|| html_attribute_pattern("aria-hidden"));

/// Images should have alternate text (alt text).
///
/// A Markdown image with empty brackets (`![](…)`, `![][ref]`) is reported, and so is an
/// HTML `<img>` (inline or in an HTML block) without an `alt` attribute, unless it has
/// `aria-hidden="true"`. A multi-line Markdown image is reported on its first line. No
/// fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md045;

impl Rule for Md045 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD045",
        aliases: &["no-alt-text"],
        tags: &["accessibility", "images"],
        description: "Images should have alternate text (alt text)",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut ranges = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Image(_)) {
                continue;
            }
            if link_syntax(doc, id).is_none_or(|syntax| !syntax.text.is_empty()) {
                continue;
            }
            ranges.push(first_line_span(doc, ast.span(id)));
        }
        for (range, _) in html_pieces(doc) {
            let html = &text[range.start..range.end];
            let is_img = html_tag_name(html)
                .is_some_and(|(name, close)| !close && name.eq_ignore_ascii_case("img"));
            if is_img
                && html_attribute(html, &ALT).is_none()
                && html_attribute(html, &ARIA_HIDDEN)
                    .is_none_or(|hidden| !hidden.eq_ignore_ascii_case("true"))
            {
                let first_line = html.find(['\r', '\n']).unwrap_or(html.len());
                ranges.push(ByteRange::new(range.start, range.start + first_line));
            }
        }
        ranges
            .into_iter()
            .map(|range| Self::META.diagnostic(doc, range, message(&Self::META, None, None)))
            .collect()
    }
}
