//! MD021 `no-multiple-space-closed-atx`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{Atx, edit, ellipsify, heading_line, message};

/// Multiple spaces inside hashes on closed atx style heading.
///
/// A closed ATX heading (open ones are MD019's) with more than one space or tab after
/// its opening or before its closing hashes is reported, once per side; the fix keeps
/// the first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md021;

impl Rule for Md021 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD021",
        aliases: &["no-multiple-space-closed-atx"],
        tags: &["headings", "atx_closed", "spaces"],
        description: "Multiple spaces inside hashes on closed atx style heading",
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
            let Some((_, close_space)) = atx.close else {
                continue;
            };
            for (space, opening) in [(atx.open_space, true), (close_space, false)] {
                if space.len() <= 1 {
                    continue;
                }
                let extra = ByteRange::new(line.start + space.start + 1, line.start + space.end);
                let fix = Fix::new("Keep a single space", vec![edit(extra, "")]);
                let shown = ellipsify(content.trim(), opening, !opening);
                diagnostics.push(
                    Self::META
                        .diagnostic(doc, extra, message(&Self::META, None, Some(&shown)))
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}
