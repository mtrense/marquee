//! MD046 `code-block-style`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};

use super::{expected_actual, line_of, message};

/// Code block style.
///
/// Every code block must be fenced or indented as `style` says; `consistent` takes the
/// first block's style. Reported on the block's first line. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md046;

/// Options of MD046 `code-block-style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md046Options {
    /// Block style.
    pub style: CodeBlockStyle,
}

/// A code block style of MD046.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodeBlockStyle {
    /// Like the first code block.
    #[default]
    Consistent,
    /// Between fences.
    Fenced,
    /// Indented by four spaces.
    Indented,
}

impl CodeBlockStyle {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Consistent => "consistent",
            Self::Fenced => "fenced",
            Self::Indented => "indented",
        }
    }
}

impl Rule for Md046 {
    type Options = Md046Options;

    const META: RuleMeta = RuleMeta {
        id: "MD046",
        aliases: &["code-block-style"],
        tags: &["code"],
        description: "Code block style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md046Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let mut expected = options.style;
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::CodeBlock(code) = ast.kind(id) else {
                continue;
            };
            let actual = if code.fence.is_some() {
                CodeBlockStyle::Fenced
            } else {
                CodeBlockStyle::Indented
            };
            if expected == CodeBlockStyle::Consistent {
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
