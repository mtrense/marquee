//! `mrq interpolate`: the inputs with their `mrq:include` directives expanded by
//! [`Includer`], printed, or their include regions refreshed in place (`--in-place`) or
//! checked (`--check`).

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use mrq_core::Slashed as _;
use mrq_core::config::LoadedConfig;
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::{Document, DocumentOptions};
use mrq_core::input::Input;
use mrq_core::source::{SourceFile, apply_edits};
use mrq_core::transform::Includer;
#[cfg(feature = "remote-include")]
use mrq_core::transform::{HttpLoader, HttpLoaderOptions, IncludeCache};

use crate::cli::{GlobalArgs, InterpolateArgs};

/// Exit code of `--check` when an include region is out of date.
const EXIT_STALE: u8 = 1;

/// Expands the `mrq:include` directives of every input and prints the results in input
/// order, the markers replaced by the included content ([`Includer::expand`]).
///
/// `--keep-directives` prints instead what `--in-place` writes: the markers kept and the
/// content between them regenerated ([`Includer::refresh`]; a bare start marker gets an
/// `<!-- mrq:end -->`). `--in-place` rewrites the files that change atomically; stdin is
/// never written back, its refreshed text goes to stdout. `--check` writes nothing and
/// prints, for each input `--in-place` would change, a unified diff from `a/<name>` to
/// `b/<name>`, and exits 1 if there is one. Only the expansion merges `frontmatter=merge`
/// frontmatter into the including file's; refreshing leaves it alone.
///
/// Includes resolve relative to the including file, `/…` (and every include of stdin)
/// from the current directory, and included files are parsed with the configured
/// flavor. Remote includes need `include.allow_remote`; fetched content is cached in the
/// user cache directory.
///
/// Problems with includes go to stderr in the `--format`, after all inputs. A failed
/// include is left as it is, and errors make the exit code 2, as do unreadable inputs
/// and failed writes (2 wins); warnings (unknown attributes, clamped headings) do not
/// change it.
pub fn run(
    args: &InterpolateArgs,
    inputs: &[Input],
    options: &DocumentOptions,
    loaded: &LoadedConfig,
    global: &GlobalArgs,
) -> ExitCode {
    let includer = includer(options, loaded);
    let refresh = args.in_place || args.check || args.keep_directives;
    let (mut failed, mut stale) = (false, false);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut sources: Vec<SourceFile> = Vec::new();
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
        let (text, found) = if refresh {
            let regions = includer.refresh(&doc);
            let edits: Vec<_> = regions.iter().filter_map(|r| r.edit.clone()).collect();
            let text = apply_edits(doc.text(), &edits)
                .expect("refresh edits target disjoint regions of the document");
            (
                text,
                regions.into_iter().flat_map(|r| r.diagnostics).collect(),
            )
        } else {
            let expanded = includer.expand(&doc);
            (expanded.text, expanded.diagnostics)
        };
        failed |= found.iter().any(|d| d.severity == Severity::Error);
        add_sources(&doc, &found, &mut sources);
        diagnostics.extend(found);

        let changed = text != doc.text();
        if args.check {
            if changed {
                stale = true;
                let diff = crate::unified_diff(doc.name(), doc.text(), &text);
                let _ = stdout.lock().write_all(diff.as_bytes());
            }
        } else if let Some(path) = doc.path().filter(|_| args.in_place && input.is_writable()) {
            if changed && let Err(err) = crate::write_atomic(path, &text) {
                report(&format!("{}: {err}", path.slashed()));
                failed = true;
            }
        } else {
            let _ = stdout.lock().write_all(text.as_bytes());
        }
    }

    // Stdout holds documents or diffs, so the diagnostics go to stderr.
    crate::emit_diagnostics(&diagnostics, &sources, global);

    if failed {
        ExitCode::from(crate::EXIT_USAGE)
    } else if stale {
        ExitCode::from(EXIT_STALE)
    } else {
        ExitCode::SUCCESS
    }
}

/// The includer for the configuration: `/…` from the current directory, included files
/// parsed with `options`, and remote includes as `include.allow_remote` says, fetched
/// through the user cache.
fn includer(options: &DocumentOptions, loaded: &LoadedConfig) -> Includer {
    let includer = Includer::new()
        .root(".")
        .document_options(options.clone())
        .allow_remote(loaded.config().include.allow_remote);
    #[cfg(feature = "remote-include")]
    let includer = includer.loader(HttpLoader::new(HttpLoaderOptions {
        cache: IncludeCache::in_user_cache_dir(),
        ..Default::default()
    }));
    includer
}

/// Adds the sources `diagnostics` point into to `sources`, for their snippets: `doc`'s,
/// and the included files', read again (remote ones have none).
fn add_sources(doc: &Document, diagnostics: &[Diagnostic], sources: &mut Vec<SourceFile>) {
    for file in diagnostics.iter().map(|d| &d.file) {
        if sources
            .iter()
            .any(|source| source.path() == Some(file.as_path()))
        {
            continue;
        }
        let text = if file == doc.name() {
            doc.text().to_owned()
        } else if let Some(text) = read_local(file) {
            text
        } else {
            continue;
        };
        sources.push(SourceFile::new(Some(file.clone()), text));
    }
}

/// The text of the local file `path`; `None` for URLs and unreadable files.
fn read_local(path: &Path) -> Option<String> {
    let url = path.to_str().is_some_and(|p| p.contains("://"));
    (!url).then(|| fs::read_to_string(path).ok()).flatten()
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
