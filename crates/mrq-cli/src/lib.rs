//! `mrq`: a Swiss-army knife for Markdown.
//!
//! The library target exposes the clap command tree ([`cli::command`]) so that
//! tooling such as `cargo xtask man` can generate documentation from it.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use mrq_core::config::{ConfigError, ConfigLoader, FlavorName, LoadedConfig, user_config_path};
use mrq_core::diag::{self, EmitOptions, HumanStyle, OutputFormat};
use mrq_core::source::SourceFile;

pub mod cli;

use cli::{Cli, ColorChoice, Command, ConfigCommand, DiagnosticFormat, Flavor, GlobalArgs};

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
    let stderr = io::stderr();
    let mut out = stderr.lock();
    let source = err.to_diagnostic().and_then(|diagnostic| {
        let text = fs::read_to_string(&diagnostic.file).ok()?;
        Some((
            diagnostic.clone(),
            SourceFile::new(Some(diagnostic.file), text),
        ))
    });
    let Some((diagnostic, file)) = source else {
        let _ = writeln!(out, "mrq: {err}");
        return;
    };
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
    let _ = diag::emit(format, &[diagnostic], &[file], &options, &mut out);
}
