//! Handing link targets the viewer does not show itself (web pages, `mailto:` addresses,
//! images and other local files) to another program: the [`Opener`] seam, and the
//! [`SystemOpener`] that asks the operating system.

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::process::{Command, Stdio};
use std::thread;

/// Opens a URL or a local path outside the viewer. Tests substitute one that records the
/// targets instead of launching anything.
pub trait Opener: fmt::Debug {
    /// Opens `target`, a URL as written in the link or a resolved file path.
    fn open(&mut self, target: &OsStr) -> io::Result<()>;
}

/// The operating system's opener: `open` on macOS, `explorer` on Windows, `xdg-open`
/// elsewhere. It runs detached from the terminal (no stdin, output discarded) and is not
/// waited for, so only a failure to start it is an error.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&mut self, target: &OsStr) -> io::Result<()> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        };
        let mut child = Command::new(program)
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        // Reaped in the background, so a slow opener does not hold up the viewer.
        thread::spawn(move || child.wait());
        Ok(())
    }
}
