//! Syntax highlighting of code blocks: syntect with the extra syntaxes and themes of
//! two-face (bat's set), compiled with the pure-Rust `fancy-regex` engine.
//!
//! # Languages
//!
//! [`Highlighter::highlight`] takes the code and the fence info string. The language is
//! the first word of the info string ([`language_token`]), matched case-insensitively
//! against a table of common aliases (`sh`, `shell`, `console`, `py`, `c++`, `golang`,
//! …), then against syntect's file extensions and syntax names. `text`, `plain`, an
//! unknown language or no info string give plain text.
//!
//! # Themes
//!
//! [`Highlighter::new`] maps the theme's [`CodeStyle`]: a bundled theme name (exact, then
//! case-insensitive; [`bundled_themes`]) or a `.tmTheme` file.
//!
//! # Output
//!
//! [`Highlighted`] is a list of lines, each a list of [`Span`]s of text with a [`Style`]
//! (colors, bold, italic, underline) that every backend maps: inline colors for ANSI and
//! PDF, `style` attributes or one CSS class per distinct style for HTML ([`Style::css`]).
//! A span's colors are set only where they differ from the block's
//! [`Highlighted::foreground`] and [`Highlighted::background`], so a backend that paints
//! the block in those colors only has to style the spans that stand out. Plain text has
//! neither block colors nor styled spans; backends use `colors.code_foreground` and
//! `colors.code_background` of the theme for it.

mod language;

use std::path::PathBuf;
use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{self, FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use two_face::theme::LazyThemeSet;

use crate::theme::{CodeStyle, Color};

pub use language::language_token;

/// Two-face's syntaxes, loaded once.
fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// Two-face's themes, each deserialized on first use.
fn themes() -> &'static LazyThemeSet {
    static THEMES: OnceLock<LazyThemeSet> = OnceLock::new();
    THEMES.get_or_init(|| two_face::theme::extra().into())
}

/// The names of the bundled code themes, sorted.
pub fn bundled_themes() -> impl Iterator<Item = &'static str> {
    themes().theme_names()
}

/// The code theme of a [`CodeStyle`] could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum HighlightError {
    /// The name is neither a bundled theme nor a `.tmTheme` path.
    #[error(
        "unknown code theme `{name}`; the bundled themes are {}, or give a `.tmTheme` file path",
        bundled_themes().collect::<Vec<_>>().join(", ")
    )]
    UnknownTheme { name: String },
    /// A `.tmTheme` file could not be read or parsed.
    #[error("cannot load code theme {}: {source}", path.display())]
    ThemeFile {
        path: PathBuf,
        #[source]
        source: syntect::LoadingError,
    },
}

/// The style of a [`Span`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// Text color; `None` keeps the block's foreground.
    pub foreground: Option<Color>,
    /// Background color; `None` keeps the block's background.
    pub background: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Style {
    /// CSS declarations for the style, `;`-separated without a trailing `;`; empty for
    /// the default style.
    pub fn css(&self) -> String {
        let mut decls = Vec::new();
        if let Some(fg) = self.foreground {
            decls.push(format!("color:{fg}"));
        }
        if let Some(bg) = self.background {
            decls.push(format!("background-color:{bg}"));
        }
        if self.bold {
            decls.push("font-weight:bold".to_owned());
        }
        if self.italic {
            decls.push("font-style:italic".to_owned());
        }
        if self.underline {
            decls.push("text-decoration:underline".to_owned());
        }
        decls.join(";")
    }
}

/// A run of text in one style; never contains a line break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// A highlighted code block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Highlighted {
    /// The syntect syntax name (e.g. `Rust`); `None` for plain text.
    pub language: Option<&'static str>,
    /// The block's text color from the code theme; `None` for plain text.
    pub foreground: Option<Color>,
    /// The block's background from the code theme; `None` for plain text.
    pub background: Option<Color>,
    /// The source lines without their line endings (`\n` or `\r\n`). Adjacent spans
    /// have different styles; an empty line has no spans.
    pub lines: Vec<Vec<Span>>,
}

/// Highlights code blocks with one code theme.
#[derive(Debug, Clone)]
pub struct Highlighter {
    theme: highlighting::Theme,
}

