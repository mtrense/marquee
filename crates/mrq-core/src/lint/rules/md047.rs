//! MD047 `single-trailing-newline`.

use crate::Document;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::{ByteRange, TextEdit};

use super::line_ending;

/// Files should end with a single newline character.
///
/// The last line must be blank: a file whose last line has content gets a line ending
/// (the document's own, `\n` or `\r\n`) appended. More than one trailing blank line is
/// MD012's business.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md047;

impl Rule for Md047 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD047",
        aliases: &["single-trailing-newline"],
        tags: &["blank_lines"],
        description: "Files should end with a single newline character",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let text = doc.text();
        let last_line = &text[text.rfind('\n').map_or(0, |newline| newline + 1)..];
        if last_line.trim().is_empty() {
            return Vec::new();
        }
        let end = ByteRange::empty(text.len());
        let fix = Fix::new(
            "Add a newline at the end of the file",
            vec![TextEdit {
                range: end,
                replacement: line_ending(doc).to_owned(),
            }],
        );
        vec![
            Self::META
                .diagnostic(doc, end, Self::META.description)
                .with_fix(fix),
        ]
    }
}
