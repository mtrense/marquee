//! `mrq fmt`: the inputs printed as canonical Markdown by [`mrq_core::fmt`], rewritten in
//! place, or checked with `--check`.

use std::io::{self, Write};
use std::process::ExitCode;

use mrq_core::Slashed as _;
use mrq_core::config::LoadedConfig;
use mrq_core::diag::Diagnostic;
use mrq_core::document::DocumentOptions;
use mrq_core::fmt::{CommandFormatter, format_with};
use mrq_core::input::Input;
use mrq_core::source::SourceFile;

use crate::cli::{FmtArgs, GlobalArgs};

/// Exit code of `--check` when an input is not formatted.
const EXIT_UNFORMATTED: u8 = 1;

/// Formats every input with the `fmt` settings of the configuration and rewrites the
/// files that change atomically; formatted files are not touched. Stdin is never written
/// back: its formatted text goes to stdout.
///
/// Fenced code is formatted by the configured `fmt.code_formatters`, a relative program
/// path resolved against the config file that declares it
/// ([`CommandFormatter::from_loaded`]). What fmt leaves as written (code a formatter
/// failed on, frontmatter that cannot be normalized) is reported as warnings on stderr in
/// the `--format`, after all inputs; warnings never change the exit code.
///
/// `--check` writes nothing and prints, for each input that is not formatted, a unified
/// diff from `a/<name>` to `b/<name>` on stdout, and exits 1 if there is one.
///
/// The exit code is 2 (taking precedence) if an input cannot be read or a file cannot be
/// written; those inputs are reported on stderr and skipped, the others still formatted.
pub fn run(
    args: &FmtArgs,
    inputs: &[Input],
    options: &DocumentOptions,
    loaded: &LoadedConfig,
    global: &GlobalArgs,
) -> ExitCode {
    let config = &loaded.config().fmt;
    let code = CommandFormatter::from_loaded(loaded);
    let (mut failed, mut unformatted) = (false, false);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut sources = Vec::new();
    let stdout = io::stdout();
    for input in inputs {
        let doc = match options.read(input) {
            Ok(doc) => doc,
            Err(err) => {
                report(&err.to_string());
                failed = true;
                continue;
            }
        };
        let formatted = format_with(&doc, config, &code);
        let (original, text) = (doc.text(), formatted.text.as_str());
        if !formatted.diagnostics.is_empty() {
            diagnostics.extend(formatted.diagnostics);
            sources.push(SourceFile::new(Some(doc.name().to_path_buf()), original));
        }
        if args.check {
            if text != original {
                unformatted = true;
                let diff = crate::unified_diff(doc.name(), original, text);
                let _ = stdout.lock().write_all(diff.as_bytes());
            }
        } else if let Some(path) = doc.path() {
            if text != original
                && let Err(err) = crate::write_atomic(path, text)
            {
                report(&format!("{}: {err}", path.slashed()));
                failed = true;
            }
        } else {
            let _ = stdout.lock().write_all(text.as_bytes());
        }
    }

    // Stdout holds formatted text or diffs, so the warnings go to stderr.
    crate::emit_diagnostics(&diagnostics, &sources, global);

    if failed {
        ExitCode::from(crate::EXIT_USAGE)
    } else if unformatted {
        ExitCode::from(EXIT_UNFORMATTED)
    } else {
        ExitCode::SUCCESS
    }
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
