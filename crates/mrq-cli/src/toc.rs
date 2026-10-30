//! `mrq toc`: print the heading outline of each input.

use std::io::{self, Write};
use std::process::ExitCode;

use mrq_core::document::{Document, DocumentOptions};
use mrq_core::input::Input;
use mrq_core::toc::{self, JsonToc, TocEntry};

use crate::cli::TocArgs;

/// Prints the outline of every input: indented text, a `--links` list or `--json`.
///
/// With several inputs, the text forms put a `==> name <==` header before each file and
/// a blank line between files; `--json` is always an array with one object per input.
/// Inputs that cannot be read are reported on stderr and skipped, and make the exit
/// code 2.
pub fn run(args: &TocArgs, inputs: &[Input], options: &DocumentOptions) -> ExitCode {
    let mut failed = false;
    let mut docs = Vec::new();
    // Reads lazily so text output of earlier files is not held back by later ones.
    let mut read = |input: &Input| match options.read(input) {
        Ok(doc) => Some(doc),
        Err(err) => {
            let _ = writeln!(io::stderr().lock(), "mrq: {err}");
            failed = true;
            None
        }
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if args.json {
        for input in inputs {
            if let Some(doc) = read(input) {
                docs.push(JsonToc::new(&doc, &entries(&doc, args.depth)));
            }
        }
        let mut json = serde_json::to_string_pretty(&docs).expect("a toc serializes");
        json.push('\n');
        let _ = out.write_all(json.as_bytes());
    } else {
        let mut first = true;
        for input in inputs {
            let Some(doc) = read(input) else { continue };
            let entries = entries(&doc, args.depth);
            if inputs.len() > 1 {
                let separator = if first { "" } else { "\n" };
                let _ = writeln!(out, "{separator}==> {} <==", doc.name().display());
            }
            first = false;
            let text = if args.links {
                toc::link_list(&entries)
            } else {
                toc::outline(&entries)
            };
            let _ = out.write_all(text.as_bytes());
        }
    }
    if failed {
        ExitCode::from(crate::EXIT_USAGE)
    } else {
        ExitCode::SUCCESS
    }
}

/// The document's headings up to level `depth`.
fn entries(doc: &Document, depth: Option<u8>) -> Vec<TocEntry> {
    let mut entries = doc.toc();
    if let Some(depth) = depth {
        entries.retain(|entry| entry.level <= depth);
    }
    entries
}
