//! MD035 `hr-style`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{expected_actual, message};

/// Horizontal rule style.
///
/// Every thematic break must be written exactly as `style` (trimmed): its text from
/// the first marker to the end of the line, trailing whitespace included. `consistent`
/// takes the first break's. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md035;

/// Options of MD035 `hr-style`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md035Options {
    /// Horizontal rule style: `consistent` or the exact text, like `---`.
    pub style: String,
}

impl Default for Md035Options {
    fn default() -> Self {
        Self {
            style: "consistent".to_owned(),
        }
    }
}

impl Rule for Md035 {
    type Options = Md035Options;

    const META: RuleMeta = RuleMeta {
        id: "MD035",
        aliases: &["hr-style"],
        tags: &["hr"],
        description: "Horizontal rule style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md035Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut style = match options.style.trim() {
            "" => "consistent",
            style => style,
        };
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::ThematicBreak) {
                continue;
            }
            let span = ast.span(id);
            let actual = &text[span.start..span.end];
            if style == "consistent" {
                style = actual;
            }
            if actual != style {
                diagnostics.push(Self::META.diagnostic(
                    doc,
                    ByteRange::new(span.start, span.end),
                    message(&Self::META, Some(&expected_actual(style, actual)), None),
                ));
            }
        }
        diagnostics
    }
}
