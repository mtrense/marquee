//! Highlighting as HTML with CSS classes: one class per scope atom (`hl-keyword`,
//! `hl-string`, …), so the markup does not depend on the code theme and the stylesheet
//! ([`Highlighter::css`]) carries the colors.

use syntect::html::{ClassStyle, ClassedHTMLGenerator, css_for_theme_with_class_style};

use super::{Highlighter, language, source_lines, syntaxes};

/// The prefix of every highlighting class; the block itself gets `hl-code`.
pub const CLASS_PREFIX: &str = "hl-";

const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed {
    prefix: CLASS_PREFIX,
};

impl Highlighter {
    /// `code` in the language of the fence info string `info` as HTML: escaped text in
    /// nested `<span class="hl-…">`s, lines ending in `\n`. `None` for plain text (no or
    /// an unknown language) and when the syntax fails on the input.
    pub fn highlight_html(&self, code: &str, info: &str) -> Option<String> {
        let syntax = language::find(syntaxes(), info)?;
        let mut html = ClassedHTMLGenerator::new_with_class_style(syntax, syntaxes(), CLASS_STYLE);
        for line in source_lines(code) {
            html.parse_html_for_line_which_includes_newline(&format!("{line}\n"))
                .ok()?;
        }
        Some(html.finalize())
    }

    /// The stylesheet of the code theme for [`highlight_html`](Self::highlight_html):
    /// `.hl-code` with the theme's text and background colors, then one rule per theme
    /// scope.
    pub fn css(&self) -> String {
        css_for_theme_with_class_style(&self.theme, CLASS_STYLE).unwrap_or_default()
    }
}
