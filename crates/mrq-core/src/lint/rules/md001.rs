//! MD001 `heading-increment`.

use regex::RegexBuilder;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};

use super::{expected_actual, heading_line, message};

/// Heading levels should only increment by one level at a time.
///
/// Every heading deeper than one level below the heading before it is reported. A
/// frontmatter `title` counts as a level 1 heading before the first one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md001;

/// Options of MD001 `heading-increment`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md001Options {
    /// Regular expression (case-insensitive) for a frontmatter line that holds the
    /// title; `""` ignores the frontmatter. An invalid expression matches nothing.
    pub front_matter_title: String,
}

impl Default for Md001Options {
    fn default() -> Self {
        Self {
            front_matter_title: FRONT_MATTER_TITLE.to_owned(),
        }
    }
}

/// markdownlint's default `front_matter_title`.
pub(crate) const FRONT_MATTER_TITLE: &str = r#"^\s*"?title"?\s*[:=]"#;

/// Whether a line of the frontmatter (delimiters included) matches `pattern`, as
/// markdownlint's `frontMatterHasTitle`; an empty pattern never matches.
pub(crate) fn front_matter_has_title(doc: &Document, pattern: &str) -> bool {
    let Some(frontmatter) = doc.frontmatter() else {
        return false;
    };
    if pattern.is_empty() {
        return false;
    }
    let Ok(title) = RegexBuilder::new(pattern).case_insensitive(true).build() else {
        return false;
    };
    let span = frontmatter.span();
    doc.text()[span.start..span.end]
        .lines()
        .any(|line| title.is_match(line))
}

impl Rule for Md001 {
    type Options = Md001Options;

    const META: RuleMeta = RuleMeta {
        id: "MD001",
        aliases: &["heading-increment"],
        tags: &["headings"],
        description: "Heading levels should only increment by one level at a time",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md001Options) -> Vec<Diagnostic> {
        let mut previous = if front_matter_has_title(doc, &options.front_matter_title) {
            1
        } else {
            u8::MAX
        };
        let ast = doc.ast();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::Heading(heading) = ast.kind(id) else {
                continue;
            };
            if heading.level > previous.saturating_add(1) {
                let detail =
                    expected_actual(format!("h{}", previous + 1), format!("h{}", heading.level));
                diagnostics.push(Self::META.diagnostic(
                    doc,
                    heading_line(doc, id),
                    message(&Self::META, Some(&detail), None),
                ));
            }
            previous = heading.level;
        }
        diagnostics
    }
}
