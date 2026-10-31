//! Formatting the code of fenced blocks: the [`CodeFormatter`] seam and
//! [`CommandFormatter`], which pipes the code through the external commands of
//! `fmt.code_formatters`.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::config::{FmtConfig, LoadedConfig};
use crate::process;

/// Formats the code of fenced blocks for [`format_with`](super::format_with).
///
/// [`CommandFormatter`] runs the configured commands; tests and other callers can
/// substitute their own, closures included:
///
/// ```
/// use mrq_core::Document;
/// use mrq_core::config::FmtConfig;
/// use mrq_core::fmt::format_with;
///
/// let upper = |lang: &str, code: &str| (lang == "sh").then(|| Ok(code.to_uppercase()));
/// let doc = Document::parse("```sh\necho hi\n```\n");
/// let out = format_with(&doc, &FmtConfig::default(), &upper);
/// assert_eq!(out.text, "```sh\nECHO HI\n```\n");
/// ```
pub trait CodeFormatter {
    /// The formatted `code` of a block whose language (the first word of its info string)
    /// is `lang`: `None` when there is no formatter for `lang`, an error message when the
    /// formatter failed. `code` and the result end with a newline (one is added to a
    /// result without).
    fn format(&self, lang: &str, code: &str) -> Option<Result<String, String>>;
}

impl<F> CodeFormatter for F
where
    F: Fn(&str, &str) -> Option<Result<String, String>>,
{
    fn format(&self, lang: &str, code: &str) -> Option<Result<String, String>> {
        self(lang, code)
    }
}

/// A [`CodeFormatter`] that runs external commands, one per language (matched ignoring
/// ASCII case), like `fmt.code_formatters`: each gets the code on stdin and writes the
/// formatted code to stdout. A command fails when it cannot be started, exits with a
/// non-zero status (its stderr goes into the error), runs longer than the timeout (it is
/// killed) or writes output that is not UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFormatter {
    commands: BTreeMap<String, Vec<String>>,
    timeout: Duration,
}

impl CommandFormatter {
    /// How long a command may run by default.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

    /// Runs `commands`: language → program and arguments.
    pub fn new(commands: &BTreeMap<String, Vec<String>>) -> Self {
        Self {
            commands: commands
                .iter()
                .map(|(lang, argv)| (lang.to_ascii_lowercase(), argv.clone()))
                .collect(),
            timeout: Self::DEFAULT_TIMEOUT,
        }
    }

    /// Runs the `code_formatters` of `config`. Relative program paths are relative to
    /// the current directory; [`from_loaded`](Self::from_loaded) resolves them against
    /// their config file.
    pub fn from_config(config: &FmtConfig) -> Self {
        Self::new(&config.code_formatters)
    }

    /// Runs the `fmt.code_formatters` of `loaded`, with a relative program path that
    /// names a directory (`./tools/fmt.sh`, not `rustfmt`) resolved against the
    /// directory of the config file that declares it. Bare names are looked up on
    /// `PATH`, and arguments are passed as written.
    pub fn from_loaded(loaded: &LoadedConfig) -> Self {
        let mut commands = loaded.config().fmt.code_formatters.clone();
        for (lang, argv) in &mut commands {
            process::resolve_program(loaded, &["fmt", "code_formatters", lang.as_str()], argv);
        }
        Self::new(&commands)
    }

    /// Kills commands that run longer than `timeout`.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl CodeFormatter for CommandFormatter {
    fn format(&self, lang: &str, code: &str) -> Option<Result<String, String>> {
        let command = self.commands.get(&lang.to_ascii_lowercase())?;
        let Some((program, args)) = command.split_first() else {
            return Some(Err("no command is configured".to_owned()));
        };
        Some(run(program, args, code, self.timeout))
    }
}

/// Runs `program` with `args`, `input` on stdin, and returns its stdout as text.
fn run(program: &str, args: &[String], input: &str, timeout: Duration) -> Result<String, String> {
    let stdout =
        process::run(program, args, input.as_bytes(), timeout).map_err(|err| err.to_string())?;
    String::from_utf8(stdout).map_err(|_| format!("`{program}` wrote output that is not UTF-8"))
}
