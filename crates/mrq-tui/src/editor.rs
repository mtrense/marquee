//! Handing the shown file to an external editor (`e`): the [`Editor`] seam, and the
//! [`SystemEditor`] that runs `$VISUAL` or `$EDITOR` on the terminal the viewer gives up
//! for it.

use std::env;
use std::fmt;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::Command;

use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Edits a file outside the viewer and returns when the editing is done. Tests
/// substitute one that records the request (and maybe writes the file) instead of
/// launching anything.
pub trait Editor: fmt::Debug {
    /// Edits the file `path` with the cursor on `line` (1-based), waiting for it.
    fn edit(&mut self, path: &Path, line: usize) -> io::Result<()>;
}

/// The user's editor, `$EDITOR +line file`: the command in `$VISUAL`, else `$EDITOR`,
/// else `vi`, split at whitespace (so `code --wait` works, quoting does not). While it
/// runs, the terminal leaves raw mode and the alternate screen, and is set up again
/// afterwards; the viewer clears and redraws it then.
#[derive(Debug, Clone, Default)]
pub struct SystemEditor {
    /// The command; `None` reads the environment on each edit.
    command: Option<String>,
}

impl SystemEditor {
    /// An editor running `command` (a program and its first arguments).
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: Some(command.into()),
        }
    }

    /// The editor for the values of `$VISUAL` and `$EDITOR`: the first that is not blank,
    /// else `vi`.
    pub fn from_vars(visual: Option<&str>, editor: Option<&str>) -> Self {
        let command = [visual, editor]
            .into_iter()
            .flatten()
            .find(|command| !command.trim().is_empty())
            .unwrap_or("vi");
        Self::new(command)
    }

    /// The command that edits `path` at `line`: the editor's words, `+line`, `path`.
    pub fn command(&self, path: &Path, line: usize) -> Command {
        let configured = self.command.clone().unwrap_or_else(|| {
            let var = |name| env::var(name).ok();
            let editor = Self::from_vars(var("VISUAL").as_deref(), var("EDITOR").as_deref());
            editor.command.unwrap_or_default()
        });
        let mut words = configured.split_whitespace();
        let mut command = Command::new(words.next().unwrap_or("vi"));
        command.args(words).arg(format!("+{line}")).arg(path);
        command
    }
}

impl Editor for SystemEditor {
    fn edit(&mut self, path: &Path, line: usize) -> io::Result<()> {
        let mut command = self.command(path, line);
        let program = command.get_program().to_string_lossy().into_owned();
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
        let status = command.status();
        // The viewer's terminal comes back even if the editor did not start.
        execute!(io::stdout(), EnterAlternateScreen)?;
        enable_raw_mode()?;
        io::stdout().flush()?;
        let status = status
            .map_err(|err| io::Error::new(err.kind(), format!("cannot run `{program}`: {err}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "`{program}` exited with {status}"
            )))
        }
    }
}
