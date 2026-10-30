//! MD042 `no-empty-links`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{Definitions, ellipsify, link_syntax, message, one_line};

/// No empty links.
///
/// A link (not an image) whose destination is empty or `#` is reported: an inline link
/// with nothing (or `<>`) between the parentheses or a destination of `#` (after
/// trimming), and a reference link whose definition's destination is `#`. Like
/// markdownlint, the label is looked up as written, so it only matches a definition
/// when it is already lowercase with single spaces. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md042;

impl Rule for Md042 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD042",
        aliases: &["no-empty-links"],
        tags: &["links"],
        description: "No empty links",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let definitions = Definitions::of(doc);
        let is_hash =
            |label: ByteRange| definitions.get(text[label.start..label.end].trim()) == Some("#");
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Link(_)) {
                continue;
            }
            let Some(syntax) = link_syntax(doc, id) else {
                continue;
            };
            let reference = syntax.reference.filter(|reference| !reference.is_empty());
            let shortcut_or_collapsed = !syntax.inline && reference.is_none();
            let empty = if !syntax.text.is_empty() && shortcut_or_collapsed {
                is_hash(syntax.text)
            } else {
                match (reference, syntax.destination) {
                    (Some(reference), None) => is_hash(reference),
                    (None, Some(destination)) => {
                        text[destination.start..destination.end].trim() == "#"
                    }
                    (None, None) => true,
                    (Some(_), Some(_)) => false,
                }
            };
            if empty {
                let span = ast.span(id);
                let range = ByteRange::new(span.start, span.end);
                let shown = ellipsify(&one_line(&text[span.start..span.end]), false, false);
                diagnostics.push(Self::META.diagnostic(
                    doc,
                    range,
                    message(&Self::META, None, Some(&shown)),
                ));
            }
        }
        diagnostics
    }
}
