//! MD020 `no-missing-space-closed-atx`.

use std::sync::LazyLock;

use regex::Regex;

use crate::Document;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, lines, message, opaque_block_lines};

/// No space inside hashes on closed atx style heading.
///
/// Like markdownlint, a raw line shaped like a closed ATX heading (outside code, HTML
/// and math blocks) without whitespace after the opening or before the closing hashes
/// is reported; an escaped `\#` before the closing hashes counts as missing space. The
/// fix rewrites the line as `hashes text hashes` with single spaces.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md020;

/// markdownlint's pattern: opening hashes, space, text, space, an optional escaped
/// hash, closing hashes, trailing whitespace.
static CLOSED_ATX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(#+)([ \t]*)([^# \t\\]|[^# \t][^#]*?[^# \t\\])([ \t]*)((?:\\#)?)(#+)(\s*)$")
        .expect("valid regex")
});

impl Rule for Md020 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD020",
        aliases: &["no-missing-space-closed-atx"],
        tags: &["headings", "atx_closed", "spaces"],
        description: "No space inside hashes on closed atx style heading",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let text = doc.text();
        let opaque = opaque_block_lines(doc);
        let mut diagnostics = Vec::new();
        for (line, range) in lines(doc) {
            if opaque[line] {
                continue;
            }
            let content = &text[range.start..range.end];
            let Some(parts) = CLOSED_ATX.captures(content) else {
                continue;
            };
            let part = |index: usize| parts.get(index).map_or("", |m| m.as_str());
            let (left_hash, content_text, escape, right_hash) =
                (part(1), part(3), part(5), part(6));
            let left = part(2).is_empty();
            let right = part(4).is_empty() || !escape.is_empty();
            if !left && !right {
                continue;
            }
            let highlight = if left {
                ByteRange::new(
                    range.start,
                    text.ceil_char_boundary(range.start + left_hash.len() + 1),
                )
            } else {
                let close = range.start + parts.get(6).map_or(0, |m| m.start());
                ByteRange::new(
                    text.floor_char_boundary(close - 1),
                    close + right_hash.len(),
                )
            };
            let escape = if escape.is_empty() {
                String::new()
            } else {
                format!("{escape} ")
            };
            let fix = Fix::new(
                "Add spaces inside the hashes",
                vec![edit(
                    range,
                    format!("{left_hash} {content_text} {escape}{right_hash}"),
                )],
            );
            let shown = ellipsify(content.trim(), left, right);
            diagnostics.push(
                Self::META
                    .diagnostic(doc, highlight, message(&Self::META, None, Some(&shown)))
                    .with_fix(fix),
            );
        }
        diagnostics
    }
}
