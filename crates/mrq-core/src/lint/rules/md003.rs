//! MD003 `heading-style`.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};

use super::{Atx, expected_actual, heading_line, message};

/// Heading style.
///
/// Every heading must have the configured style; `consistent` takes the first
/// heading's. The `setext_with_*` styles want setext for levels 1 and 2 and the given
/// ATX style below.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md003;

/// Options of MD003 `heading-style`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md003Options {
    /// Heading style.
    pub style: HeadingStyle,
}

/// A heading style of MD003.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeadingStyle {
    /// Like the first heading.
    #[default]
    Consistent,
    /// `# Heading`
    Atx,
    /// `# Heading #`
    AtxClosed,
    /// Underlined with `===` or `---`.
    Setext,
    /// Setext for levels 1 and 2, `atx` below.
    SetextWithAtx,
    /// Setext for levels 1 and 2, `atx_closed` below.
    SetextWithAtxClosed,
}

impl fmt::Display for HeadingStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Consistent => "consistent",
            Self::Atx => "atx",
            Self::AtxClosed => "atx_closed",
            Self::Setext => "setext",
            Self::SetextWithAtx => "setext_with_atx",
            Self::SetextWithAtxClosed => "setext_with_atx_closed",
        })
    }
}

impl Rule for Md003 {
    type Options = Md003Options;

    const META: RuleMeta = RuleMeta {
        id: "MD003",
        aliases: &["heading-style"],
        tags: &["headings"],
        description: "Heading style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md003Options) -> Vec<Diagnostic> {
        let mut style = options.style;
        let ast = doc.ast();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::Heading(heading) = ast.kind(id) else {
                continue;
            };
            let line = heading_line(doc, id);
            let actual = if heading.setext {
                HeadingStyle::Setext
            } else if Atx::parse(&doc.text()[line.start..line.end])
                .close
                .is_some()
            {
                HeadingStyle::AtxClosed
            } else {
                HeadingStyle::Atx
            };
            if style == HeadingStyle::Consistent {
                style = actual;
            }
            let h12 = heading.level <= 2;
            let expected = match style {
                HeadingStyle::SetextWithAtx | HeadingStyle::SetextWithAtxClosed if h12 => {
                    HeadingStyle::Setext
                }
                HeadingStyle::SetextWithAtx => HeadingStyle::Atx,
                HeadingStyle::SetextWithAtxClosed => HeadingStyle::AtxClosed,
                other => other,
            };
            if actual != expected {
                let detail = expected_actual(expected, actual);
                diagnostics.push(Self::META.diagnostic(
                    doc,
                    line,
                    message(&Self::META, Some(&detail), None),
                ));
            }
        }
        diagnostics
    }
}
