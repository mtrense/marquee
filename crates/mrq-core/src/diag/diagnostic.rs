use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::source::{ByteRange, TextEdit};

/// How serious a [`Diagnostic`] is. Ordered from least (`Hint`) to most (`Error`) severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Hint,
    Info,
    Warning,
    Error,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hint => "hint",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A secondary span in the same file as the diagnostic's primary span, with a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub range: ByteRange,
    pub message: String,
}

impl Label {
    pub fn new(range: ByteRange, message: impl Into<String>) -> Self {
        Self {
            range,
            message: message.into(),
        }
    }
}

/// A machine-applicable fix: edits against the original text of the diagnostic's file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    pub message: String,
    pub edits: Vec<TextEdit>,
}

impl Fix {
    pub fn new(message: impl Into<String>, edits: Vec<TextEdit>) -> Self {
        Self {
            message: message.into(),
            edits,
        }
    }
}

/// One finding (validation error, lint violation, …) about a span of a source file.
///
/// `file` identifies the [`SourceFile`](crate::source::SourceFile) the byte ranges refer to:
/// emitters look the file up by its path to compute lines and columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable identifier such as `MD013` or `MRQ001`.
    pub code: String,
    pub severity: Severity,
    pub file: PathBuf,
    /// The primary span.
    pub range: ByteRange,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Documentation URL for `code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Diagnostic {
    pub fn new(
        code: impl Into<String>,
        severity: Severity,
        file: impl Into<PathBuf>,
        range: ByteRange,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            file: file.into(),
            range,
            message: message.into(),
            labels: Vec::new(),
            fix: None,
            help: None,
            url: None,
        }
    }

    #[must_use]
    pub fn with_label(mut self, label: Label) -> Self {
        self.labels.push(label);
        self
    }

    #[must_use]
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }

    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_by_seriousness() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
        assert!(Severity::Info > Severity::Hint);
    }

    #[test]
    fn severity_serializes_lowercase() {
        for (severity, name) in [
            (Severity::Error, "error"),
            (Severity::Warning, "warning"),
            (Severity::Info, "info"),
            (Severity::Hint, "hint"),
        ] {
            assert_eq!(severity.to_string(), name);
            let json = serde_json::to_string(&severity).unwrap();
            assert_eq!(json, format!("\"{name}\""));
            assert_eq!(serde_json::from_str::<Severity>(&json).unwrap(), severity);
        }
    }

    #[test]
    fn builder_sets_every_field() {
        let edit = TextEdit {
            range: ByteRange::new(0, 1),
            replacement: "b".into(),
        };
        let diag = Diagnostic::new("X1", Severity::Warning, "a.md", ByteRange::new(0, 1), "msg")
            .with_label(Label::new(ByteRange::new(2, 3), "here"))
            .with_fix(Fix::new("replace", vec![edit.clone()]))
            .with_help("try this")
            .with_url("https://example.com");
        assert_eq!(diag.code, "X1");
        assert_eq!(diag.file, PathBuf::from("a.md"));
        assert_eq!(diag.labels, vec![Label::new(ByteRange::new(2, 3), "here")]);
        assert_eq!(diag.fix, Some(Fix::new("replace", vec![edit])));
        assert_eq!(diag.help.as_deref(), Some("try this"));
        assert_eq!(diag.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn diagnostic_serde_round_trip_skips_empty_fields() {
        let diag = Diagnostic::new("X1", Severity::Error, "a.md", ByteRange::new(1, 2), "m");
        let json = serde_json::to_string(&diag).unwrap();
        assert_eq!(
            json,
            r#"{"code":"X1","severity":"error","file":"a.md","range":{"start":1,"end":2},"message":"m"}"#
        );
        assert_eq!(serde_json::from_str::<Diagnostic>(&json).unwrap(), diag);
    }
}
