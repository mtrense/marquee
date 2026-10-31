//! The printer hooks of the `mkdocs` flavor: text that the passes would read as an
//! admonition header, heading attributes or math is escaped.
//!
//! - A line that starts with `!!!` or `???` gets its first `!` or `?` escaped, as any
//!   line of a paragraph may be an admonition header.
//! - In the last text of a heading, a `{` after a space whose text ends with `}`
//!   (trailing spaces aside) is escaped, so it is not read as an attribute list.
//! - `\[` and `\(` open arithmatex math, so an escaped `[` or `(` is written as a
//!   character reference (`&#91;`, `&#40;`). `\]` and `\)` close math only after an
//!   opener, and the printer writes none but those of math.
//!
//! Abbreviation definitions (`*[`) need nothing more: `[` is always escaped, and so is
//! `:` at the start of a line (with definition lists on).

use crate::flavor::{EscapeContext, PrinterHooks};

#[derive(Debug)]
pub(super) struct MkdocsPrinter;

impl PrinterHooks for MkdocsPrinter {
    fn escapes(&self, text: &EscapeContext<'_>, at: usize) -> bool {
        let rest = &text.literal[at..];
        let after_space = text.literal[..at]
            .chars()
            .next_back()
            .or(text.before)
            .is_none_or(|c| c == ' ');
        text.ends_heading
            && rest.starts_with('{')
            && rest.trim_end_matches(' ').ends_with('}')
            && after_space
    }

    fn backslash_escapes(&self, c: char) -> bool {
        !matches!(c, '[' | '(')
    }

    fn line_start_marker(&self, line: &str) -> Option<usize> {
        (line.starts_with("!!!") || line.starts_with("???")).then_some(0)
    }
}
