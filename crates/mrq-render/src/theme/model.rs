//! The theme data model: one struct per section of a theme file.
//!
//! Every struct rejects unknown keys (`deny_unknown_fields`) and fills missing keys from
//! its [`Default`], which is the `light` theme.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::value::{Color, FontWeight, Length, LengthUnit};

/// A render theme (`--style`, `render.style`): colors, fonts, heading styles and the
/// code-highlighting theme, which every backend maps as well as it can, plus a native
/// override file per backend.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(title = "mrq render theme")]
pub struct Theme {
    /// The theme this one builds on: a built-in theme name (`light`, `dark`, `github`,
    /// `paper`) or a theme file path relative to this file. Without it, the theme builds
    /// on `light`. Always `None` in a loaded theme.
    pub extends: Option<String>,
    /// Colors.
    pub colors: Colors,
    /// Font families and sizes.
    pub fonts: Fonts,
    /// Per-level heading styles.
    pub headings: Headings,
    /// Code blocks.
    pub code: CodeStyle,
    /// Native per-backend files for what the portable theme can't express.
    pub overrides: Overrides,
}

/// `colors` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    /// Body text.
    pub foreground: Color,
    /// Page background; terminal backends leave the background alone.
    pub background: Color,
    /// Heading text, unless the heading style sets its own color.
    pub heading: Color,
    /// Links and other highlights.
    pub accent: Color,
    /// Secondary text: blockquotes, captions, footnotes.
    pub muted: Color,
    /// Table borders, rules and blockquote bars.
    pub border: Color,
    /// Inline code and code blocks without a highlighted language.
    pub code_foreground: Color,
    /// Background of inline code and code blocks.
    pub code_background: Color,
    /// Alerts, callouts and admonitions, by kind.
    pub alerts: AlertColors,
}

/// `colors.alerts` section: the GFM alert kinds. Other callout and admonition kinds map
/// to the closest one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AlertColors {
    pub note: Color,
    pub tip: Color,
    pub important: Color,
    pub warning: Color,
    pub caution: Color,
}

/// `fonts` section. A family is a font name or a comma-separated list of names tried in
/// order; backends add their own generic fallback (`sans-serif`, `monospace`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Fonts {
    /// Body text.
    pub body: String,
    /// Headings; `None` uses `body`.
    pub heading: Option<String>,
    /// Code.
    pub mono: String,
    /// Body font size; `em` and `rem` elsewhere in the theme are relative to it.
    pub size: Length,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
}

/// `headings` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Headings {
    pub h1: HeadingStyle,
    pub h2: HeadingStyle,
    pub h3: HeadingStyle,
    pub h4: HeadingStyle,
    pub h5: HeadingStyle,
    pub h6: HeadingStyle,
}

impl Headings {
    /// The style of heading level `level`; levels above 6 get the `h6` style, and 0 the
    /// `h1` style.
    pub fn level(&self, level: u8) -> &HeadingStyle {
        match level {
            0 | 1 => &self.h1,
            2 => &self.h2,
            3 => &self.h3,
            4 => &self.h4,
            5 => &self.h5,
            _ => &self.h6,
        }
    }
}

/// The style of one heading level. Every property is optional; an unset one keeps the
/// backend's default (body size and weight, `colors.heading`, no italic or underline).
/// A file that extends another theme overrides the properties it sets and inherits the
/// others; `null` unsets an inherited one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HeadingStyle {
    /// Font size, usually in `em` of the body font size.
    pub size: Option<Length>,
    pub weight: Option<FontWeight>,
    pub italic: Option<bool>,
    /// A line under the heading (a bottom border in HTML).
    pub underline: Option<bool>,
    /// Overrides `colors.heading`.
    pub color: Option<Color>,
}

/// `code` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CodeStyle {
    /// The highlighting theme: a bundled syntect theme name (e.g. `InspiredGitHub`) or a
    /// `.tmTheme` file path relative to the theme file.
    pub theme: String,
}

/// `overrides` section: native files per backend, relative to the theme file that sets
/// them. Loaded themes hold absolute paths when the theme file path was absolute.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    /// CSS appended to the generated stylesheet.
    pub html: Option<PathBuf>,
    /// Typst template wrapping the document.
    pub pdf: Option<PathBuf>,
    /// Per-element ANSI styles.
    pub ansi: Option<PathBuf>,
}

/// Parses a built-in hex color literal.
fn hex(text: &str) -> Color {
    text.parse().expect("built-in colors are valid")
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            foreground: hex("#222222"),
            background: hex("#ffffff"),
            heading: hex("#111111"),
            accent: hex("#1a5fb4"),
            muted: hex("#6a6a6a"),
            border: hex("#d4d4d4"),
            code_foreground: hex("#222222"),
            code_background: hex("#f4f4f4"),
            alerts: AlertColors::default(),
        }
    }
}

impl Default for AlertColors {
    fn default() -> Self {
        Self {
            note: hex("#1a5fb4"),
            tip: hex("#26a269"),
            important: hex("#813d9c"),
            warning: hex("#b35900"),
            caution: hex("#c01c28"),
        }
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            body: "system-ui".to_owned(),
            heading: None,
            mono: "ui-monospace".to_owned(),
            size: Length::new(16.0, LengthUnit::Px),
            line_height: 1.5,
        }
    }
}

impl Default for Headings {
    fn default() -> Self {
        let style = |size, underline: bool| HeadingStyle {
            size: Some(Length::new(size, LengthUnit::Em)),
            weight: Some(FontWeight::Bold),
            underline: underline.then_some(true),
            ..HeadingStyle::default()
        };
        Self {
            h1: style(2.0, true),
            h2: style(1.5, true),
            h3: style(1.25, false),
            h4: style(1.0, false),
            h5: style(0.875, false),
            h6: style(0.85, false),
        }
    }
}

impl Default for CodeStyle {
    fn default() -> Self {
        Self {
            theme: "InspiredGitHub".to_owned(),
        }
    }
}
