//! MD023 `heading-start-left`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, line_of, line_prefix, message};

/// Headings must start at the beginning of the line.
///
/// A heading indented within its container (the document, a block quote after its `>`
/// and space, a list item beyond its content indentation) is reported; the fix deletes
/// the indentation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md023;

impl Rule for Md023 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD023",
        aliases: &["heading-start-left"],
        tags: &["headings", "spaces"],
        description: "Headings must start at the beginning of the line",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Heading(_)) {
                continue;
            }
            let start = ast.span(id).start;
            let line = line_of(doc, start);
            let prefix = line_prefix(doc, line, ast.parent(id));
            let indent = text.get(prefix.end..start).unwrap_or_default();
            if indent.is_empty() || !indent.trim_matches([' ', '\t']).is_empty() {
                continue;
            }
            let range = ByteRange::new(prefix.end, start);
            let fix = Fix::new("Delete the indentation", vec![edit(range, "")]);
            let content = doc
                .source()
                .line_range(line)
                .map_or("", |line| &text[line.start..line.end]);
            let shown = ellipsify(content, true, false);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, range, message(&Self::META, None, Some(&shown)))
                    .with_fix(fix),
            );
        }
        diagnostics
    }
}
