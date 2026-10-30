//! `mrq`: a Swiss-army knife for Markdown.

use std::process::ExitCode;

use clap::Parser;
use mrq_cli::cli::Cli;

fn main() -> ExitCode {
    mrq_cli::run(Cli::parse())
}
