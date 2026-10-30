//! MD018 `no-missing-space-atx`.

use crate::Document;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, lines, message, opaque_block_lines};

/// No space after hash on atx style heading.
///
/// Like markdownlint, a raw line that starts with `#` followed by something other than
/// `#`, space or tab, and does not end with `#`, is reported outside code, HTML and math
/// blocks (so `#hashtag` at the start of a line counts, and a heading in a list or quote
/// does not). The fix inserts a space after the hashes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md018;

impl Rule for Md018 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD018",
        aliases: &["no-missing-space-atx"],
        tags: &["headings", "atx", "spaces"],
        description: "No space after hash on atx style heading",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let text = doc.text();
        let opaque = opaque_block_lines(doc);
        let mut diagnostics = Vec::new();
        for (line, range) in lines(doc) {
            let content = &text[range.start..range.end];
            let hashes = content.len() - content.trim_start_matches('#').len();
            let next = content[hashes..].chars().next();
            let Some(next) = next else {
                continue;
            };
            if opaque[line]
                || hashes == 0
                || matches!(next, ' ' | '\t')
                || content.trim_end().ends_with('#')
                || content.starts_with("#\u{fe0f}\u{20e3}")
            {
                continue;
            }
            let at = range.start + hashes;
            let fix = Fix::new(
                "Add a space after the hashes",
                vec![edit(ByteRange::empty(at), " ")],
            );
            let shown = ellipsify(content.trim(), false, false);
            diagnostics.push(
                Self::META
                    .diagnostic(
                        doc,
                        ByteRange::new(range.start, at + next.len_utf8()),
                        message(&Self::META, None, Some(&shown)),
                    )
                    .with_fix(fix),
            );
        }
        diagnostics
    }
}
