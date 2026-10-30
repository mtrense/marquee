//! MD048 `code-fence-style`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};

use super::{expected_actual, line_of, message};

/// Code fence style.
///
/// Every fenced code block must use backticks or tildes as `style` says;
/// `consistent` takes the first fence's character. Reported on the opening fence's
/// line. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md048;

/// Options of MD048 `code-fence-style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md048Options {
    /// Code fence style.
    pub style: FenceStyle,
}

/// A code fence style of MD048.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FenceStyle {
    /// Like the first fence.
    #[default]
    Consistent,
    /// `` ``` ``
    Backtick,
    /// `~~~`
    Tilde,
}

impl FenceStyle {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Consistent => "consistent",
            Self::Backtick => "backtick",
            Self::Tilde => "tilde",
        }
    }
}

impl Rule for Md048 {
    type Options = Md048Options;

    const META: RuleMeta = RuleMeta {
        id: "MD048",
        aliases: &["code-fence-style"],
        tags: &["code"],
        description: "Code fence style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md048Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let mut expected = options.style;
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::CodeBlock(code) = ast.kind(id) else {
                continue;
            };
            let Some(fence) = code.fence else {
                continue;
            };
            let actual = if fence.char == '~' {
                FenceStyle::Tilde
            } else {
                FenceStyle::Backtick
            };
            if expected == FenceStyle::Consistent {
                expected = actual;
            }
            if actual != expected {
                let line = line_of(doc, ast.span(id).start);
                let Some(range) = doc.source().line_range(line) else {
                    continue;
                };
                let detail = expected_actual(expected.as_str(), actual.as_str());
                diagnostics.push(Self::META.diagnostic(
                    doc,
                    range,
                    message(&Self::META, Some(&detail), None),
                ));
            }
        }
        diagnostics
    }
}
