//! MD049 `emphasis-style` and MD050 `strong-style`, which share their logic.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, expected_actual, message};

/// Emphasis style.
///
/// Every emphasis must use `*` or `_` as `style` says; `consistent` takes the first
/// emphasis's character. Its opening and closing sequences are reported separately,
/// except when `_` is wanted and the emphasis is inside a word (where `_` would not
/// work). The fix replaces the sequence.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md049;

/// Strong style.
///
/// As [`Md049`], for strong emphasis (`**` or `__`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Md050;

/// Options of MD049 `emphasis-style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md049Options {
    /// Emphasis style.
    pub style: EmphasisStyle,
}

/// Options of MD050 `strong-style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md050Options {
    /// Strong style.
    pub style: EmphasisStyle,
}

/// An emphasis or strong style of MD049 and MD050.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmphasisStyle {
    /// Like the first emphasis.
    #[default]
    Consistent,
    /// `*` or `**`
    Asterisk,
    /// `_` or `__`
    Underscore,
}

impl EmphasisStyle {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Consistent => "consistent",
            Self::Asterisk => "asterisk",
            Self::Underscore => "underscore",
        }
    }
}

impl Rule for Md049 {
    type Options = Md049Options;

    const META: RuleMeta = RuleMeta {
        id: "MD049",
        aliases: &["emphasis-style"],
        tags: &["emphasis"],
        description: "Emphasis style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md049Options) -> Vec<Diagnostic> {
        check(doc, &Self::META, options.style, false)
    }
}

impl Rule for Md050 {
    type Options = Md050Options;

    const META: RuleMeta = RuleMeta {
        id: "MD050",
        aliases: &["strong-style"],
        tags: &["emphasis"],
        description: "Strong style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md050Options) -> Vec<Diagnostic> {
        check(doc, &Self::META, options.style, true)
    }
}

/// Checks every emphasis (or every strong emphasis, if `strong`) against `style`.
fn check(
    doc: &Document,
    meta: &RuleMeta,
    mut style: EmphasisStyle,
    strong: bool,
) -> Vec<Diagnostic> {
    let ast = doc.ast();
    let text = doc.text();
    let len = if strong { 2 } else { 1 };
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let mut diagnostics = Vec::new();
    for id in ast.descendants(ast.root()) {
        let wanted = if strong {
            matches!(ast.kind(id), NodeKind::Strong)
        } else {
            matches!(ast.kind(id), NodeKind::Emphasis)
        };
        let span = ast.span(id);
        if !wanted || span.end - span.start < 2 * len {
            continue;
        }
        let actual = if text[span.start..].starts_with('*') {
            EmphasisStyle::Asterisk
        } else {
            EmphasisStyle::Underscore
        };
        if style == EmphasisStyle::Consistent {
            style = actual;
        }
        if style == actual {
            continue;
        }
        let intraword = style == EmphasisStyle::Underscore
            && (is_word(text[..span.start].chars().next_back())
                || is_word(text[span.end..].chars().next()));
        if intraword {
            continue;
        }
        let marker = if style == EmphasisStyle::Asterisk {
            "*"
        } else {
            "_"
        };
        let detail = expected_actual(style.as_str(), actual.as_str());
        for range in [
            ByteRange::new(span.start, span.start + len),
            ByteRange::new(span.end - len, span.end),
        ] {
            diagnostics.push(
                meta.diagnostic(doc, range, message(meta, Some(&detail), None))
                    .with_fix(Fix::new(
                        format!("Use {}", style.as_str()),
                        vec![edit(range, marker.repeat(len))],
                    )),
            );
        }
    }
    diagnostics
}
