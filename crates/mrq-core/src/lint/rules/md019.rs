//! MD019 `no-multiple-space-atx`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{Atx, edit, ellipsify, heading_line, message};

/// Multiple spaces after hash on atx style heading.
///
/// An open ATX heading (closed ones are MD021's) with more than one space or tab
/// after its hashes is reported; the fix keeps the first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md019;

impl Rule for Md019 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD019",
        aliases: &["no-multiple-space-atx"],
        tags: &["headings", "atx", "spaces"],
        description: "Multiple spaces after hash on atx style heading",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Heading(heading) if !heading.setext) {
                continue;
            }
            let line = heading_line(doc, id);
            let content = &text[line.start..line.end];
            let atx = Atx::parse(content);
            let space = atx.open_space;
            if atx.close.is_some() || space.len() <= 1 {
                continue;
            }
            let extra = ByteRange::new(line.start + space.start + 1, line.start + space.end);
            let fix = Fix::new("Keep a single space", vec![edit(extra, "")]);
            let shown = ellipsify(content.trim(), true, false);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, extra, message(&Self::META, None, Some(&shown)))
                    .with_fix(fix),
            );
        }
        diagnostics
    }
}
