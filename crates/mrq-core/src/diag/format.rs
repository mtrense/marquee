use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Output format for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Graphical, miette-style reports for people.
    #[default]
    Human,
    /// One pretty-printed JSON array.
    Json,
    /// One compact JSON object per line.
    Ndjson,
    /// A SARIF 2.1.0 log.
    Sarif,
    /// GitHub Actions workflow commands (`::error file=…::message`).
    Github,
}

impl OutputFormat {
    pub const ALL: [Self; 5] = [
        Self::Human,
        Self::Json,
        Self::Ndjson,
        Self::Sarif,
        Self::Github,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Json => "json",
            Self::Ndjson => "ndjson",
            Self::Sarif => "sarif",
            Self::Github => "github",
        }
    }

    /// Picks the format: `explicit` if given, otherwise `github` when the environment
    /// variable `GITHUB_ACTIONS` is `true`, otherwise `human`.
    ///
    /// `env` looks up environment variables; pass `|k| std::env::var(k).ok()` or use
    /// [`OutputFormat::select_from_env`].
    pub fn select(explicit: Option<Self>, env: impl Fn(&str) -> Option<String>) -> Self {
        explicit.unwrap_or_else(|| {
            if env("GITHUB_ACTIONS").as_deref() == Some("true") {
                Self::Github
            } else {
                Self::Human
            }
        })
    }

    /// [`OutputFormat::select`] over the process environment.
    pub fn select_from_env(explicit: Option<Self>) -> Self {
        Self::select(explicit, |key| std::env::var(key).ok())
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error for an unknown [`OutputFormat`] name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown output format `{0}` (expected human, json, ndjson, sarif or github)")]
pub struct UnknownFormat(pub String);

impl FromStr for OutputFormat {
    type Err = UnknownFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|format| format.as_str() == s)
            .ok_or_else(|| UnknownFormat(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn defaults_to_human() {
        assert_eq!(OutputFormat::select(None, env(&[])), OutputFormat::Human);
        assert_eq!(OutputFormat::default(), OutputFormat::Human);
    }

    #[test]
    fn github_actions_selects_github() {
        let gha = env(&[("GITHUB_ACTIONS", "true")]);
        assert_eq!(OutputFormat::select(None, gha), OutputFormat::Github);
    }

    #[test]
    fn github_actions_other_values_are_ignored() {
        for value in ["false", "1", "TRUE", ""] {
            let vars = env(&[("GITHUB_ACTIONS", value)]);
            assert_eq!(OutputFormat::select(None, vars), OutputFormat::Human);
        }
    }

    #[test]
    fn explicit_format_wins_over_environment() {
        let gha = env(&[("GITHUB_ACTIONS", "true")]);
        assert_eq!(
            OutputFormat::select(Some(OutputFormat::Sarif), gha),
            OutputFormat::Sarif
        );
        assert_eq!(
            OutputFormat::select(
                Some(OutputFormat::Human),
                env(&[("GITHUB_ACTIONS", "true")])
            ),
            OutputFormat::Human
        );
    }

    #[test]
    fn select_from_env_honours_explicit_format() {
        assert_eq!(
            OutputFormat::select_from_env(Some(OutputFormat::Json)),
            OutputFormat::Json
        );
    }

    #[test]
    fn names_round_trip() {
        for format in OutputFormat::ALL {
            assert_eq!(format.to_string().parse::<OutputFormat>(), Ok(format));
            let json = serde_json::to_string(&format).unwrap();
            assert_eq!(json, format!("\"{format}\""));
        }
        let err = "xml".parse::<OutputFormat>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "unknown output format `xml` (expected human, json, ndjson, sarif or github)"
        );
    }
}
