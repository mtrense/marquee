//! `mrq`: a Swiss-army knife for Markdown.
//!
//! The library target exposes the clap command tree ([`cli::command`]) so that
//! tooling such as `cargo xtask man` can generate documentation from it.

use std::io::{self, Write};
use std::process::ExitCode;

pub mod cli;

use cli::{Cli, Command};

/// Exit code for usage and I/O errors, and for commands not implemented yet.
const EXIT_USAGE: u8 = 2;

/// Executes a parsed command line and returns the process exit code.
pub fn run(cli: Cli) -> ExitCode {
    match cli.command {
        Command::Completions { shell } => {
            let mut cmd = cli::command();
            clap_complete::generate(shell, &mut cmd, "mrq", &mut io::stdout().lock());
            ExitCode::SUCCESS
        }
        other => {
            let _ = writeln!(
                io::stderr().lock(),
                "mrq: `{}` is not implemented yet",
                other.name()
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}
