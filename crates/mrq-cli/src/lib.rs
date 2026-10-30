//! `mrq`: a Swiss-army knife for Markdown.
//!
//! The library target exposes the clap command tree ([`cli::command`]) so that
//! tooling such as `cargo xtask man` can generate documentation from it.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mrq_core::config::{
    ConfigError, ConfigLoader, FlavorName, LoadedConfig, Origin, user_config_path,
};
use mrq_core::diag::{self, Diagnostic, EmitOptions, HumanStyle, OutputFormat};
use mrq_core::document::DocumentOptions;
use mrq_core::input::{Input, InputResolver};
use mrq_core::source::SourceFile;

pub mod cli;
mod query;
mod stats;
mod toc;

use cli::{Cli, ColorChoice, Command, ConfigCommand, DiagnosticFormat, Flavor, GlobalArgs, Inputs};

/// Exit code for usage, configuration and I/O errors, and for commands not implemented yet.
const EXIT_USAGE: u8 = 2;

/// Executes a parsed command line and returns the process exit code.
pub fn run(cli: Cli) -> ExitCode {
    match cli.command {
        Command::Completions { shell } => {
            let mut cmd = cli::command();
            clap_complete::generate(shell, &mut cmd, "mrq", &mut io::stdout().lock());
            ExitCode::SUCCESS
        }
        Command::Config(ConfigCommand::Schema) => {
            print(&mrq_core::config::schema_json());
            ExitCode::SUCCESS
        }
        Command::Config(ConfigCommand::Show { origin }) => match load_config(&cli.global) {
            Ok(loaded) => {
                print(&loaded.to_yaml(origin));
                ExitCode::SUCCESS
            }
            Err(err) => {
                report_config_error(&err, &cli.global);
                ExitCode::from(EXIT_USAGE)
            }
        },
        Command::Toc(args) => {
            let Some((options, inputs)) = prepare(&cli.global, &args.inputs, false) else {
                return ExitCode::from(EXIT_USAGE);
            };
            if args.insert {
                toc::insert(&args, &inputs, &options, &cli.global)
            } else {
                toc::run(&args, &inputs, &options)
            }
        }
        Command::Stats(args) => {
            let Some((options, inputs)) = prepare(&cli.global, &args.inputs, true) else {
                return ExitCode::from(EXIT_USAGE);
            };
            stats::run(&args, &inputs, &options)
        }
        Command::Query(args) => {
            // Compiled first, so a bad query fails before any input is resolved or read.
            let Some(compiled) = query::compile(&args) else {
                return ExitCode::from(EXIT_USAGE);
            };
            let Some((options, inputs)) = prepare(&cli.global, &args.inputs, false) else {
                return ExitCode::from(EXIT_USAGE);
            };
            query::run(&args, &compiled, &inputs, &options)
        }
        other => {
            let _ = writeln!(
                io::stderr().lock(),
                "mrq: `{}` is not implemented yet",
                other.name()
            );
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Loads the configuration and resolves the input arguments of a document command,
/// reporting failures on stderr (`None`, exit 2). `-` reads stdin. Without paths, the
/// current directory is used if `default_to_cwd` is set (project-wide commands such as
/// `stats`); otherwise at least one path is required.
fn prepare(
    global: &GlobalArgs,
    inputs: &Inputs,
    default_to_cwd: bool,
) -> Option<(DocumentOptions, Vec<Input>)> {
    let loaded = match load_config(global) {
        Ok(loaded) => loaded,
        Err(err) => {
            report_config_error(&err, global);
            return None;
        }
    };
    let cwd = [PathBuf::from(".")];
    let paths = if inputs.paths.is_empty() && default_to_cwd {
        &cwd[..]
    } else {
        &inputs.paths[..]
    };
    if paths.is_empty() {
        let _ = writeln!(
            io::stderr().lock(),
            "mrq: no input given; pass files, directories, globs or `-` for stdin"
        );
        return None;
    }
    let resolved = InputResolver::new()
        .ignore_patterns(loaded.config().ignore.iter().cloned())
        .resolve(paths);
    match resolved {
        Ok(resolved) => Some((document_options(global, &loaded), resolved)),
        Err(err) => {
            let _ = writeln!(io::stderr().lock(), "mrq: {err}");
            None
        }
    }
}

/// Document options for `--flavor` and the configured flavor. A configured flavor that
/// is only the built-in default is left unset, so documents report it as the default.
fn document_options(global: &GlobalArgs, loaded: &LoadedConfig) -> DocumentOptions {
    let options = DocumentOptions::new();
    if let Some(flavor) = global.flavor {
        options.flavor(flavor_name(flavor).as_str())
    } else if let Some(Origin::File { .. }) = loaded.origin(&["flavor"]) {
        options.config_flavor(loaded.config().flavor.as_str())
    } else {
        options
    }
}

fn print(text: &str) {
    let _ = io::stdout().lock().write_all(text.as_bytes());
}

/// Loads the layered configuration for the current directory: user config (see
/// [`user_config_path`]; `MRQ_CONFIG_HOME` overrides its directory), then `--config` or
/// the discovered `.mrq.yaml` files, then `--flavor`.
fn load_config(global: &GlobalArgs) -> Result<LoadedConfig, ConfigError> {
    let start = env::current_dir().map_err(|source| ConfigError::Io {
        path: ".".into(),
        source,
    })?;
    ConfigLoader::new(start)
        .user_config(user_config_path(|key| env::var(key).ok()))
        .explicit(global.config.clone())
        .flavor(global.flavor.map(flavor_name))
        .load()
}

fn flavor_name(flavor: Flavor) -> FlavorName {
    match flavor {
        Flavor::Commonmark => FlavorName::Commonmark,
        Flavor::Gfm => FlavorName::Gfm,
        Flavor::Obsidian => FlavorName::Obsidian,
        Flavor::Mkdocs => FlavorName::Mkdocs,
    }
}

fn output_format(format: DiagnosticFormat) -> OutputFormat {
    match format {
        DiagnosticFormat::Human => OutputFormat::Human,
        DiagnosticFormat::Json => OutputFormat::Json,
        DiagnosticFormat::Ndjson => OutputFormat::Ndjson,
        DiagnosticFormat::Sarif => OutputFormat::Sarif,
        DiagnosticFormat::Github => OutputFormat::Github,
    }
}

/// Writes a config error to stderr: as a diagnostic in the selected `--format` when it
/// points into a readable file, else as a plain `mrq: …` line.
fn report_config_error(err: &ConfigError, global: &GlobalArgs) {
    let source = err.to_diagnostic().and_then(|diagnostic| {
        let text = fs::read_to_string(&diagnostic.file).ok()?;
        Some((
            diagnostic.clone(),
            SourceFile::new(Some(diagnostic.file), text),
        ))
    });
    let Some((diagnostic, file)) = source else {
        let _ = writeln!(io::stderr().lock(), "mrq: {err}");
        return;
    };
    emit_diagnostics(&[diagnostic], &[file], global);
}

/// Writes `diagnostics` about `files` to stderr in the selected `--format` (human unless
/// set or detected otherwise), honouring `--color`.
fn emit_diagnostics(diagnostics: &[Diagnostic], files: &[SourceFile], global: &GlobalArgs) {
    let stderr = io::stderr();
    let mut out = stderr.lock();
    let format = OutputFormat::select_from_env(global.format.map(output_format));
    let mut human = HumanStyle::detect(stderr.is_terminal(), |key| env::var(key).ok());
    human.color = match global.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => human.color,
    };
    let options = EmitOptions {
        human,
        ..EmitOptions::default()
    };
    let _ = diag::emit(format, diagnostics, files, &options, &mut out);
}

/// Replaces the file at `path` with `text` atomically: writes a temp file in the same
/// directory, gives it the original's permissions, then renames it over the original.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut temp = tempfile::Builder::new()
        .prefix(".mrq-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    if let Ok(metadata) = fs::metadata(path) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.persist(path).map_err(|err| err.error)?;
    Ok(())
}
