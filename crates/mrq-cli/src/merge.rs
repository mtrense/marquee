//! `mrq merge`: the inputs combined into one document by [`mrq_core::transform::merge`],
//! written to stdout or to `-o`.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mrq_core::document::DocumentOptions;
use mrq_core::input::Input;
use mrq_core::transform::{FrontmatterMerge, MergeOptions, merge};

use crate::cli::{FrontmatterMode, GlobalArgs, MergeArgs};

/// Merges the inputs, in order, and writes the result to `-o` (atomically, creating its
/// directory) or stdout. Links are made relative to the output file, or to the current
/// directory for stdout, and `/…` links are read from the current directory. The output
/// file is never an input, so merging a directory into a file in it can be repeated.
///
/// Warnings (clamped headings, anchors that could not be mapped) go to stderr in the
/// `--format` and never change the exit code. The exit code is 2 if an input cannot be
/// read, in which case nothing is written, or if the output cannot be written.
pub fn run(
    args: &MergeArgs,
    inputs: &[Input],
    options: &DocumentOptions,
    global: &GlobalArgs,
) -> ExitCode {
    let output = args.output.as_deref().map(canonical);
    let mut docs = Vec::new();
    let mut failed = false;
    for input in inputs {
        if output.is_some() && input.path().map(canonical) == output {
            continue;
        }
        match options.read(input) {
            Ok(doc) => docs.push(doc),
            Err(err) => {
                report(&err.to_string());
                failed = true;
            }
        }
    }
    if failed {
        return ExitCode::from(crate::EXIT_USAGE);
    }

    let mut merge_options = MergeOptions::new()
        .wrap(!args.no_wrap)
        .frontmatter(strategy(args.frontmatter));
    if let Some(title) = &args.title {
        merge_options = merge_options.title(title);
    }
    if let Some(shift) = args.shift {
        merge_options = merge_options.shift(shift);
    }
    if let Some(path) = &args.output {
        merge_options = merge_options.output(path);
    }
    let merged = merge(&docs, &merge_options);
    let sources: Vec<_> = docs.iter().map(|doc| doc.source().clone()).collect();
    crate::emit_diagnostics(&merged.diagnostics, &sources, global);

    let Some(path) = &args.output else {
        let _ = io::stdout().lock().write_all(merged.text.as_bytes());
        return ExitCode::SUCCESS;
    };
    let written = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => fs::create_dir_all(dir),
        _ => Ok(()),
    }
    .and_then(|()| crate::write_atomic(path, &merged.text));
    if let Err(err) = written {
        report(&format!("{}: {err}", path.display()));
        return ExitCode::from(crate::EXIT_USAGE);
    }
    ExitCode::SUCCESS
}

fn strategy(mode: FrontmatterMode) -> FrontmatterMerge {
    match mode {
        FrontmatterMode::KeepFirst => FrontmatterMerge::KeepFirst,
        FrontmatterMode::Merge => FrontmatterMerge::Merge,
        FrontmatterMode::Strip => FrontmatterMerge::Strip,
    }
}

/// `path` with symlinks and `..` resolved as far as it exists, to compare inputs with
/// the output.
fn canonical(path: &Path) -> PathBuf {
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    if let Ok(path) = fs::canonicalize(path) {
        return path;
    }
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => canonical(dir).join(name),
        _ => std::path::absolute(path).unwrap_or_else(|_| path.to_owned()),
    }
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
