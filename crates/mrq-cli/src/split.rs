//! `mrq split`: a file split into one file per section by [`mrq_core::transform::split`],
//! plus an `index.md` that `mrq interpolate` expands back into it.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use mrq_core::document::DocumentOptions;
use mrq_core::input::Input;
use mrq_core::transform::{SplitOptions, split};

use crate::cli::{GlobalArgs, SplitArgs};
use crate::merge::canonical;

/// Splits the file at its headings of level `-d` and writes `index.md` and the parts to
/// `--out` (default: the file's path without its extension), creating it and replacing
/// files there atomically; other files in it are left alone. Links are made relative to
/// the new files, and `/…` links are read from the current directory. `--dry-run` writes
/// nothing and prints the paths that would be written, `index.md` first.
///
/// Warnings (sections whose headings are not promoted) go to stderr in the `--format`
/// and never change the exit code. The exit code is 2, with nothing written, if the
/// file cannot be read, has no heading to split at, or is one of the files to write
/// (stdin needs `--out`), and 2 if a file cannot be written.
pub fn run(args: &SplitArgs, options: &DocumentOptions, global: &GlobalArgs) -> ExitCode {
    let stdin = args.file == Path::new("-");
    if stdin && args.out.is_none() {
        report("splitting stdin needs --out");
        return ExitCode::from(crate::EXIT_USAGE);
    }
    let input = if stdin {
        Input::stdin()
    } else {
        Input::file(&args.file)
    };
    let doc = match options.read(&input) {
        Ok(doc) => doc,
        Err(err) => {
            report(&err.to_string());
            return ExitCode::from(crate::EXIT_USAGE);
        }
    };
    let mut split_options = SplitOptions::new().depth(args.depth);
    if let Some(out) = &args.out {
        split_options = split_options.dir(out);
    }
    let split = split(&doc, &split_options);
    crate::emit_diagnostics(&split.diagnostics, &[doc.source().clone()], global);
    if split.parts.is_empty() {
        report(&format!(
            "{}: no level-{} headings to split at",
            doc.name().display(),
            args.depth
        ));
        return ExitCode::from(crate::EXIT_USAGE);
    }

    let files: Vec<_> = std::iter::once(&split.index).chain(&split.parts).collect();
    if let Some(path) = doc.path() {
        let input = canonical(path);
        if let Some(file) = files.iter().find(|file| canonical(&file.path) == input) {
            report(&format!(
                "{}: splitting would overwrite the file itself",
                file.path.display()
            ));
            return ExitCode::from(crate::EXIT_USAGE);
        }
    }
    if args.dry_run {
        let mut stdout = io::stdout().lock();
        for file in &files {
            let _ = writeln!(stdout, "{}", file.path.display());
        }
        return ExitCode::SUCCESS;
    }
    let dir = match split.index.path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    if let Err(err) = fs::create_dir_all(dir) {
        report(&format!("{}: {err}", dir.display()));
        return ExitCode::from(crate::EXIT_USAGE);
    }
    let mut failed = false;
    for file in &files {
        if let Err(err) = crate::write_atomic(&file.path, &file.text) {
            report(&format!("{}: {err}", file.path.display()));
            failed = true;
        }
    }
    if failed {
        ExitCode::from(crate::EXIT_USAGE)
    } else {
        ExitCode::SUCCESS
    }
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
