//! MRQ002 `heading-punctuation`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{Ast, NodeId, NodeKind, NodeType};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::edit;
use super::md026::GEMOJI_CODE;

/// Heading text ends with punctuation.
///
/// MD026 looks at the raw source of the heading line; this rule looks at the text a
/// reader sees. Punctuation inside emphasis or link text (`# Done *now.*`) counts, and
/// what is not prose does not: a heading ending in inline code, math or an image (alt
/// text is not shown) is fine, trailing comments, raw HTML and footnote references are
/// looked past, and entities count as the character they stand for. GitHub emoji codes
/// (`:smile:`) are left alone, as MD026 does. The default `punctuation` is MD026's, so
/// switching from one to the other only changes how headings are read; the rule is off
/// by default for that reason. The fix deletes the punctuation (and the whitespace
/// before it) when the source spells it out, an escape (`\!`) included.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mrq002;

/// Options of MRQ002 `heading-punctuation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Mrq002Options {
    /// Characters a heading's text must not end with.
    pub punctuation: String,
}

impl Default for Mrq002Options {
    fn default() -> Self {
        Self {
            punctuation: ".,;:!。，；：！".to_owned(),
        }
    }
}

impl Rule for Mrq002 {
    type Options = Mrq002Options;

    const META: RuleMeta = RuleMeta {
        id: "MRQ002",
        aliases: &["heading-punctuation"],
        tags: &["headings", "mrq"],
        description: "Heading text ends with punctuation",
        default_enabled: false,
    };

    fn check(&self, doc: &Document, options: &Mrq002Options) -> Vec<Diagnostic> {
        if options.punctuation.is_empty() {
            return Vec::new();
        }
        let ast = doc.ast();
        let mut diagnostics = Vec::new();
        for heading in ast.descendants(ast.root()) {
            if !matches!(ast.kind(heading), NodeKind::Heading(_))
                || GEMOJI_CODE.is_match(&ast.text(heading))
            {
                continue;
            }
            let Some(last) = last_text(ast, heading) else {
                continue;
            };
            let NodeKind::Text(value) = ast.kind(last) else {
                continue;
            };
            // Whitespace before a trailing comment or footnote reference is no text.
            let value = value.trim_end();
            let kept = value.trim_end_matches(|c| options.punctuation.contains(c));
            if kept.len() == value.len() {
                continue;
            }
            let found = &value[kept.trim_end().len()..];
            let punctuation = found.trim_start();
            let span = ast.span(last);
            let source = doc.text()[span.start..span.end].trim_end();
            let span = ByteRange::new(span.start, span.start + source.len());
            let escaped = format!("\\{punctuation}");
            let removed = if punctuation.chars().count() == 1 && source.ends_with(&escaped) {
                // `\!`: the escape goes too, with the whitespace before it.
                let before = &source[..source.len() - escaped.len()];
                Some(escaped.len() + before.len() - before.trim_end().len())
            } else if source.ends_with(found) {
                Some(found.len())
            } else {
                // An entity or other spelling: reported, but not fixed.
                None
            };
            let range = removed.map_or(span, |len| ByteRange::new(span.end - len, span.end));
            let mut diagnostic = Self::META.diagnostic(
                doc,
                range,
                format!("heading text ends with `{punctuation}`"),
            );
            if removed.is_some() {
                diagnostic =
                    diagnostic.with_fix(Fix::new("Delete the punctuation", vec![edit(range, "")]));
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}

/// The last leaf of `heading` that shows text, unless it is inside an image (whose alt
/// text is not shown): a [`Text`](NodeKind::Text), [`Code`](NodeKind::Code), …
fn last_text(ast: &Ast, heading: NodeId) -> Option<NodeId> {
    let leaf = ast
        .descendants(heading)
        .skip(1)
        .filter(|&id| ast.children(id).is_empty() && !ast.text(id).trim().is_empty())
        .last()?;
    let in_image = ast
        .ancestors(leaf)
        .take_while(|&id| id != heading)
        .any(|id| ast.kind(id).node_type() == NodeType::Image);
    (!in_image).then_some(leaf)
}
