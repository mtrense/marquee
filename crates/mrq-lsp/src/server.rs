//! [`Backend`]: the [`LanguageServer`] implementation.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use mrq_core::source::PositionEncoding;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidChangeWorkspaceFoldersParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, FileSystemWatcher, GlobPattern,
    InitializeParams, InitializeResult, InitializedParams, MessageType, OneOf, Registration,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, WorkspaceFoldersServerCapabilities, WorkspaceServerCapabilities,
};
use tower_lsp_server::{Client, LanguageServer};

use crate::ServerOptions;
use crate::convert::{self, uri_to_path};
use crate::diagnostics::{Change, Publisher};
use crate::index::{IndexReport, WorkspaceIndex, is_config_file, is_indexed_kind};
use crate::store::DocumentStore;

/// The globs of the files the server asks the client to watch: Markdown files and the
/// config files (`.mrq.yaml`, markdownlint's).
const WATCHED_GLOBS: [&str; 3] = [
    "**/*.{md,markdown}",
    "**/.mrq.{yaml,yml}",
    "**/.markdownlint*",
];

/// What the server knows: the open documents and the workspace index.
#[derive(Debug)]
pub struct State {
    /// The position encoding negotiated in `initialize`.
    pub encoding: PositionEncoding,
    pub store: DocumentStore,
    pub index: WorkspaceIndex,
    /// The root folders from `initialize`, indexed in `initialized`.
    pending_roots: Vec<PathBuf>,
    /// Whether the client registers file watchers on request.
    watch_files: bool,
}

/// The language server. Handlers run one at a time, in the order the client sent them
/// (see [`serve`](crate::serve)), so the state they see is always current.
#[derive(Debug)]
pub struct Backend {
    client: Client,
    state: Arc<Mutex<State>>,
    /// Told about every change, to publish the diagnostics again.
    publisher: Publisher,
    /// Set by `shutdown`, so that `exit` can tell a clean stop.
    shut_down: Arc<AtomicBool>,
}

impl Backend {
    /// Must be called on a tokio runtime: it starts the diagnostics [`Publisher`].
    pub(crate) fn new(client: Client, options: ServerOptions, shut_down: Arc<AtomicBool>) -> Self {
        let delay = options.diagnostics_delay;
        let state = Arc::new(Mutex::new(State {
            encoding: PositionEncoding::Utf16,
            store: DocumentStore::new(),
            index: WorkspaceIndex::new(options),
            pending_roots: Vec::new(),
            watch_files: false,
        }));
        let publisher = Publisher::spawn(client.clone(), Arc::clone(&state), delay);
        Self {
            client,
            state,
            publisher,
            shut_down,
        }
    }

    /// The client connection, for notifications and requests to it.
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// The state, locked. Never hold the guard across an `.await`.
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Runs `update` on the state on a blocking thread (it reads and parses files) and
    /// shows the problems it returns to the user.
    async fn update_blocking(
        &self,
        update: impl FnOnce(&mut State) -> Vec<String> + Send + 'static,
    ) {
        let state = Arc::clone(&self.state);
        let errors = tokio::task::spawn_blocking(move || {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            update(&mut state)
        })
        .await
        .unwrap_or_else(|err| vec![format!("internal error: {err}")]);
        for error in errors {
            self.client
                .show_message(MessageType::ERROR, format!("mrq: {error}"))
                .await;
        }
    }

