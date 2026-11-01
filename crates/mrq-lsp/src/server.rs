//! [`Backend`]: the [`LanguageServer`] implementation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use mrq_core::source::PositionEncoding;
use tower_lsp_server::jsonrpc::{Error, Result};
use tower_lsp_server::ls_types::{
    CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions,
    DidChangeWorkspaceFoldersParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentSymbolParams, DocumentSymbolResponse, FileChangeType, FileSystemWatcher, GlobPattern,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, Location, MessageType, OneOf, Position,
    PrepareRenameResponse, ReferenceParams, Registration, RenameFilesParams, RenameOptions,
    RenameParams, ServerCapabilities, ServerInfo, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, Uri,
    WorkDoneProgressOptions, WorkspaceEdit, WorkspaceFileOperationsServerCapabilities,
    WorkspaceFoldersServerCapabilities, WorkspaceServerCapabilities, WorkspaceSymbolParams,
    WorkspaceSymbolResponse,
};
use tower_lsp_server::{Client, LanguageServer};

use crate::ServerOptions;
use crate::convert::{self, uri_to_path};
use crate::diagnostics::{Change, Publisher};
use crate::index::{IndexReport, WorkspaceIndex, is_config_file, is_indexed_kind};
use crate::store::DocumentStore;
use crate::{complete, navigate, rename, symbols};

/// The globs of the files the server asks the client to watch: Markdown files and the
/// config files (`.mrq.yaml`, markdownlint's).
const WATCHED_GLOBS: [&str; 3] = [
    "**/*.{md,markdown}",
    "**/.mrq.{yaml,yml}",
    "**/.markdownlint*",
];

/// The (old, new) paths of the `file:` URIs of a rename.
fn renamed_paths(params: &RenameFilesParams) -> Vec<(PathBuf, PathBuf)> {
    let path = |uri: &str| uri_to_path(&uri.parse().ok()?);
    params
        .files
        .iter()
        .filter_map(|rename| Some((path(&rename.old_uri)?, path(&rename.new_uri)?)))
        .collect()
}

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
    /// Whether the client takes versioned `documentChanges` in workspace edits.
    document_changes: bool,
}

impl State {
    /// `edits` as a workspace edit for the client; see [`rename::workspace_edit`].
    fn workspace_edit(&self, edits: rename::Edits) -> Option<WorkspaceEdit> {
        rename::workspace_edit(
            &self.index,
            &self.store,
            edits,
            self.encoding,
            self.document_changes,
        )
    }
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
            document_changes: false,
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

