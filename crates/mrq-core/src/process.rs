//! Running external commands as filters: input on stdin, output from stdout, with a
//! timeout. Code formatters ([`CommandFormatter`](crate::fmt::CommandFormatter)) and the
//! diagram renderers of `mrq-render` use it.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{LoadedConfig, Origin};

/// Why [`run`] produced no output.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// The program could not be started; `not_found` when it does not exist (is not
    /// installed, or not on `PATH`).
    #[error("`{program}` could not be started: {reason}")]
    NotStarted {
        program: String,
        reason: String,
        not_found: bool,
    },
    /// It started but failed: a non-zero exit (with its stderr), a timeout, or output
    /// that was never closed.
    #[error("{0}")]
    Failed(String),
}

/// How often a running command is checked for having exited.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Runs `program` with `args` and `input` on stdin, and returns its stdout. A command
/// that runs longer than `timeout` is killed.
pub fn run(
    program: &str,
    args: &[String],
    input: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, RunError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| RunError::NotStarted {
            program: program.to_owned(),
            reason: err.to_string(),
            not_found: err.kind() == io::ErrorKind::NotFound,
        })?;
    // Stdin is written and the outputs read on threads of their own, so that a command
    // that writes before it has read everything cannot block on a full pipe.
    let stdin = child.stdin.take().map(|mut stdin| {
        let input = input.to_owned();
        // A command may exit without reading its input; that is not an error here.
        thread::spawn(move || drop(stdin.write_all(&input)))
    });
    let stdout = read_all(child.stdout.take());
    let stderr = read_all(child.stderr.take());
    let status = wait(&mut child, timeout)
        .map_err(|err| RunError::Failed(format!("`{program}` could not be waited for: {err}")))?;
    let Some(status) = status else {
        return Err(RunError::Failed(format!(
            "`{program}` timed out after {}",
            humanize(timeout)
        )));
    };
    if let Some(stdin) = stdin {
        drop(stdin.join());
    }
    // A process the command left running may hold the pipes open; it is not waited for
    // longer than the timeout.
    let stdout = stdout
        .recv_timeout(timeout)
        .map_err(|_| RunError::Failed(format!("`{program}` exited but kept its output open")))?;
    if !status.success() {
        let stderr = stderr.recv_timeout(timeout).unwrap_or_default();
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        return Err(RunError::Failed(if stderr.is_empty() {
            format!("`{program}` failed ({status})")
        } else {
            format!("`{program}` failed ({status}): {stderr}")
        }));
    }
    Ok(stdout)
}

/// `argv` (a command from the configuration at `key`) with a relative program path that
/// names a directory (`./tools/fmt.sh`, not `rustfmt`) resolved against the directory of
/// the config file that declares it. Bare names are left for a `PATH` lookup, and
/// arguments are kept as written.
pub fn resolve_program<S: AsRef<str>>(loaded: &LoadedConfig, key: &[S], argv: &mut [String]) {
    let Some(program) = argv.first_mut() else {
        return;
    };
    let path = Path::new(program.as_str());
    if path.is_absolute() || path.components().count() < 2 {
        return;
    }
    if let Some(Origin::File { path: file, .. }) = loaded.origin(key)
        && let Some(dir) = file.parent()
    {
        *program = dir.join(path).to_string_lossy().into_owned();
    }
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
fn wait(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
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
