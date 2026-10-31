//! Escaping for Typst markup and string literals.

use std::fmt::Write as _;

/// ASCII punctuation that is never markup in the places mrq writes text, so it stays
/// unescaped for readable sources. Everything else (`#`, `$`, `*`, `_`, `` ` ``, `<`,
/// `@`, `[`, `]`, `/`, `\`, `=`, `-`, `+`, `.`, `:`, `~`, quotes, …) gets a backslash,
/// which Typst accepts before any character but whitespace.
const PLAIN: &[char] = &[',', ';', '!', '?', '%', '&', '|', '{', '}', '(', ')', '^'];

/// Appends `text` as Typst markup that shows exactly `text`. Line breaks and tabs
/// become spaces, other control characters are dropped.
pub(super) fn push_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c if c.is_ascii_punctuation() && !PLAIN.contains(&c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
}

/// `text` as Typst markup; see [`push_text`].
pub(super) fn text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_text(&mut out, text);
    out
}

/// `text` as a Typst string literal, quotes included.
pub(super) fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
