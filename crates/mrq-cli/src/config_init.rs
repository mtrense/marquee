//! `mrq config init [--user|--project] [--force]`: writes the reference configuration.

use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mrq_core::Slashed as _;
use mrq_core::config::{CONFIG_FILE_NAMES, REFERENCE_CONFIG, user_config_path};
use mrq_core::source::write_atomic;

use crate::cli::{ConfigInitArgs, GlobalArgs};

/// Writes [`REFERENCE_CONFIG`] to the global `--config` file, the user config (`--user`,
/// located through `MRQ_CONFIG_HOME`, `XDG_CONFIG_HOME` or `HOME`) or `.mrq.yaml` in the
/// current directory (`--project`, the default), creating missing parent directories.
/// `--config` together with `--user` or `--project` is a usage error (exit code 2).
///
/// An existing target, or for the project default an existing `.mrq.yml` the new file
/// would shadow, is left alone with exit code 2 unless `--force`. The written path is
/// reported on stderr as `wrote PATH`, unless `--quiet`.
pub fn command(args: &ConfigInitArgs, global: &GlobalArgs) -> ExitCode {
    if global.config.is_some() && (args.user || args.project) {
        return fail("--config cannot be used with --user or --project");
    }
    let path = if let Some(path) = &global.config {
        path.clone()
    } else if args.user {
        match user_config_path(|key| env::var(key).ok()) {
            Some(path) => path,
            None => {
                return fail(
                    "cannot locate the user config: set HOME, XDG_CONFIG_HOME or MRQ_CONFIG_HOME",
                );
            }
        }
    } else {
        PathBuf::from(CONFIG_FILE_NAMES[0])
    };

    if !args.force {
        let existing = if args.user || global.config.is_some() {
            vec![path.clone()]
        } else {
            CONFIG_FILE_NAMES.iter().map(PathBuf::from).collect()
        };
        if let Some(existing) = existing.iter().find(|p| p.exists()) {
            return fail(&format!(
                "{} already exists (use --force to overwrite)",
                existing.slashed()
            ));
        }
    }

    if let Err(err) = write(&path) {
        return fail(&format!("{}: {err}", path.slashed()));
    }
    if global.quiet == 0 {
        let _ = writeln!(io::stderr().lock(), "wrote {}", path.slashed());
    }
    ExitCode::SUCCESS
}

fn write(path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    write_atomic(path, REFERENCE_CONFIG)
}

fn fail(message: &str) -> ExitCode {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
    ExitCode::from(crate::EXIT_USAGE)
}
