//! Diagnostics: one [`Diagnostic`] type and emitters for every output format.
//!
//! A [`Diagnostic`] addresses its file by path and its spans by [`ByteRange`]. Emitters
//! receive the [`SourceFile`]s the diagnostics refer to and look them up by path to turn
//! byte ranges into lines and columns. Lines and columns are **1-based**, and columns
//! count **UTF-16 code units** (the SARIF default `columnKind`, and what editors and
//! GitHub show for text in the Basic Multilingual Plane). When a diagnostic's file is
//! not among the sources, or a range does not fit it, emitters fall back to byte
//! offsets only.
//!
//! | Format | Emitter | Notes |
//! | --- | --- | --- |
//! | [`OutputFormat::Human`] | [`emit_human`] (feature `human`), else [`emit_plain`] | miette graphical reports; color and unicode via [`HumanStyle`] |
//! | [`OutputFormat::Json`] | [`emit_json`] | a pretty-printed JSON array |
//! | [`OutputFormat::Ndjson`] | [`emit_ndjson`] | one JSON object per line |
//! | [`OutputFormat::Sarif`] | [`emit_sarif`] | SARIF 2.1.0 log |
//! | [`OutputFormat::Github`] | [`emit_github`] | GitHub Actions workflow commands |
//!
//! [`OutputFormat::select`] picks `github` automatically under GitHub Actions.
//!
//! miette is the only heavy dependency here, so it sits behind the default `human`
//! feature: library users that only need machine-readable output can disable default
//! features and get [`emit_plain`] for the human format instead.
//!
//! [`ByteRange`]: crate::source::ByteRange

mod diagnostic;
mod format;
mod github;
#[cfg(feature = "human")]
mod human;
mod json;
mod location;
mod plain;
mod sarif;

use std::io::{self, Write};

pub use diagnostic::{Diagnostic, Fix, Label, Severity};
pub use format::{OutputFormat, UnknownFormat};
pub use github::emit_github;
#[cfg(feature = "human")]
pub use human::emit_human;
pub use json::{emit_json, emit_ndjson};
pub use plain::emit_plain;
pub use sarif::{ToolInfo, emit_sarif};

use crate::source::SourceFile;

/// Presentation of the human format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanStyle {
    /// ANSI colors.
    pub color: bool,
    /// Unicode box drawing; ASCII art otherwise.
    pub unicode: bool,
    /// Wrap width in columns.
    pub width: usize,
}

impl Default for HumanStyle {
    fn default() -> Self {
        Self {
            color: false,
            unicode: true,
            width: 100,
        }
    }
}

impl HumanStyle {
    /// Colors only when writing to a terminal and `NO_COLOR` is unset or empty
    /// (<https://no-color.org>). `env` looks up environment variables.
    pub fn detect(is_terminal: bool, env: impl Fn(&str) -> Option<String>) -> Self {
        let no_color = env("NO_COLOR").is_some_and(|value| !value.is_empty());
        Self {
            color: is_terminal && !no_color,
            ..Self::default()
        }
    }
}

/// Settings for [`emit`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmitOptions {
    /// Tool identity reported in SARIF.
    pub tool: ToolInfo,
    /// Presentation of the human format.
    pub human: HumanStyle,
}

/// Writes `diagnostics` to `out` in `format`.
pub fn emit(
    format: OutputFormat,
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    options: &EmitOptions,
    out: &mut impl Write,
) -> io::Result<()> {
    match format {
        #[cfg(feature = "human")]
        OutputFormat::Human => emit_human(diagnostics, sources, &options.human, out),
        #[cfg(not(feature = "human"))]
        OutputFormat::Human => emit_plain(diagnostics, sources, out),
        OutputFormat::Json => emit_json(diagnostics, sources, out),
        OutputFormat::Ndjson => emit_ndjson(diagnostics, sources, out),
        OutputFormat::Sarif => emit_sarif(diagnostics, sources, &options.tool, out),
        OutputFormat::Github => emit_github(diagnostics, sources, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(value: Option<&'static str>) -> impl Fn(&str) -> Option<String> {
        move |key| {
            (key == "NO_COLOR")
                .then_some(value)
                .flatten()
                .map(str::to_owned)
        }
    }

    #[test]
    fn detect_colors_only_on_terminals_without_no_color() {
        assert!(HumanStyle::detect(true, env(None)).color);
        assert!(HumanStyle::detect(true, env(Some(""))).color);
        assert!(!HumanStyle::detect(true, env(Some("1"))).color);
        assert!(!HumanStyle::detect(false, env(None)).color);
    }
}
