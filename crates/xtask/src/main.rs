//! Repository automation for mrq. Run via the `cargo xtask` alias.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
    }
}
