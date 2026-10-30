//! Configuration errors with provenance.

use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::diag::{Diagnostic, Severity};
use crate::source::ByteRange;

/// Diagnostic code of configuration errors.
pub const CONFIG_ERROR_CODE: &str = "config";

/// Where in a config file an error was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The config file.
    pub path: PathBuf,
    /// 1-based line.
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
    /// Bytes of the offending key, item or token.
    pub range: ByteRange,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.path.display(), self.line, self.column)
    }
}

/// Loading the configuration failed.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A config file could not be read.
    #[error("cannot read config file {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A config file is not valid YAML, or not a mapping at the top level.
    #[error("{location}: invalid YAML: {message}")]
    Syntax { location: Location, message: String },
    /// A key is unknown or a value has the wrong type or an unsupported value.
    #[error("{}{message}", location.as_ref().map(|l| format!("{l}: ")).unwrap_or_default())]
    Invalid {
        /// The file and line that set the offending key, when known.
        location: Option<Location>,
        /// Dotted path of the offending key, e.g. `fmt.wrap`.
        key: String,
        message: String,
    },
}

impl ConfigError {
    /// The file and line the error points at, if any.
    pub fn location(&self) -> Option<&Location> {
        match self {
            Self::Io { .. } => None,
            Self::Syntax { location, .. } => Some(location),
            Self::Invalid { location, .. } => location.as_ref(),
        }
    }

    /// The error as a [`Diagnostic`] on the config file, when it has a location.
    pub fn to_diagnostic(&self) -> Option<Diagnostic> {
        let (location, message) = match self {
            Self::Io { .. } => return None,
            Self::Syntax { location, message } => (location, format!("invalid YAML: {message}")),
            Self::Invalid {
                location, message, ..
            } => (location.as_ref()?, message.clone()),
        };
        Some(Diagnostic::new(
            CONFIG_ERROR_CODE,
            Severity::Error,
            location.path.clone(),
            location.range,
            message,
        ))
    }
}
