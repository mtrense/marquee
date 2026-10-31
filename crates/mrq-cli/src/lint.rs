//! `mrq lint`: the configured [`mrq_core::lint`] rules over the inputs, as diagnostics on
//! stdout in the selected `--format`, and their fixes.

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use mrq_core::config::LoadedConfig;
use mrq_core::diag::Diagnostic;
use mrq_core::document::DocumentOptions;
use mrq_core::input::Input;
use mrq_core::lint::{Linter, Registry};
use mrq_core::source::SourceFile;

use crate::cli::{GlobalArgs, LintArgs};

/// Exit code when findings are left (or, with `--diff`, when there is something to fix).
const EXIT_FINDINGS: u8 = 1;

/// Lints every input and prints the findings to stdout in the `--format` (human unless
/// set, or github under GitHub Actions); nothing in the human format when all is well.
///
/// The rules are those of `lint` in the configuration, or of the `.markdownlint.*` file
/// it falls back to; `--rule` runs only the named rules, aliases and tags, even ones the
/// configuration turns off ([`Linter::restrict_to`]). Findings come file by file in input
/// order and by position within a file; they are lint findings only, parse diagnostics
/// such as malformed frontmatter are `mrq validate`'s.
///
/// - `--fix` fixes every input to a fixpoint ([`Linter::fix`]), rewrites the files that
///   changed atomically and reports the findings left in the fixed text. Stdin is never
///   written back: its fixed text (unchanged if there is nothing to fix) goes to stdout,
///   and the findings go to stderr instead.
/// - `--fix-only` does the same but reports nothing.
/// - `--diff` writes nothing and prints, for each input `--fix` would change, a unified
///   diff from `a/<name>` to `b/<name>`.
///
/// The exit code is 1 if findings are left (none with `--fix-only`; with `--diff`, if
/// there is a diff), and 2 (taking precedence) if `--rule` names an unknown rule, an
/// input cannot be read, a rule's fix does not apply or a file cannot be written; those
/// inputs are reported on stderr and skipped, the others still linted.
pub fn run(
    args: &LintArgs,
    inputs: &[Input],
    options: &DocumentOptions,
    loaded: &LoadedConfig,
    global: &GlobalArgs,
) -> ExitCode {
    let usage = || ExitCode::from(crate::EXIT_USAGE);
    let Some(linter) = linter(args, loaded) else {
        return usage();
    };
    let fix = args.fix || args.fix_only;

    let (mut failed, mut changed, mut stdin_printed) = (false, false, false);
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
        let name = doc.name().to_path_buf();
        if !fix && !args.diff {
            diagnostics.extend(linter.check(&doc));
            sources.push(SourceFile::new(Some(name), doc.text()));
            continue;
        }

        let original = doc.text().to_owned();
        let fixed = match linter.fix(doc, options) {
            Ok(fixed) => fixed,
            Err(err) => {
                report(&err.to_string());
                failed = true;
                continue;
            }
        };
        let text = fixed.document.text();
        if args.diff {
            if text != original {
                changed = true;
                let diff = crate::unified_diff(&name, &original, text);
                let _ = stdout.lock().write_all(diff.as_bytes());
            }
            continue;
        }
        if let Some(path) = fixed.document.path() {
            if text != original
                && let Err(err) = crate::write_atomic(path, text)
            {
                report(&format!("{}: {err}", path.display()));
                failed = true;
            }
        } else {
            let _ = stdout.lock().write_all(text.as_bytes());
            stdin_printed = true;
        }
        if !args.fix_only {
            diagnostics.extend(fixed.diagnostics);
            sources.push(SourceFile::new(Some(name), text));
        }
    }

    if !args.fix_only && !args.diff {
        // Stdout holds the fixed stdin text, so the findings go next to it on stderr.
        if stdin_printed {
            crate::emit_diagnostics(&diagnostics, &sources, global);
        } else {
            let is_terminal = stdout.is_terminal();
            crate::emit_diagnostics_to(
                &mut stdout.lock(),
                is_terminal,
                &diagnostics,
                &sources,
                global,
            );
        }
    }

    let findings = if args.diff {
        changed
    } else {
        !args.fix_only && !diagnostics.is_empty()
    };
    if failed {
        usage()
    } else if findings {
        ExitCode::from(EXIT_FINDINGS)
    } else {
        ExitCode::SUCCESS
    }
}

/// The linter for the configuration and `--rule`; `None` (reported) if a rule name is
/// unknown.
fn linter(args: &LintArgs, loaded: &LoadedConfig) -> Option<Linter> {
    let linter = Linter::new(&Registry::builtin(), &loaded.config().lint)
        .inspect_err(|err| report(&err.to_string()))
        .ok()?;
    if args.rule.is_empty() {
        return Some(linter);
    }
    linter
        .restrict_to(&args.rule)
        .inspect_err(|err| report(&format!("--rule: {err}")))
        .ok()
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
