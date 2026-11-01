//! Language server for `mrq` (`mrq lsp`), built on `tower-lsp-server`.
//!
//! [`serve`] runs the server over any pair of async streams, [`run_stdio`] over stdin
//! and stdout on a runtime of its own. The [`Backend`] keeps two things current:
//!
//! - a [`DocumentStore`](store::DocumentStore) of the open documents, synced
//!   incrementally (`didChange` ranges in the negotiated position encoding: UTF-8 if the
//!   client offers it, else UTF-16; see [`convert`]);
//! - a [`WorkspaceIndex`](index::WorkspaceIndex) of every Markdown file under the root
//!   folders, parsed, with anchors and links, updated on `didOpen`/`didChange`/
//!   `didClose`, on `workspace/didChangeWatchedFiles` (the server registers watchers for
//!   Markdown and config files when the client supports dynamic registration),
//!   `workspace/didChangeWorkspaceFolders`, and reloaded with the configuration when a
//!   config file changes.
//!
//! After every change it publishes the [`diagnostics`] of the open documents (`validate`'s
//! internal checks and `lint`), once [`ServerOptions::diagnostics_delay`] has passed
//! without another change.
//!
//! Requests read the index: [`navigate`] answers `textDocument/definition`,
//! `textDocument/references` and `textDocument/hover`, [`symbols`]
//! `textDocument/documentSymbol` and `workspace/symbol`.
//!
//! The custom request [`INDEX_METHOD`] returns an [`IndexReport`] of what is indexed,
//! for tests and debugging.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mrq_core::config::FlavorName;
use tokio::io::{AsyncRead, AsyncWrite};
use tower_lsp_server::{LspService, Server};

pub mod convert;
pub mod diagnostics;
pub mod index;
pub mod navigate;
mod server;
pub mod store;
pub mod symbols;

pub use index::{DocumentReport, IndexReport, RootReport};
pub use server::{Backend, State};
pub use tower_lsp_server::ls_types;

/// The custom request returning an [`IndexReport`] (no parameters).
pub const INDEX_METHOD: &str = "mrq/index";

/// What the server is started with, besides what the client sends: the configuration
/// layers that do not come from the root folders (`mrq lsp` passes the global flags).
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// The user config file, the lowest layer (e.g.
    /// [`user_config_path`](mrq_core::config::user_config_path)); ignored if missing.
    pub user_config: Option<PathBuf>,
    /// A config file used instead of discovering `.mrq.yaml` files (`--config`); an
    /// absolute path.
    pub config: Option<PathBuf>,
    /// The flavor of every document (`--flavor`), over the configuration.
    pub flavor: Option<FlavorName>,
    /// How long the documents must stay unchanged before their diagnostics are
    /// published ([`DIAGNOSTICS_DELAY`] by default).
    pub diagnostics_delay: Duration,
}

/// The default [`ServerOptions::diagnostics_delay`].
pub const DIAGNOSTICS_DELAY: Duration = Duration::from_millis(200);

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            user_config: None,
            config: None,
            flavor: None,
            diagnostics_delay: DIAGNOSTICS_DELAY,
        }
    }
}

/// Serves one client on `input` and `output` until it sends `exit` (or closes `input`).
/// Returns whether `shutdown` came first, as the exit code should tell (LSP: 0 if so,
/// else 1).
///
/// Messages are handled one at a time in order, so a `didChange` is never applied after
/// a later request.
pub async fn serve<I, O>(input: I, output: O, options: ServerOptions) -> bool
where
    I: AsyncRead + Unpin,
    O: AsyncWrite,
{
    let shut_down = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shut_down);
    let (service, socket) = LspService::build(move |client| Backend::new(client, options, flag))
        .custom_method(INDEX_METHOD, Backend::index_report)
        .finish();
    Server::new(input, output, socket)
        .concurrency_level(1)
        .serve(service)
        .await;
    shut_down.load(Ordering::SeqCst)
}

/// [`serve`] on stdin and stdout, on a new multi-threaded tokio runtime.
pub fn run_stdio(options: ServerOptions) -> io::Result<bool> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    Ok(runtime.block_on(serve(tokio::io::stdin(), tokio::io::stdout(), options)))
}
