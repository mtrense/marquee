//! Escaping for text, attribute values and URLs, and GFM's tag filter for raw HTML.

use std::fmt::Write as _;

/// Appends `text` with `&`, `<`, `>` and `"` escaped; safe in text and in
/// double-quoted attribute values.
pub(super) fn push_escaped(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// `text` escaped as by [`push_escaped`].
pub(super) fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_escaped(&mut out, text);
    out
}

/// Appends a link or image destination as an `href`/`src` value, like cmark: bytes
/// outside a safe set are percent-encoded, existing `%XX` escapes are kept, `&` becomes
/// `&amp;` and `'` becomes `&#x27;`.
pub(super) fn push_href(out: &mut String, url: &str) {
    const SAFE: &[u8] = b"-_.+!*(),#@?=;:/$~";
    let bytes = url.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'&' => out.push_str("&amp;"),
            b'\'' => out.push_str("&#x27;"),
            b'%' if bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit) =>
            {
                out.push_str(&url[i..i + 3]);
                i += 2;
            }
            _ if b.is_ascii_alphanumeric() || SAFE.contains(&b) => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
        i += 1;
    }
}

/// The tags GFM's tag filter disables in raw HTML.
const FILTERED_TAGS: &[&str] = &[
    "title",
    "textarea",
    "style",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "script",
    "plaintext",
];

/// Appends raw HTML, with the `<` of every start or end tag of a [`FILTERED_TAGS`]
/// element written as `&lt;` when `filter` is set (GFM's "disallowed raw HTML").
pub(super) fn push_raw_html(out: &mut String, html: &str, filter: bool) {
    if !filter {
        out.push_str(html);
        return;
    }
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let name = after.strip_prefix('/').unwrap_or(after);
        let filtered = FILTERED_TAGS.iter().any(|tag| {
            name.get(..tag.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(tag))
                && name[tag.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_ascii_whitespace() || c == '>' || c == '/')
        });
        out.push_str(if filtered { "&lt;" } else { "<" });
        rest = after;
    }
    out.push_str(rest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hrefs_are_percent_encoded_like_cmark() {
        let href = |url: &str| {
            let mut out = String::new();
            push_href(&mut out, url);
            out
        };
        assert_eq!(href("/a b?x=1&y='2'"), "/a%20b?x=1&amp;y=&#x27;2&#x27;");
        assert_eq!(href("%20%zz\u{e4}"), "%20%25zz%C3%A4");
        assert_eq!(href("mailto:a@b.c"), "mailto:a@b.c");
    }

    #[test]
    fn tag_filter_disables_listed_tags_only() {
        let filtered = |html: &str| {
            let mut out = String::new();
            push_raw_html(&mut out, html, true);
            out
        };
        assert_eq!(
            filtered("<SCRIPT x></script><scripts><b>"),
            "&lt;SCRIPT x>&lt;/script><scripts><b>"
        );
        assert_eq!(filtered("<iframe/>"), "&lt;iframe/>");
    }
}
