//! MD041 `first-line-heading`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::validate::reference_definitions;

use super::md001::{FRONT_MATTER_TITLE, front_matter_has_title};
use super::{cleared_lines, ellipsify, html_tag_name, is_html_comment, line_of, message, raw_html};

/// First line in a file should be a top-level heading.
///
/// Unless the front matter has a title (`front_matter_title`), the first top-level block
/// that is not an HTML comment must be a heading of `level`, or an HTML block whose first
/// tag is `<h{level}>`. With `allow_preamble`, other blocks may come first, and only the
/// first heading (Markdown or `<h1>`–`<h6>`) is checked. Link reference definitions
/// count as blocks. The finding is on the offending block's first line. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md041;

/// Options of MD041 `first-line-heading`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md041Options {
    /// Allow content before first heading.
    pub allow_preamble: bool,
    /// Regular expression (case-insensitive) for a frontmatter line that holds the
    /// title; `""` ignores the frontmatter. An invalid expression matches nothing.
    pub front_matter_title: String,
    /// Heading level.
    pub level: u8,
}

impl Default for Md041Options {
    fn default() -> Self {
        Self {
            allow_preamble: false,
            front_matter_title: FRONT_MATTER_TITLE.to_owned(),
            level: 1,
        }
    }
}

impl Rule for Md041 {
    type Options = Md041Options;

    const META: RuleMeta = RuleMeta {
        id: "MD041",
        aliases: &["first-line-heading", "first-line-h1"],
        tags: &["headings"],
        description: "First line in a file should be a top-level heading",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md041Options) -> Vec<Diagnostic> {
        if front_matter_has_title(doc, &options.front_matter_title) {
            return Vec::new();
        }
        let ast = doc.ast();
        let text = doc.text();
        // Top-level blocks by start, definitions (which the tree lacks) included.
        let mut blocks: Vec<(usize, Option<_>)> = ast
            .children(ast.root())
            .iter()
            .map(|&id| (ast.span(id).start, Some(id)))
            .collect();
        blocks.extend(
            reference_definitions(doc)
                .into_iter()
                .map(|definition| (definition.span.start, None)),
        );
        blocks.sort_by_key(|&(start, _)| start);
        let mut offending = None;
        for (start, block) in blocks {
            let Some(id) = block else {
                if !options.allow_preamble {
                    offending = Some(start);
                    break;
                }
                continue;
            };
            let span = ast.span(id);
            match ast.kind(id) {
                NodeKind::FrontMatter(_)
                | NodeKind::IncludeDirective(_)
                | NodeKind::TocDirective(_)
                | NodeKind::DirectiveEnd => {}
                // From the line start: an indented comment does not count.
                NodeKind::Comment(_) | NodeKind::HtmlBlock(_)
                    if is_html_comment(&text[line_start(doc, span.start)..span.end]) => {}
                NodeKind::Heading(heading) => {
                    if heading.level != options.level {
                        offending = Some(start);
                    }
                    break;
                }
                NodeKind::HtmlBlock(_)
                    if first_tag(&text[span.start..span.end])
                        .is_some_and(|name| is_heading_tag(&name)) =>
                {
                    let name = first_tag(&text[span.start..span.end]).unwrap_or_default();
                    if name != format!("h{}", options.level) {
                        offending = Some(start);
                    }
                    break;
                }
                _ if !options.allow_preamble => {
                    offending = Some(start);
                    break;
                }
                _ => {}
            }
        }
        let Some(start) = offending else {
            return Vec::new();
        };
        let Some(range) = doc.source().line_range(line_of(doc, start)) else {
            return Vec::new();
        };
        let line = cleared_lines(doc).swap_remove(line_of(doc, start));
        let shown = ellipsify(&line, false, false);
        vec![Self::META.diagnostic(doc, range, message(&Self::META, None, Some(&shown)))]
    }
}

/// The lowercase name of the first tag in an HTML block (markdownlint parses it again as
/// inline content), or `None` if the first piece of HTML is a comment or the like.
fn first_tag(block: &str) -> Option<String> {
    let start = block.find('<')?;
    let len = raw_html(&block[start..])?;
    html_tag_name(&block[start..start + len]).map(|(name, _)| name.to_lowercase())
}

/// The start of the line `offset` is on.
fn line_start(doc: &Document, offset: usize) -> usize {
    doc.source()
        .line_range(line_of(doc, offset))
        .map_or(offset, |line| line.start)
}

/// `h1` to `h6`.
fn is_heading_tag(name: &str) -> bool {
    name.len() == 2 && name.starts_with('h') && matches!(name.as_bytes()[1], b'1'..=b'6')
}
