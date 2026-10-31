//! Formatting the code of fenced blocks: the [`CodeFormatter`] seam and
//! [`CommandFormatter`], which pipes the code through the external commands of
//! `fmt.code_formatters`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::FmtConfig;

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

    /// Runs the `code_formatters` of `config`.
    pub fn from_config(config: &FmtConfig) -> Self {
        Self::new(&config.code_formatters)
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

/// How often a running command is checked for having exited.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Runs `program` with `args`, `input` on stdin, and returns its stdout.
fn run(program: &str, args: &[String], input: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("`{program}` could not be started: {err}"))?;
    // Stdin is written and the outputs read on threads of their own, so that a command
    // that writes before it has read everything cannot block on a full pipe.
    let stdin = child.stdin.take().map(|mut stdin| {
        let input = input.to_owned();
        // A command may exit without reading its input; that is not an error here.
        thread::spawn(move || drop(stdin.write_all(input.as_bytes())))
    });
    let stdout = read_all(child.stdout.take());
    let stderr = read_all(child.stderr.take());
    let status = wait(&mut child, timeout)
        .map_err(|err| format!("`{program}` could not be waited for: {err}"))?;
    let Some(status) = status else {
        return Err(format!("`{program}` timed out after {}", humanize(timeout)));
    };
    if let Some(stdin) = stdin {
        drop(stdin.join());
    }
    // A process the command left running may hold the pipes open; it is not waited for
    // longer than the timeout.
    let stdout = stdout
        .recv_timeout(timeout)
        .map_err(|_| format!("`{program}` exited but kept its output open"))?;
    if !status.success() {
        let stderr = stderr.recv_timeout(timeout).unwrap_or_default();
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("`{program}` failed ({status})")
        } else {
            format!("`{program}` failed ({status}): {stderr}")
        });
    }
    String::from_utf8(stdout).map_err(|_| format!("`{program}` wrote output that is not UTF-8"))
}

/// Reads `pipe` to its end on a thread; the bytes arrive on the returned channel.
fn read_all(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            drop(pipe.read_to_end(&mut bytes));
            drop(sender.send(bytes));
        });
    }
    receiver
}

/// Waits for `child` to exit, for at most `timeout`; kills it and returns `None` when it
/// runs longer.
fn wait(child: &mut Child, timeout: Duration) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            // It may have exited in the meantime; reaping it is all that is left.
            drop(child.kill());
            drop(child.wait());
            return Ok(None);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// `duration` as `10s` or `300ms`.
fn humanize(duration: Duration) -> String {
    if duration.subsec_millis() == 0 && duration.as_secs() > 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}