impl Highlighter {
    /// A highlighter for the theme `style.theme`: a bundled theme name, matched exactly
    /// and then case-insensitively, else a `.tmTheme` path when it looks like one (a
    /// `.tmTheme` extension or a path separator).
    pub fn new(style: &CodeStyle) -> Result<Self, HighlightError> {
        let name = style.theme.as_str();
        let bundled = themes().get(name).or_else(|| {
            let folded = bundled_themes().find(|t| t.eq_ignore_ascii_case(name))?;
            themes().get(folded)
        });
        if let Some(theme) = bundled {
            return Ok(Self {
                theme: theme.clone(),
            });
        }
        let is_path = name.contains(['/', '\\'])
            || name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("tmtheme"));
        if !is_path {
            return Err(HighlightError::UnknownTheme {
                name: name.to_owned(),
            });
        }
        let path = PathBuf::from(name);
        let theme = ThemeSet::get_theme(&path)
            .map_err(|source| HighlightError::ThemeFile { path, source })?;
        Ok(Self { theme })
    }

    /// The code theme's text color.
    pub fn foreground(&self) -> Option<Color> {
        self.theme.settings.foreground.map(color)
    }

    /// The code theme's background color.
    pub fn background(&self) -> Option<Color> {
        self.theme.settings.background.map(color)
    }

    /// The syntax name of the language of a fence info string; `None` for plain text.
    pub fn language(&self, info: &str) -> Option<&'static str> {
        language::find(syntaxes(), info).map(|syntax| syntax.name.as_str())
    }

    /// Highlights `code` as the language of the fence info string `info`. Falls back to
    /// plain text when the language is unknown or the syntax fails on the input.
    pub fn highlight(&self, code: &str, info: &str) -> Highlighted {
        language::find(syntaxes(), info)
            .and_then(|syntax| self.highlight_syntax(code, syntax))
            .unwrap_or_else(|| plain(code))
    }

    fn highlight_syntax(
        &self,
        code: &str,
        syntax: &'static syntect::parsing::SyntaxReference,
    ) -> Option<Highlighted> {
        let base = (self.foreground(), self.background());
        let mut highlight = HighlightLines::new(syntax, &self.theme);
        let mut lines = Vec::new();
        for line in source_lines(code) {
            // The newline syntaxes expect every line to end in `\n`.
            let line = format!("{line}\n");
            let ranges = highlight.highlight_line(&line, syntaxes()).ok()?;
            let mut spans: Vec<Span> = Vec::new();
            for (style, text) in ranges {
                let text = text.strip_suffix('\n').unwrap_or(text);
                if text.is_empty() {
                    continue;
                }
                let style = span_style(style, base);
                match spans.last_mut() {
                    Some(last) if last.style == style => last.text.push_str(text),
                    _ => spans.push(Span {
                        text: text.to_owned(),
                        style,
                    }),
                }
            }
            lines.push(spans);
        }
        Some(Highlighted {
            language: Some(syntax.name.as_str()),
            foreground: base.0,
            background: base.1,
            lines,
        })
    }
}

/// `code` as plain text.
fn plain(code: &str) -> Highlighted {
    let lines = source_lines(code)
        .map(|line| {
            if line.is_empty() {
                Vec::new()
            } else {
                vec![Span {
                    text: line.to_owned(),
                    style: Style::default(),
                }]
            }
        })
        .collect();
    Highlighted {
        language: None,
        foreground: None,
        background: None,
        lines,
    }
}

/// The lines of `code` without their `\n` or `\r\n`; no final empty line after a
/// trailing line ending.
fn source_lines(code: &str) -> impl Iterator<Item = &str> {
    code.split_inclusive('\n').map(|line| {
        let line = line.strip_suffix('\n').unwrap_or(line);
        line.strip_suffix('\r').unwrap_or(line)
    })
}

/// A syntect style relative to the block's `(foreground, background)`.
fn span_style(style: highlighting::Style, base: (Option<Color>, Option<Color>)) -> Style {
    let differs = |c: highlighting::Color, base: Option<Color>| {
        let c = color(c);
        (Some(c) != base).then_some(c)
    };
    Style {
        foreground: differs(style.foreground, base.0),
        background: differs(style.background, base.1),
        bold: style.font_style.contains(FontStyle::BOLD),
        italic: style.font_style.contains(FontStyle::ITALIC),
        underline: style.font_style.contains(FontStyle::UNDERLINE),
    }
}

fn color(c: highlighting::Color) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}
