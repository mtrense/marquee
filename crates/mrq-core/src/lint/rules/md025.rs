//! MD025 `single-title`.

use regex::RegexBuilder;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{Ast, LinkForm, NodeId, NodeKind};
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};

use super::{ellipsify, heading_line, heading_text, message};

/// Multiple top-level headings in the same document.
///
/// If the document has a title (front matter matching `front_matter_title`, or a
/// first heading of `level` with only comments before it), every other heading of
/// `level` is reported. Headings that are docfx tabs (`# [Tab](#tab/id)`) do not
/// count. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md025;

/// Options of MD025 `single-title`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md025Options {
    /// Heading level.
    pub level: u8,
    /// `RegExp` for matching title in front matter; empty to ignore front matter.
    pub front_matter_title: String,
}

impl Default for Md025Options {
    fn default() -> Self {
        Self {
            level: 1,
            front_matter_title: DEFAULT_TITLE.to_owned(),
        }
    }
}

/// markdownlint's default `front_matter_title`.
const DEFAULT_TITLE: &str = r#"^\s*"?title"?\s*[:=]"#;

impl Rule for Md025 {
    type Options = Md025Options;

    const META: RuleMeta = RuleMeta {
        id: "MD025",
        aliases: &["single-title", "single-h1"],
        tags: &["headings"],
        description: "Multiple top-level headings in the same document",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md025Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let headings: Vec<NodeId> = ast
            .descendants(ast.root())
            .filter(|&id| {
                matches!(ast.kind(id), NodeKind::Heading(heading) if heading.level == options.level)
                    && !is_docfx_tab(ast, id)
            })
            .collect();
        let Some(&first) = headings.first() else {
            return Vec::new();
        };
        let front_matter_title = front_matter_has_title(doc, &options.front_matter_title);
        let title = front_matter_title
            || (ast.parent(first) == Some(ast.root())
                && ast
                    .children(ast.root())
                    .iter()
                    .take_while(|&&id| id != first)
                    .all(|&id| {
                        matches!(
                            ast.kind(id),
                            NodeKind::FrontMatter(_)
                                | NodeKind::Comment(_)
                                | NodeKind::IncludeDirective(_)
                                | NodeKind::TocDirective(_)
                                | NodeKind::DirectiveEnd
                        )
                    }));
        if !title {
            return Vec::new();
        }
        headings
            .into_iter()
            .skip(usize::from(!front_matter_title))
            .map(|id| {
                let shown = ellipsify(&heading_text(doc, id), false, false);
                Self::META.diagnostic(
                    doc,
                    heading_line(doc, id),
                    message(&Self::META, None, Some(&shown)),
                )
            })
            .collect()
    }
}

/// Whether a front matter line matches `pattern` (case-insensitively); an empty or
/// invalid pattern never matches.
fn front_matter_has_title(doc: &Document, pattern: &str) -> bool {
    let Some(frontmatter) = doc.frontmatter() else {
        return false;
    };
    let Ok(title) = RegexBuilder::new(pattern).case_insensitive(true).build() else {
        return false;
    };
    let span = frontmatter.span();
    !pattern.is_empty()
        && doc.text()[span.start..span.end]
            .lines()
            .any(|line| title.is_match(line))
}

/// Whether `heading` is a docfx tab: an ATX heading that is only a link to `#tab/…`.
fn is_docfx_tab(ast: &Ast, heading: NodeId) -> bool {
    let NodeKind::Heading(payload) = ast.kind(heading) else {
        return false;
    };
    match ast.children(heading) {
        [link] if !payload.setext => matches!(
            ast.kind(*link),
            NodeKind::Link(link) if link.form == LinkForm::Inline && link.url.starts_with("#tab/")
        ),
        _ => false,
    }
}
