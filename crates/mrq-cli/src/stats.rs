//! `mrq stats`: per-file and total counts of every input, as a table or `--json`.

use std::io::{self, Write};
use std::process::ExitCode;

use mrq_core::document::DocumentOptions;
use mrq_core::input::Input;
use mrq_core::stats::{Counts, Report, StatsOptions};

use crate::cli::StatsArgs;

/// Prints the [`Report`] of every input: an aligned table (with a `total` row for
/// several inputs) or, with `--json`, an object with `wpm`, `files` and `total`.
///
/// Inputs that cannot be read are reported on stderr and skipped, and make the exit
/// code 2; the others are still counted and printed.
pub fn run(args: &StatsArgs, inputs: &[Input], options: &DocumentOptions) -> ExitCode {
    let stats_options = StatsOptions::new().include_code(args.include_code);
    let mut report = Report::new(args.wpm);
    let mut failed = false;
    for input in inputs {
        match options.read(input) {
            Ok(doc) => {
                let counts = Counts::of(&doc, &stats_options);
                report.push(&doc, counts);
            }
            Err(err) => {
                let _ = writeln!(io::stderr().lock(), "mrq: {err}");
                failed = true;
            }
        }
    }
    let text = if args.json {
        let mut json = serde_json::to_string_pretty(&report).expect("a report serializes");
        json.push('\n');
        json
    } else {
        report.table()
    };
    let _ = io::stdout().lock().write_all(text.as_bytes());
    if failed {
        ExitCode::from(crate::EXIT_USAGE)
    } else {
        ExitCode::SUCCESS
    }
}
