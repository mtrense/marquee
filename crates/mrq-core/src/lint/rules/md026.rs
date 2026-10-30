//! MD026 `no-trailing-punctuation`.

use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, message};

/// Trailing punctuation in heading.
///
/// A heading whose text ends with characters from `punctuation` (and the whitespace
/// before them) is reported, unless it ends with an HTML entity (`&copy;`) or a GitHub
/// emoji code (`:smile:`). The fix deletes the punctuation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md026;

/// Options of MD026 `no-trailing-punctuation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md026Options {
    /// Punctuation characters.
    pub punctuation: String,
}

impl Default for Md026Options {
    fn default() -> Self {
        Self {
            punctuation: ".,;:!。，；：！".to_owned(),
        }
    }
}

/// markdownlint's `endOfLineHtmlEntityRe`.
static HTML_ENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"&(?:#\d+|#[xX][\da-fA-F]+|[a-zA-Z]{2,31}|blk\d{2}|emsp1[34]|frac\d{2}|sup\d|there4);$",
    )
    .expect("valid regex")
});

/// markdownlint's `endOfLineGemojiCodeRe`.
static GEMOJI_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r":(?:[abmovx]|[-+]1|100|1234|(?:1st|2nd|3rd)_place_medal|8ball|clock\d{1,4}|e-mail|non-potable_water|o2|t-rex|u5272|u5408|u55b6|u6307|u6708|u6709|u6e80|u7121|u7533|u7981|u7a7a|[a-z]{2,15}2?|[a-z]{1,14}(?:_[a-z\d]{1,16})+):$",
    )
    .expect("valid regex")
});

impl Rule for Md026 {
    type Options = Md026Options;

    const META: RuleMeta = RuleMeta {
        id: "MD026",
        aliases: &["no-trailing-punctuation"],
        tags: &["headings"],
        description: "Trailing punctuation in heading",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md026Options) -> Vec<Diagnostic> {
        if options.punctuation.is_empty() {
            return Vec::new();
        }
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Heading(_)) {
                continue;
            }
            let children = ast.children(id);
            let (Some(&first), Some(&last)) = (children.first(), children.last()) else {
                continue;
            };
            let content = &text[ast.span(first).start..ast.span(last).end];
            if HTML_ENTITY.is_match(content) || GEMOJI_CODE.is_match(content) {
                continue;
            }
            let punctuation = content.len()
                - content
                    .trim_end_matches(|c| options.punctuation.contains(c))
                    .len();
            if punctuation == 0 {
                continue;
            }
            let before = &content[..content.len() - punctuation];
            let start = before.trim_end().len();
            let found = &content[start..];
            let range = ByteRange::new(ast.span(last).end - found.len(), ast.span(last).end);
            let fix = Fix::new("Delete the punctuation", vec![edit(range, "")]);
            let detail = format!("Punctuation: '{found}'");
            diagnostics.push(
                Self::META
                    .diagnostic(doc, range, message(&Self::META, Some(&detail), None))
                    .with_fix(fix),
            );
        }
        diagnostics
    }
}
