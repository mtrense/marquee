//! The printer hooks of the `obsidian` flavor: text that the `inlines` pass would read as
//! a tag or a `%%` comment is escaped.
//!
//! - `#` where a tag may start (at the start of its parent's text or after whitespace)
//!   when the name after it has a non-digit, or runs into whatever follows the text.
//! - `%` before another `%` (or at the end of a text node, when what follows is not
//!   known): with every `%%` escaped, no comment delimiter is left.
//!
//! Wikilinks, embeds and callouts need nothing: `[` and `]` are always escaped.

use crate::flavor::{EscapeContext, PrinterHooks};

#[derive(Debug)]
pub(super) struct ObsidianPrinter;

impl PrinterHooks for ObsidianPrinter {
    fn escapes(&self, text: &EscapeContext<'_>, at: usize) -> bool {
        let literal = text.literal;
        let rest = &literal[at..];
        if let Some(name) = rest.strip_prefix('#') {
            let before = literal[..at].chars().next_back().or(text.before);
            if !before.is_none_or(char::is_whitespace) {
                return false;
            }
            let len = name
                .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/')))
                .unwrap_or(name.len());
            let open_ended = len == name.len() && text.after.is_none();
            name[..len].contains(|c: char| !c.is_ascii_digit()) || open_ended
        } else if let Some(after) = rest.strip_prefix('%') {
            match after.chars().next() {
                Some(next) => next == '%',
                None => text.after.is_none_or(|after| after == '%'),
            }
        } else {
            false
        }
    }
}
