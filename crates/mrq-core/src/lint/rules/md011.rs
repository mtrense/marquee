//! MD011 `no-reversed-links`.

use std::sync::LazyLock;

use regex::Regex;

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, lines, message, opaque_block_lines};

/// Reversed link syntax.
///
/// `(text)[url]` is reported outside code, HTML and math, unless the `(` is escaped,
/// a `(` follows, the url starts with `^` (a footnote) or either part ends with a
/// backslash. The fix swaps the brackets: `[text](url)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md011;

/// markdownlint's pattern; group 1 is the character before the `(`.
static REVERSED_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|[^\\])\(([^()]+)\)\[([^\]^][^\]]*)\]").expect("valid regex"));

impl Rule for Md011 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD011",
        aliases: &["no-reversed-links"],
        tags: &["links"],
        description: "Reversed link syntax",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let opaque = opaque_block_lines(doc);
        let inline_code: Vec<ByteRange> = ast
            .descendants(ast.root())
            .filter(|&id| matches!(ast.kind(id), NodeKind::Code(_) | NodeKind::Math(_)))
            .map(|id| ast.span(id))
            .collect();
        let mut diagnostics = Vec::new();
        for (line, range) in lines(doc) {
            if opaque[line] {
                continue;
            }
            let content = &text[range.start..range.end];
            for captures in REVERSED_LINK.captures_iter(content) {
                let (Some(whole), Some(pre), Some(label), Some(url)) = (
                    captures.get(0),
                    captures.get(1),
                    captures.get(2),
                    captures.get(3),
                ) else {
                    continue;
                };
                if content[whole.end()..].starts_with('(')
                    || label.as_str().ends_with('\\')
                    || url.as_str().ends_with('\\')
                {
                    continue;
                }
                let link = ByteRange::new(range.start + pre.end(), range.start + whole.end());
                // Inclusive comparison, as markdownlint's `hasOverlap`.
                if inline_code
                    .iter()
                    .any(|code| code.start < link.end && link.start <= code.end)
                {
                    continue;
                }
                let reversed = &content[pre.end()..whole.end()];
                let fix = Fix::new(
                    "Swap the brackets",
                    vec![edit(
                        link,
                        format!("[{}]({})", label.as_str(), url.as_str()),
                    )],
                );
                diagnostics.push(
                    Self::META
                        .diagnostic(doc, link, message(&Self::META, Some(reversed), None))
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}