    /// Runs `read` on the state on a blocking thread (it may read files and walk the
    /// vault) and returns its answer.
    async fn read_blocking<T: Send + 'static>(
        &self,
        read: impl FnOnce(&State) -> T + Send + 'static,
    ) -> Result<T> {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            read(&state)
        })
        .await
        .map_err(|err| {
            let mut error = Error::internal_error();
            error.message = err.to_string().into();
            error
        })
    }

    /// Runs `read` with the path of `uri` and the byte offset of `position` in its
    /// indexed document (see [`read_blocking`](Self::read_blocking)); `None` if the
    /// document is not indexed.
    async fn read_at<T: Send + 'static>(
        &self,
        uri: &Uri,
        position: Position,
        read: impl FnOnce(&State, &Path, usize) -> Option<T> + Send + 'static,
    ) -> Result<Option<T>> {
        let Some(path) = uri_to_path(uri) else {
            return Ok(None);
        };
        self.read_blocking(move |state| {
            let document = state.index.document(&path)?;
            let offset = convert::offset(document.source(), position, state.encoding);
            read(state, &path, offset)
        })
        .await
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
        let workspace = params.capabilities.workspace.as_ref();
        let watch_files = workspace
            .and_then(|workspace| workspace.did_change_watched_files.as_ref())
            .and_then(|watched| watched.dynamic_registration)
            .unwrap_or(false);
        let document_changes = workspace
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.document_changes)
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
            state.document_changes = document_changes;
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
                file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                    will_rename: Some(rename::file_operations()),
                    did_rename: Some(rename::file_operations()),
                    ..WorkspaceFileOperationsServerCapabilities::default()
                }),
            }),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            document_symbol_provider: Some(OneOf::Left(true)),
            workspace_symbol_provider: Some(OneOf::Left(true)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            rename_provider: Some(OneOf::Right(RenameOptions {
                prepare_provider: Some(true),
                work_done_progress_options: WorkDoneProgressOptions::default(),
            })),
            completion_provider: Some(CompletionOptions {
                trigger_characters: Some(complete::TRIGGER_CHARACTERS.map(String::from).to_vec()),
                ..CompletionOptions::default()
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

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let at = params.text_document_position_params;
        let found = self
            .read_at(&at.text_document.uri, at.position, |state, path, offset| {
                navigate::definition(&state.index, path, offset, state.encoding)
            })
            .await?;
        Ok(found.map(GotoDefinitionResponse::Scalar))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let at = params.text_document_position;
        let include_declaration = params.context.include_declaration;
        self.read_at(
            &at.text_document.uri,
            at.position,
            move |state, path, offset| {
                let found = navigate::references(
                    &state.index,
                    path,
                    offset,
                    include_declaration,
                    state.encoding,
                );
                Some(found)
            },
        )
        .await
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let at = params.text_document_position_params;
        self.read_at(&at.text_document.uri, at.position, |state, path, offset| {
            navigate::hover(&state.index, path, offset, state.encoding)
        })
        .await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let at = params.text_document_position;
        let found = self
            .read_at(&at.text_document.uri, at.position, |state, path, offset| {
                complete::completions(&state.index, path, offset, state.encoding)
            })
            .await?;
        Ok(found.map(CompletionResponse::Array))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        self.read_at(
            &params.text_document.uri,
            params.position,
            |state, path, offset| {
                let (span, placeholder) = rename::heading_at(&state.index, path, offset)?;
                let source = state.index.document(path)?.source();
                Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: convert::range(source, span, state.encoding),
                    placeholder,
                })
            },
        )
        .await
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let at = params.text_document_position;
        let new_name = params.new_name;
        let renamed = self
            .read_at(
                &at.text_document.uri,
                at.position,
                move |state, path, offset| {
                    let renamed = rename::rename_heading(&state.index, path, offset, &new_name)?;
                    Some(renamed.map(|edits| state.workspace_edit(edits)))
                },
            )
            .await?;
        match renamed {
            Some(Ok(edit)) => Ok(edit),
            Some(Err(message)) => Err(Error::invalid_params(message)),
            None => Ok(None),
        }
    }

    async fn will_rename_files(&self, params: RenameFilesParams) -> Result<Option<WorkspaceEdit>> {
        let renames = renamed_paths(&params);
        self.read_blocking(move |state| {
            let edits = rename::rename_files(&state.index, &renames);
            state.workspace_edit(edits)
        })
        .await
    }

    async fn did_rename_files(&self, params: RenameFilesParams) {
        let renames = renamed_paths(&params);
        self.update_blocking(move |state| {
            let mut errors = Vec::new();
            for (old, new) in renames.iter().filter(|(old, _)| is_indexed_kind(old)) {
                errors.extend(
                    state
                        .index
                        .file_changed(old, FileChangeType::DELETED, &state.store),
                );
                errors.extend(
                    state
                        .index
                        .file_changed(new, FileChangeType::CREATED, &state.store),
                );
            }
            errors
        })
        .await;
        self.publisher.changed(Change::Updated);
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let Some(path) = uri_to_path(&params.text_document.uri) else {
            return Ok(None);
        };
        let state = self.state();
        let symbols = state
            .index
            .document(&path)
            .map(|document| symbols::document_symbols(document, state.encoding));
        Ok(symbols.map(DocumentSymbolResponse::Nested))
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<WorkspaceSymbolResponse>> {
        let state = self.state();
        let symbols = symbols::workspace_symbols(&state.index, &params.query, state.encoding);
        Ok(Some(WorkspaceSymbolResponse::Nested(symbols)))
    }
}
