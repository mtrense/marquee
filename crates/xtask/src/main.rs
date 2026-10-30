//! Repository automation for mrq. Run via the `cargo xtask` alias.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod lint_parity;
mod man;

#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about = "Repository automation for mrq")]
struct Xtask {
    #[command(subcommand)]
    task: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Generate roff man pages for `mrq` and every subcommand.
    Man {
        /// Output directory.
        #[arg(long, value_name = "DIR", default_value = "target/man")]
        out: PathBuf,
    },
    /// Record markdownlint-cli2's findings and fixes on the lint fixtures, then run the
    /// parity test that compares them with mrq's (needs Node.js and network for npx).
    LintParity {
        /// Fixture root: one directory per rule (`md001/`, …) of `*.md` fixtures.
        #[arg(
            long,
            value_name = "DIR",
            default_value = "crates/mrq-core/tests/fixtures/lint"
        )]
        fixtures: PathBuf,
        /// markdownlint-cli2 command line, split on whitespace.
        #[arg(long, default_value = lint_parity::DEFAULT_COMMAND)]
        command: String,
        /// Only check that the recorded files are up to date; exit 1 if not.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    match Xtask::parse().task {
        Task::Man { out } => match man::generate(&out) {
            Ok(pages) => {
                for page in pages {
                    println!("{}", page.display());
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("xtask man: {}: {err}", out.display());
                ExitCode::FAILURE
            }
        },
        Task::LintParity {
            fixtures,
            command,
            check,
        } => lint_parity_task(&fixtures, &command, check),
    }
}

fn lint_parity_task(fixtures: &Path, command: &str, check: bool) -> ExitCode {
    let changed = match lint_parity::run(fixtures, command, check) {
        Ok(changed) => changed,
        Err(err) => {
            eprintln!("xtask lint-parity: {}: {err}", fixtures.display());
            return ExitCode::FAILURE;
        }
    };
    for fixture in &changed {
        let verb = if check { "stale" } else { "recorded" };
        println!("{verb}: {}", fixture.display());
    }
    if check {
        return if changed.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let status =
        std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()))
            .args(["test", "-p", "mrq-core", "--test", "lint_rules"])
            .status();
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("xtask lint-parity: cargo test: {err}");
            ExitCode::FAILURE
        }
    }
}
