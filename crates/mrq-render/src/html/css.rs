//! The stylesheet generated from a [`Theme`]: page, text, headings, code (with the code
//! theme's highlighting rules), tables, admonitions, footnotes and task lists.

use std::fmt::Write as _;

use crate::highlight::Highlighter;
use crate::theme::{Color, Fonts, HeadingStyle, Theme};

/// The alert kinds of [`AlertColors`](crate::theme::AlertColors), each with the
/// callout and admonition kinds (GFM, Obsidian, mkdocs) that take its color. A kind not
/// listed takes `note`'s.
pub(crate) const ALERT_KINDS: &[(&str, &[&str])] = &[
    (
        "note",
        &[
            "note", "info", "todo", "abstract", "summary", "tldr", "quote", "cite", "seealso",
        ],
    ),
    ("tip", &["tip", "hint", "success", "check", "done"]),
    ("important", &["important", "example"]),
    (
        "warning",
        &["warning", "attention", "question", "help", "faq"],
    ),
    (
        "caution",
        &[
            "caution", "danger", "error", "failure", "fail", "missing", "bug",
        ],
    ),
];

/// The stylesheet for `theme`, with `highlighter`'s rules for highlighted code.
#[allow(clippy::too_many_lines)] // one rule per element
pub(super) fn stylesheet(theme: &Theme, highlighter: &Highlighter) -> String {
    let c = &theme.colors;
    let fonts = &theme.fonts;
    let body = font_family(&fonts.body, generic(&fonts.body));
    let mono = font_family(&fonts.mono, "monospace");
    let mut css = String::new();
    let mut rule = |selector: &str, decls: &[String]| {
        let _ = writeln!(css, "{selector} {{ {}; }}", decls.join("; "));
    };
    rule(
        "html",
        &[
            format!("font-size: {}", fonts.size),
            format!("color: {}", c.foreground),
            format!("background-color: {}", c.background),
        ],
    );
    rule(
        "body",
        &[
            "margin: 0".to_owned(),
            format!("font-family: {body}"),
            format!("line-height: {}", fonts.line_height),
        ],
    );
    rule(
        "main",
        &[
            "max-width: 50rem".to_owned(),
            "margin: 0 auto".to_owned(),
            "padding: 2rem 1rem".to_owned(),
        ],
    );
    rule("a", &[format!("color: {}", c.accent)]);
    for level in 1..=6 {
        rule(
            &format!("h{level}"),
            &heading(
                theme.headings.level(level),
                fonts,
                theme.colors.heading,
                c.border,
            ),
        );
    }
    rule(
        "hr",
        &[
            "border: 0".to_owned(),
            format!("border-top: 1px solid {}", c.border),
        ],
    );
    rule(
        "blockquote",
        &[
            "margin-left: 0".to_owned(),
            "padding: 0 1em".to_owned(),
            format!("color: {}", c.muted),
            format!("border-left: 0.25em solid {}", c.border),
        ],
    );
    rule(
        "code, kbd, samp, pre",
        &[
            format!("font-family: {mono}"),
            "font-size: 0.9em".to_owned(),
        ],
    );
    rule(
        "code",
        &[
            format!("color: {}", c.code_foreground),
            format!("background-color: {}", c.code_background),
            "padding: 0.1em 0.3em".to_owned(),
            "border-radius: 0.25em".to_owned(),
        ],
    );
    rule(
        "pre",
        &[
            format!("color: {}", c.code_foreground),
            format!("background-color: {}", c.code_background),
            "padding: 0.75em 1em".to_owned(),
            "overflow-x: auto".to_owned(),
            "border-radius: 0.25em".to_owned(),
            "line-height: 1.4".to_owned(),
        ],
    );
    rule(
        "pre code",
        &[
            "color: inherit".to_owned(),
            "background: none".to_owned(),
            "padding: 0".to_owned(),
            "font-size: inherit".to_owned(),
        ],
    );
    rule("table", &["border-collapse: collapse".to_owned()]);
    rule(
        "th, td",
        &[
            format!("border: 1px solid {}", c.border),
            "padding: 0.3em 0.75em".to_owned(),
        ],
    );
    rule("img", &["max-width: 100%".to_owned()]);
    rule(
        "mark",
        &[format!(
            "background-color: {}",
            translucent(c.alerts.warning)
        )],
    );
    rule("dt", &["font-weight: bold".to_owned()]);
    rule(
        "li:has(> input[type=\"checkbox\"]:first-child), li:has(> p:first-child > input[type=\"checkbox\"]:first-child)",
        &["list-style: none".to_owned()],
    );
    rule(
        "li > input[type=\"checkbox\"]:first-child, li > p:first-child > input[type=\"checkbox\"]:first-child",
        &["margin: 0 0.4em 0 -1.3em".to_owned()],
    );
    rule(".tag", &[format!("color: {}", c.accent)]);
    rule(
        ".admonition",
        &[
            format!("--admonition-color: {}", c.alerts.note),
            "margin: 1em 0".to_owned(),
            "padding: 0 1em".to_owned(),
            "border-left: 0.25em solid var(--admonition-color)".to_owned(),
        ],
    );
    rule(
        ".admonition-title",
        &[
            "font-weight: bold".to_owned(),
            "color: var(--admonition-color)".to_owned(),
        ],
    );
    rule(
        "summary.admonition-title",
        &["margin: 1em 0".to_owned(), "cursor: pointer".to_owned()],
    );
    for (alert, kinds) in ALERT_KINDS {
        let color = match *alert {
            "tip" => c.alerts.tip,
            "important" => c.alerts.important,
            "warning" => c.alerts.warning,
            "caution" => c.alerts.caution,
            _ => c.alerts.note,
        };
        let selector: Vec<String> = kinds.iter().map(|k| format!(".admonition.{k}")).collect();
        rule(
            &selector.join(", "),
            &[format!("--admonition-color: {color}")],
        );
    }
    rule(
        ".footnotes",
        &[
            format!("color: {}", c.muted),
            "font-size: 0.9em".to_owned(),
            format!("border-top: 1px solid {}", c.border),
        ],
    );
    rule("math[display=\"block\"]", &["margin: 1em 0".to_owned()]);
    css.push_str(&highlighter.css());
    css
}

