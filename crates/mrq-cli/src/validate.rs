//! `mrq validate`: every check of [`mrq_core::validate`] over the inputs, as diagnostics
//! on stdout in the selected `--format`.

use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use mrq_core::config::LoadedConfig;
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::{DocumentOptions, Workspace};
use mrq_core::input::Input;
use mrq_core::source::SourceFile;
#[cfg(feature = "frontmatter-schema")]
use mrq_core::validate::FrontmatterSchemas;
use mrq_core::validate::{Checker, ExternalChecker, ExternalOptions};

use crate::cli::{GlobalArgs, ValidateArgs};

/// Exit code when a check finds an error (or, with `--strict`, a warning).
const EXIT_FINDINGS: u8 = 1;

/// Validates every input and prints the diagnostics to stdout in the `--format` (human
/// unless set, or github under GitHub Actions); nothing in the human format when all is
/// well.
///
/// The inputs form one [`Workspace`] rooted at the current directory, so links starting
/// with `/` resolve from there. Diagnostics are what parsing found (malformed
/// frontmatter, directives, a bad frontmatter flavor) and what the [`Checker`] finds,
/// file by file in input order and by position within a file. Frontmatter schemas come
/// from `validate.frontmatter`; external links are checked with `--external` or
/// `validate.external`, except the `validate.external_exclude` ones.
///
/// The exit code is 1 if there is an error (with `--strict`, also a warning), and 2
/// (taking precedence) if the configuration is invalid, `--external` is given to a build
/// without the `external-links` feature, or an input cannot be read; unreadable inputs
/// are reported on stderr and skipped, the others still checked.
pub fn run(
    args: &ValidateArgs,
    inputs: &[Input],
    options: &DocumentOptions,
    loaded: &LoadedConfig,
    global: &GlobalArgs,
) -> ExitCode {
    let usage = || ExitCode::from(crate::EXIT_USAGE);
    #[cfg(feature = "frontmatter-schema")]
    let Some(schemas) = frontmatter_schemas(loaded) else {
        return usage();
    };
    #[cfg(not(feature = "frontmatter-schema"))]
    if !loaded.config().validate.frontmatter.is_empty() {
        report(
            "`validate.frontmatter` is ignored: this mrq was built without the \
             `frontmatter-schema` feature",
        );
    }
    let external = match ExternalOptions::from_config(loaded.config()) {
        Ok(external) => external.with_flag(args.external),
        Err(err) => {
            report(&format!("{err}: {}", err.source));
            return usage();
        }
    };
    let Ok(http) = external_checker(&external, args.external) else {
        return usage();
    };

    let mut failed = false;
    let mut workspace = Workspace::with_options(".", options.clone());
    for input in inputs {
        match options.read(input) {
            Ok(doc) => {
                workspace.insert(doc);
            }
            Err(err) => {
                report(&err.to_string());
                failed = true;
            }
        }
    }

    let mut checker = Checker::new(&workspace);
    #[cfg(feature = "frontmatter-schema")]
    {
        checker = checker.with_frontmatter_schemas(&schemas);
    }
    if let Some(http) = &http {
        checker = checker.with_external_links(http.as_ref(), &external.exclude);
    }
    let mut found: HashMap<PathBuf, Vec<Diagnostic>> = HashMap::new();
    for diagnostic in checker.check_workspace() {
        found
            .entry(diagnostic.file.clone())
            .or_default()
            .push(diagnostic);
    }

    let mut diagnostics = Vec::new();
    let mut sources = Vec::new();
    for doc in workspace.documents() {
        let mut own: Vec<Diagnostic> = doc.diagnostics().to_vec();
        own.extend(found.remove(doc.name()).unwrap_or_default());
        own.sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.range.end));
        diagnostics.extend(own);
        sources.push(SourceFile::new(Some(doc.name().to_path_buf()), doc.text()));
    }

    let stdout = io::stdout();
    let is_terminal = stdout.is_terminal();
    crate::emit_diagnostics_to(
        &mut stdout.lock(),
        is_terminal,
        &diagnostics,
        &sources,
        global,
    );

    let fails = |diagnostic: &Diagnostic| match diagnostic.severity {
        Severity::Error => true,
        Severity::Warning => args.strict,
        Severity::Info | Severity::Hint => false,
    };
    if failed {
        usage()
    } else if diagnostics.iter().any(fails) {
        ExitCode::from(EXIT_FINDINGS)
    } else {
        ExitCode::SUCCESS
    }
}

/// The schemas of `validate.frontmatter`; `None` (reported) if one cannot be loaded.
#[cfg(feature = "frontmatter-schema")]
fn frontmatter_schemas(loaded: &LoadedConfig) -> Option<FrontmatterSchemas> {
    FrontmatterSchemas::from_config(loaded)
        .inspect_err(|err| report(&err.to_string()))
        .ok()
}

/// The checker for external links if they are enabled: an `HttpChecker` caching working
/// URLs in the user cache directory. `Err` (reported) if it cannot be built, or if
/// `--external` (`flag`) asks for it in a build without the `external-links` feature;
/// enabling it only in the configuration then just warns.
#[cfg_attr(
    not(feature = "external-links"),
    allow(clippy::unnecessary_wraps, reason = "fails only with the feature")
)]
fn external_checker(
    external: &ExternalOptions,
    flag: bool,
) -> Result<Option<Box<dyn ExternalChecker>>, ()> {
    if !external.enabled {
        return Ok(None);
    }
    #[cfg(feature = "external-links")]
    {
        use mrq_core::validate::{CacheOptions, HttpChecker, HttpOptions};

        let _ = flag;
        let options = HttpOptions {
            cache: CacheOptions::in_user_cache_dir(),
            ..HttpOptions::default()
        };
        match HttpChecker::new(options) {
            Ok(http) => Ok(Some(Box::new(http))),
            Err(err) => {
                report(&err.to_string());
                Err(())
            }
        }
    }
    #[cfg(not(feature = "external-links"))]
    {
        let missing = "this mrq was built without the `external-links` feature";
        if flag {
            report(&format!(
                "cannot check external links (`--external`): {missing}"
            ));
            Err(())
        } else {
            report(&format!("`validate.external` is ignored: {missing}"));
            Ok(None)
        }
    }
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
