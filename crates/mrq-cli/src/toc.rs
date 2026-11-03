//! `mrq toc`: print the heading outline of each input, or refresh its `mrq:toc`
//! regions (`--insert`).

use std::io::{self, Write};
use std::process::ExitCode;

use mrq_core::Slashed as _;
use mrq_core::document::{Document, DocumentOptions};
use mrq_core::input::Input;
use mrq_core::source::{SourceFile, apply_edits};
use mrq_core::toc::{self, JsonToc, TocEntry};

use crate::cli::{GlobalArgs, TocArgs};

/// Exit code of `--check` when a table of contents is out of date.
const EXIT_STALE: u8 = 1;

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
                let _ = writeln!(out, "{separator}==> {} <==", doc.name().slashed());
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

/// `--insert`: refreshes the `<!-- mrq:toc -->` regions of every input
/// ([`toc::refresh_edits`]).
///
/// Changed files are rewritten atomically; files that are up to date or have no region
/// are not touched. Stdin is never written back: its refreshed text (unchanged if there
/// is nothing to refresh) goes to stdout. With `--check` nothing is written or printed
/// except a `<name>: table of contents is out of date` line per stale input, and the
/// exit code is 1 if there is one. Regions with an invalid `depth` attribute are
/// reported on stderr and skipped; they, unreadable inputs and failed writes make the
/// exit code 2.
pub fn insert(
    args: &TocArgs,
    inputs: &[Input],
    options: &DocumentOptions,
    global: &GlobalArgs,
) -> ExitCode {
    let (mut failed, mut stale) = (false, false);
    let stdout = io::stdout();
    for input in inputs {
        let doc = match options.read(input) {
            Ok(doc) => doc,
            Err(err) => {
                let _ = writeln!(io::stderr().lock(), "mrq: {err}");
                failed = true;
                continue;
            }
        };
        let refresh = toc::refresh_edits(&doc, args.depth);
        if !refresh.diagnostics.is_empty() {
            let file = SourceFile::new(Some(doc.name().to_path_buf()), doc.text());
            crate::emit_diagnostics(&refresh.diagnostics, &[file], global);
            failed = true;
        }
        let text = apply_edits(doc.text(), &refresh.edits)
            .expect("refresh edits target disjoint regions of the document");
        let changed = text != doc.text();
        if args.check {
            if changed {
                stale = true;
                let _ = writeln!(
                    stdout.lock(),
                    "{}: table of contents is out of date",
                    doc.name().slashed()
                );
            }
        } else if let Some(path) = doc.path().filter(|_| input.is_writable()) {
            if changed && let Err(err) = crate::write_atomic(path, &text) {
                let _ = writeln!(io::stderr().lock(), "mrq: {}: {err}", path.slashed());
                failed = true;
            }
        } else {
            let _ = stdout.lock().write_all(text.as_bytes());
        }
    }
    if failed {
        ExitCode::from(crate::EXIT_USAGE)
    } else if stale {
        ExitCode::from(EXIT_STALE)
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
