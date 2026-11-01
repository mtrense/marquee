//! `mrq lsp`: the language server of mrq-lsp on stdin and stdout.

use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

use mrq_core::config::user_config_path;
use mrq_lsp::ServerOptions;

use crate::cli::{GlobalArgs, LspArgs};

/// Serves one client until it sends `exit`: 0 after a `shutdown`, else 1 (as LSP asks);
/// 2 if the runtime cannot start. Each root folder's configuration is loaded as the
/// other commands load it from the current directory: the user config, then `--config`
/// (relative to the current directory) or the `.mrq.yaml` files from the folder up,
/// then `--flavor`.
pub fn command(_args: &LspArgs, global: &GlobalArgs) -> ExitCode {
    let config = global.config.as_ref().map(|path| match env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.clone(),
    });
    let options = ServerOptions {
        user_config: user_config_path(|key| env::var(key).ok()),
        config,
        flavor: global.flavor.map(crate::flavor_name),
    };
    match mrq_lsp::run_stdio(options) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            let _ = writeln!(io::stderr().lock(), "mrq: lsp: {err}");
            ExitCode::from(crate::EXIT_USAGE)
        }
    }
}