    /// The [`INDEX_METHOD`](crate::INDEX_METHOD) request.
    #[allow(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "tower-lsp-server's custom methods are async"
    )]
    pub(crate) async fn index_report(&self) -> Result<IndexReport> {
        let state = self.state();
        Ok(state.index.report(&state.store))
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the trait's handlers are async"
)]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let encoding = convert::negotiate_encoding(&params.capabilities);
        let watch_files = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.did_change_watched_files.as_ref())
            .and_then(|watched| watched.dynamic_registration)
            .unwrap_or(false);
        let roots = match &params.workspace_folders {
            Some(folders) => folders.iter().filter_map(|f| uri_to_path(&f.uri)).collect(),
            #[allow(deprecated, reason = "older clients only send rootUri")]
            None => params.root_uri.iter().filter_map(uri_to_path).collect(),
        };
        {
            let mut state = self.state();
            state.encoding = encoding;
            state.watch_files = watch_files;
            state.pending_roots = roots;
        }
        let capabilities = ServerCapabilities {
            position_encoding: Some(convert::encoding_kind(encoding)),
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::INCREMENTAL),
                    ..TextDocumentSyncOptions::default()
                },
            )),
            workspace: Some(WorkspaceServerCapabilities {
                workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                    supported: Some(true),
                    change_notifications: Some(OneOf::Left(true)),
                }),
                file_operations: None,
            }),
            ..ServerCapabilities::default()
        };
        Ok(InitializeResult {
            capabilities,
            server_info: Some(ServerInfo {
                name: "mrq".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            offset_encoding: None,
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.update_blocking(|state| {
            let roots = std::mem::take(&mut state.pending_roots);
            let mut errors = Vec::new();
            for root in roots {
                errors.extend(state.index.add_root(root, &state.store));
            }
            errors
        })
        .await;
        let watch_files = self.state().watch_files;
        if watch_files {
            let watchers = WATCHED_GLOBS
                .iter()
                .map(|glob| FileSystemWatcher {
                    glob_pattern: GlobPattern::String((*glob).into()),
                    kind: None,
                })
                .collect();
            let options = DidChangeWatchedFilesRegistrationOptions { watchers };
            let registration = Registration {
                id: "mrq-watched-files".into(),
                method: "workspace/didChangeWatchedFiles".into(),
                register_options: serde_json::to_value(options).ok(),
            };
            if let Err(err) = self.client.register_capability(vec![registration]).await {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("mrq: cannot watch files: {err}"),
                    )
                    .await;
            }
        }
    }

    async fn shutdown(&self) -> Result<()> {
        self.shut_down.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        let path = uri_to_path(&document.uri);
        let mut state = self.state();
        let state = &mut *state;
        state
            .store
            .open(document.uri.clone(), path, document.version, document.text);
        if let Some(open) = state.store.get(&document.uri) {
            state.index.update(open.source.clone());
        }
        self.publisher.changed(Change::Updated);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let document = params.text_document;
        let mut state = self.state();
        let state = &mut *state;
        let changed = state.store.change(
            &document.uri,
            document.version,
            params.content_changes,
            state.encoding,
        );
        if let Some(open) = changed {
            state.index.update(open.source.clone());
            self.publisher.changed(Change::Updated);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let mut state = self.state();
        let state = &mut *state;
        let closed = state.store.close(&uri);
        if let Some(path) = closed.as_ref().and_then(|doc| doc.source.path()) {
            state.index.close(path, &state.store);
        }
        self.publisher.changed(Change::Closed(uri));
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let changes: Vec<_> = params
            .changes
            .into_iter()
            .filter_map(|event| Some((uri_to_path(&event.uri)?, event.typ)))
            .collect();
        self.update_blocking(move |state| {
            // A new configuration re-reads everything anyway.
            if changes.iter().any(|(path, _)| is_config_file(path)) {
                return state.index.reload_config(&state.store);
            }
            let mut errors = Vec::new();
            for (path, change) in changes.iter().filter(|(path, _)| is_indexed_kind(path)) {
                errors.extend(state.index.file_changed(path, *change, &state.store));
            }
            errors
        })
        .await;
        self.publisher.changed(Change::Updated);
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        let event = params.event;
        self.update_blocking(move |state| {
            for folder in &event.removed {
                if let Some(path) = uri_to_path(&folder.uri) {
                    state.index.remove_root(&path, &state.store);
                }
            }
            let mut errors = Vec::new();
            for folder in &event.added {
                if let Some(path) = uri_to_path(&folder.uri) {
                    errors.extend(state.index.add_root(path, &state.store));
                }
            }
            errors
        })
        .await;
        self.publisher.changed(Change::Updated);
    }
}
