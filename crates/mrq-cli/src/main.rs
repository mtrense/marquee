//! `mrq`: a Swiss-army knife for Markdown.

use clap::Parser;

/// Command-line interface of `mrq`. The full command tree arrives in M0.4.
#[derive(Debug, Parser)]
#[command(name = "mrq", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
