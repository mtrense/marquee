//! Language lookup by fenced code block info string.

use syntect::parsing::{SyntaxReference, SyntaxSet};

/// Fence languages that syntect's own lookup (file extension, then syntax name) misses,
/// mapped to the syntax name.
const ALIASES: &[(&str, &str)] = &[
    ("shell", "Bourne Again Shell (bash)"),
    ("console", "Bourne Again Shell (bash)"),
    ("shellsession", "Bourne Again Shell (bash)"),
    ("python3", "Python"),
    ("jsx", "JavaScript"),
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("csharp", "C#"),
    ("golang", "Go"),
    ("docker", "Dockerfile"),
];

/// Fence languages that mean "no highlighting".
const PLAIN: &[&str] = &["text", "txt", "plain", "plaintext"];

/// The language token of a fence info string: its first word, lowercased, up to the
/// first `,` (rustdoc's `rust,ignore`), with Pandoc's braces and leading `.` removed
/// (`{.python .numberLines}`). `None` for an info string without one.
pub fn language_token(info: &str) -> Option<String> {
    let info = info.trim_start();
    let info = info.strip_prefix('{').unwrap_or(info);
    let word = info.split(|c: char| c.is_whitespace() || c == ',').next()?;
    let word = word.trim_start_matches('.').trim_end_matches('}');
    (!word.is_empty()).then(|| word.to_lowercase())
}

/// The syntax for a fence info string; `None` for plain text and unknown languages.
pub(super) fn find<'a>(syntaxes: &'a SyntaxSet, info: &str) -> Option<&'a SyntaxReference> {
    let token = language_token(info)?;
    if PLAIN.contains(&token.as_str()) {
        return None;
    }
    if let Some(&(_, name)) = ALIASES.iter().find(|(alias, _)| *alias == token) {
        return syntaxes.find_syntax_by_name(name);
    }
    syntaxes.find_syntax_by_token(&token)
}