/// The CSS declarations of a heading level. Unset properties take the body size and
/// weight, upright text, no rule and `colors.heading`.
fn heading(style: &HeadingStyle, fonts: &Fonts, color: Color, border: Color) -> Vec<String> {
    let mut decls = vec![
        format!(
            "font-size: {}",
            style
                .size
                .map_or_else(|| "1em".to_owned(), |s| s.to_string())
        ),
        format!(
            "font-weight: {}",
            style.weight.map_or(400, crate::theme::FontWeight::number)
        ),
        format!(
            "font-style: {}",
            if style.italic == Some(true) {
                "italic"
            } else {
                "normal"
            }
        ),
        format!("color: {}", style.color.unwrap_or(color)),
    ];
    if let Some(family) = &fonts.heading {
        decls.push(format!(
            "font-family: {}",
            font_family(family, generic(family))
        ));
    }
    if style.underline == Some(true) {
        decls.push(format!("border-bottom: 1px solid {border}"));
        decls.push("padding-bottom: 0.3em".to_owned());
    }
    decls
}

/// The generic CSS family that ends a font list: `serif` when a name in it says serif
/// (and not sans), else `sans-serif`.
fn generic(family: &str) -> &'static str {
    let lower = family.to_ascii_lowercase();
    if lower.contains("serif") && !lower.contains("sans") {
        "serif"
    } else {
        "sans-serif"
    }
}

/// CSS generic families and system keywords, never quoted.
const KEYWORDS: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "math",
    "emoji",
];

/// A theme font list (`Inter, Helvetica`) as a CSS `font-family` value: names quoted
/// unless they are keywords or already quoted, then `fallback` unless listed.
fn font_family(family: &str, fallback: &str) -> String {
    let mut names: Vec<String> = family
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            if KEYWORDS.contains(&name) || name.starts_with(['"', '\'']) {
                name.to_owned()
            } else {
                format!("\"{}\"", name.replace(['"', '\\'], ""))
            }
        })
        .collect();
    if !names.iter().any(|name| name == fallback) {
        names.push(fallback.to_owned());
    }
    names.join(", ")
}

/// `color` at a quarter of its opacity.
fn translucent(color: Color) -> Color {
    Color::rgba(color.r, color.g, color.b, color.a / 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_lists_get_quotes_and_a_generic_fallback() {
        assert_eq!(
            font_family("Inter, system-ui", "sans-serif"),
            "\"Inter\", system-ui, sans-serif"
        );
        assert_eq!(font_family("'A B'", "serif"), "'A B', serif");
        assert_eq!(font_family("monospace", "monospace"), "monospace");
        assert_eq!(generic("Libertinus Serif"), "serif");
        assert_eq!(generic("DejaVu Sans"), "sans-serif");
    }
}
