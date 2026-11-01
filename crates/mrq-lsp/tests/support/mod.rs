//! A test harness that speaks LSP to an in-process `mrq-lsp` server over in-memory pipes.
//!
//! Each test file that needs it declares `mod support;`. A [`TestClient`] starts the
//! server ([`mrq_lsp::serve`]) on a [`tokio::io::duplex`] pipe, frames JSON-RPC messages
//! with `Content-Length` headers, answers the server's own requests
//! (`client/registerCapability` and the like, with `null`, recording them), and queues
//! the server's notifications until a test waits for one. [`Fixture`] is a temporary
//! directory of files to use as a workspace folder.
//!
//! ```ignore
//! let fixture = Fixture::new(&[("a.md", "# A\n")]);
//! let mut client = TestClient::start();
//! client.initialize(&[fixture.root()]).await;
//! let uri = client.open(&fixture.path("a.md"), "# B\n").await;
//! let report = client.index().await;
//! client.shutdown().await;
//! ```

#![allow(dead_code, reason = "each test file uses part of the harness")]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mrq_core::source::{PositionEncoding, SourceFile, TextEdit as SourceEdit, apply_edits};
use mrq_lsp::ls_types::notification::{self, Notification};
use mrq_lsp::ls_types::request::{self, Request};
use mrq_lsp::ls_types::{
    ClientCapabilities, CodeAction, CodeActionContext, CodeActionKind, CodeActionOrCommand,
    CodeActionParams, CompletionItem, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidChangeWatchedFilesClientCapabilities,
    DidChangeWatchedFilesParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentChanges, DocumentFormattingParams, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, FileChangeType, FileEvent, FileRename, FormattingOptions,
    GotoDefinitionParams, GotoDefinitionResponse, HoverContents, HoverParams, InitializeParams,
    InitializeResult, InitializedParams, Location, MarkupKind, NumberOrString, OneOf,
    PartialResultParams, Position, PrepareRenameResponse, PublishDiagnosticsParams, Range,
    ReferenceContext, ReferenceParams, RenameFilesParams, RenameParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, TextEdit, Uri, VersionedTextDocumentIdentifier,
    WorkDoneProgressParams, WorkspaceClientCapabilities, WorkspaceEdit,
    WorkspaceEditClientCapabilities, WorkspaceFolder, WorkspaceSymbol, WorkspaceSymbolParams,
};
use mrq_lsp::{INDEX_METHOD, IndexReport, ServerOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, WriteHalf};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// How long [`TestClient`] waits for a response or notification before failing the test.
pub const TIMEOUT: Duration = Duration::from_secs(10);

type Writer = Arc<tokio::sync::Mutex<WriteHalf<DuplexStream>>>;
type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>;

/// The client end of a connection to an in-process server.
pub struct TestClient {
    writer: Writer,
    next_id: i64,
    pending: Pending,
    notifications: mpsc::UnboundedReceiver<Value>,
    /// Notifications received while waiting for another one, oldest first.
    backlog: VecDeque<Value>,
    server_requests: Arc<Mutex<Vec<Value>>>,
    /// The version last sent per open document.
    versions: HashMap<Uri, i32>,
    server: JoinHandle<bool>,
    reader: JoinHandle<()>,
}

impl TestClient {
    /// Starts a server with options that read no user configuration and publish
    /// diagnostics without delay.
    pub fn start() -> Self {
        Self::start_with(ServerOptions {
            diagnostics_delay: Duration::ZERO,
            ..ServerOptions::default()
        })
    }

    /// Starts a server with `options`, on the current tokio runtime.
    pub fn start_with(options: ServerOptions) -> Self {
        let (client, server) = tokio::io::duplex(1 << 16);
        let (server_read, server_write) = tokio::io::split(server);
        let server = tokio::spawn(mrq_lsp::serve(server_read, server_write, options));
        let (client_read, client_write) = tokio::io::split(client);
        let writer: Writer = Arc::new(tokio::sync::Mutex::new(client_write));
        let pending: Pending = Arc::default();
        let server_requests = Arc::new(Mutex::new(Vec::new()));
        let (notify_tx, notifications) = mpsc::unbounded_channel();
        let reader = tokio::spawn(read_loop(
            BufReader::new(client_read),
            Arc::clone(&writer),
            Arc::clone(&pending),
            Arc::clone(&server_requests),
            notify_tx,
        ));
        Self {
            writer,
            next_id: 1,
            pending,
            notifications,
            backlog: VecDeque::new(),
            server_requests,
            versions: HashMap::new(),
            server,
            reader,
        }
    }

    /// `initialize` with [`initialize_params`] for `roots`, then `initialized`.
    pub async fn initialize(&mut self, roots: &[&Path]) -> InitializeResult {
        self.initialize_with(initialize_params(roots)).await
    }

    /// `initialize` with `params`, then `initialized`; returns once the server has
    /// handled `initialized` (indexed the roots, registered watchers).
    pub async fn initialize_with(&mut self, params: InitializeParams) -> InitializeResult {
        let result = self.request::<request::Initialize>(params).await;
        self.notify::<notification::Initialized>(InitializedParams {})
            .await;
        self.sync().await;
        result
    }

    /// Returns once the server has handled everything sent before: it handles messages
    /// in order, so the answer to a request comes after them.
    pub async fn sync(&mut self) {
        self.index().await;
    }

    /// Sends request `R` and returns its result; panics on an error response.
    pub async fn request<R: Request>(&mut self, params: R::Params) -> R::Result {
        let params = serde_json::to_value(params).unwrap();
        match self.request_raw(R::METHOD, params).await {
            Ok(result) => serde_json::from_value(result).unwrap(),
            Err(error) => panic!("{} failed: {error}", R::METHOD),
        }
    }

    /// Sends a request and returns its `result`, or its `error` object.
    pub async fn request_raw(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        send(&self.writer, &message).await;
        let mut response = tokio::time::timeout(TIMEOUT, rx)
            .await
            .unwrap_or_else(|_| panic!("no response to {method}"))
            .unwrap();
        match response.get_mut("error") {
            Some(error) => Err(error.take()),
            None => Ok(response.get_mut("result").map_or(Value::Null, Value::take)),
        }
    }

    /// Sends notification `N`.
    pub async fn notify<N: Notification>(&mut self, params: N::Params) {
        self.notify_raw(N::METHOD, serde_json::to_value(params).unwrap())
            .await;
    }

    /// Sends a notification.
    pub async fn notify_raw(&mut self, method: &str, params: Value) {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        send(&self.writer, &message).await;
    }

    /// Waits for the next notification `N` from the server; other notifications stay
    /// queued for later waits.
    pub async fn notification<N: Notification>(&mut self) -> N::Params {
        self.notification_matching::<N>(|_| true).await
    }

    /// Waits for the next notification `N` whose parameters satisfy `accept`; others
    /// stay queued.
    pub async fn notification_matching<N: Notification>(
        &mut self,
        accept: impl Fn(&N::Params) -> bool,
    ) -> N::Params {
        let parse = |message: &Value| -> Option<N::Params> {
            if message["method"] != N::METHOD {
                return None;
            }
            let params = serde_json::from_value(message["params"].clone()).ok()?;
            accept(&params).then_some(params)
        };
        if let Some(i) = self.backlog.iter().position(|m| parse(m).is_some()) {
            return parse(&self.backlog.remove(i).unwrap()).unwrap();
        }
        let wait = async {
            loop {
                let message = self.notifications.recv().await.expect("server is gone");
                if let Some(params) = parse(&message) {
                    return params;
                }
                self.backlog.push_back(message);
            }
        };
        tokio::time::timeout(TIMEOUT, wait)
            .await
            .unwrap_or_else(|_| panic!("no {} notification", N::METHOD))
    }

    /// Every notification received and not yet waited for, oldest first (does not wait).
    pub fn pending_notifications(&mut self) -> Vec<Value> {
        while let Ok(message) = self.notifications.try_recv() {
            self.backlog.push_back(message);
        }
        self.backlog.iter().cloned().collect()
    }

    /// Waits for the next `textDocument/publishDiagnostics` for `uri`.
    pub async fn diagnostics(&mut self, uri: &Uri) -> PublishDiagnosticsParams {
        self.notification_matching::<notification::PublishDiagnostics>(|p| &p.uri == uri)
            .await
    }

    /// Waits for a `textDocument/publishDiagnostics` for `uri` that satisfies `accept`,
    /// dropping the earlier ones for `uri`: publications may be coalesced, so a test
    /// waits for the state it expects rather than for a number of them.
    pub async fn diagnostics_until(
        &mut self,
        uri: &Uri,
        accept: impl Fn(&PublishDiagnosticsParams) -> bool,
    ) -> PublishDiagnosticsParams {
        let wait = async {
            loop {
                let params = self.diagnostics(uri).await;
                if accept(&params) {
                    return params;
                }
            }
        };
        tokio::time::timeout(TIMEOUT, wait)
            .await
            .unwrap_or_else(|_| panic!("no matching diagnostics for {}", uri.as_str()))
    }

    /// Waits for diagnostics for `uri` whose codes are `codes`, in order.
    pub async fn diagnostics_with_codes(
        &mut self,
        uri: &Uri,
        codes: &[&str],
    ) -> PublishDiagnosticsParams {
        self.diagnostics_until(uri, |params| diagnostic_codes(params) == codes)
            .await
    }

    /// The requests the server sent so far with `method`, as JSON-RPC messages.
    pub fn server_requests(&self, method: &str) -> Vec<Value> {
        let requests = self.server_requests.lock().unwrap();
        requests
            .iter()
            .filter(|r| r["method"] == method)
            .cloned()
            .collect()
    }

    /// `textDocument/didOpen` of `path` with `text` (version 1, language `markdown`).
    pub async fn open(&mut self, path: &Path, text: &str) -> Uri {
        let uri = uri(path);
        self.versions.insert(uri.clone(), 1);
        let item = TextDocumentItem::new(uri.clone(), "markdown".into(), 1, text.into());
        self.notify::<notification::DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: item,
        })
        .await;
        uri
    }

    /// `textDocument/didChange` with `changes`, as the next version of `uri`.
    pub async fn change(&mut self, uri: &Uri, changes: Vec<TextDocumentContentChangeEvent>) {
        let version = self.versions.entry(uri.clone()).or_insert(0);
        *version += 1;
        let text_document = VersionedTextDocumentIdentifier::new(uri.clone(), *version);
        self.notify::<notification::DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document,
            content_changes: changes,
        })
        .await;
    }

    /// `textDocument/didChange` replacing the whole text of `uri`.
    pub async fn change_full(&mut self, uri: &Uri, text: &str) {
        self.change(uri, vec![full_change(text)]).await;
    }

    /// `textDocument/didClose` of `uri`.
    pub async fn close(&mut self, uri: &Uri) {
        self.versions.remove(uri);
        self.notify::<notification::DidCloseTextDocument>(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier::new(uri.clone()),
        })
        .await;
    }

    /// `workspace/didChangeWatchedFiles` reporting `changes` (path, kind).
    pub async fn watched(&mut self, changes: &[(&Path, FileChangeType)]) {
        let changes = changes
            .iter()
            .map(|(path, kind)| FileEvent::new(uri(path), *kind))
            .collect();
        self.notify::<notification::DidChangeWatchedFiles>(DidChangeWatchedFilesParams { changes })
            .await;
    }

    /// `textDocument/definition` at (line, character) `at` of `uri`, as one location.
    pub async fn definition(&mut self, uri: &Uri, at: (u32, u32)) -> Option<Location> {
        let params = GotoDefinitionParams {
            text_document_position_params: position_params(uri, at),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };
        match self.request::<request::GotoDefinition>(params).await? {
            GotoDefinitionResponse::Scalar(location) => Some(location),
            other => panic!("not a single location: {other:?}"),
        }
    }

    /// `textDocument/references` at `at` of `uri`; empty for `null`.
    pub async fn references(
        &mut self,
        uri: &Uri,
        at: (u32, u32),
        include_declaration: bool,
    ) -> Vec<Location> {
        let params = ReferenceParams {
            text_document_position: position_params(uri, at),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: ReferenceContext {
                include_declaration,
            },
        };
        self.request::<request::References>(params)
            .await
            .unwrap_or_default()
    }

    /// `textDocument/hover` at `at` of `uri`: the Markdown contents and the range.
    pub async fn hover(&mut self, uri: &Uri, at: (u32, u32)) -> Option<(String, Option<Range>)> {
        let params = HoverParams {
            text_document_position_params: position_params(uri, at),
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        let hover = self.request::<request::HoverRequest>(params).await?;
        match hover.contents {
            HoverContents::Markup(markup) => {
                assert_eq!(markup.kind, MarkupKind::Markdown);
                Some((markup.value, hover.range))
            }
            other => panic!("not Markdown: {other:?}"),
        }
    }

    /// `textDocument/completion` at `at` of `uri`, as invoked (no trigger character);
    /// `None` for `null`.
    pub async fn completion(&mut self, uri: &Uri, at: (u32, u32)) -> Option<Vec<CompletionItem>> {
        let params = CompletionParams {
            text_document_position: position_params(uri, at),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        };
        match self.request::<request::Completion>(params).await? {
            CompletionResponse::Array(items) => Some(items),
            CompletionResponse::List(list) => Some(list.items),
        }
    }

    /// `textDocument/documentSymbol` of `uri`, as nested symbols.
    pub async fn document_symbols(&mut self, uri: &Uri) -> Vec<DocumentSymbol> {
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier::new(uri.clone()),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };
        match self.request::<request::DocumentSymbolRequest>(params).await {
            Some(DocumentSymbolResponse::Nested(symbols)) => symbols,
            None => Vec::new(),
            other => panic!("not nested: {other:?}"),
        }
    }

    /// `workspace/symbol` for `query`, as `WorkspaceSymbol`s.
    pub async fn workspace_symbols(&mut self, query: &str) -> Vec<WorkspaceSymbol> {
        let params = WorkspaceSymbolParams {
            query: query.into(),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };
        // Both response forms deserialize as `Flat`, so they are told apart here.
        let method = <request::WorkspaceSymbolRequest as Request>::METHOD;
        let params = serde_json::to_value(params).unwrap();
        let result = self.request_raw(method, params).await.unwrap();
        serde_json::from_value::<Option<Vec<WorkspaceSymbol>>>(result)
            .unwrap()
            .unwrap_or_default()
    }

    /// `textDocument/prepareRename` at `at` of `uri`.
    pub async fn prepare_rename(
        &mut self,
        uri: &Uri,
        at: (u32, u32),
    ) -> Option<PrepareRenameResponse> {
        self.request::<request::PrepareRenameRequest>(position_params(uri, at))
            .await
    }

    /// `textDocument/rename` at `at` of `uri` to `new_name`: the edit, or the error
    /// object.
    pub async fn rename(
        &mut self,
        uri: &Uri,
        at: (u32, u32),
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, Value> {
        let params = RenameParams {
            text_document_position: position_params(uri, at),
            new_name: new_name.into(),
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        let method = <request::Rename as Request>::METHOD;
        let result = self
            .request_raw(method, serde_json::to_value(params).unwrap())
            .await?;
        Ok(serde_json::from_value(result).unwrap())
    }

    /// `workspace/willRenameFiles` for `renames` (old path, new path).
    pub async fn will_rename_files(&mut self, renames: &[(&Path, &Path)]) -> Option<WorkspaceEdit> {
        self.request::<request::WillRenameFiles>(rename_files_params(renames))
            .await
    }

    /// `workspace/didRenameFiles` for `renames` (old path, new path).
    pub async fn did_rename_files(&mut self, renames: &[(&Path, &Path)]) {
        self.notify::<notification::DidRenameFiles>(rename_files_params(renames))
            .await;
    }

    /// `textDocument/formatting` of `uri` (tab size 4, spaces), the edits or `None` for
    /// `null`.
    pub async fn formatting(&mut self, uri: &Uri) -> Option<Vec<TextEdit>> {
        let params = DocumentFormattingParams {
            text_document: TextDocumentIdentifier::new(uri.clone()),
            options: FormattingOptions {
                tab_size: 4,
                insert_spaces: true,
                ..FormattingOptions::default()
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        self.request::<request::Formatting>(params).await
    }

    /// `textDocument/codeAction` for `range` of `uri`, with no diagnostics in the context
    /// and the kinds `only`; the code actions, empty for `null` (commands fail the test).
    pub async fn code_actions(
        &mut self,
        uri: &Uri,
        range: Range,
        only: Option<&[CodeActionKind]>,
    ) -> Vec<CodeAction> {
        let context = CodeActionContext {
            only: only.map(<[_]>::to_vec),
            ..CodeActionContext::default()
        };
        self.code_actions_in(uri, range, context).await
    }

    /// `textDocument/codeAction` for `range` of `uri` in `context`, as
    /// [`code_actions`](Self::code_actions).
    pub async fn code_actions_in(
        &mut self,
        uri: &Uri,
        range: Range,
        context: CodeActionContext,
    ) -> Vec<CodeAction> {
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier::new(uri.clone()),
            range,
            context,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };
        self.request::<request::CodeActionRequest>(params)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => action,
                CodeActionOrCommand::Command(command) => panic!("a command: {command:?}"),
            })
            .collect()
    }

    /// The server's view of the workspace ([`INDEX_METHOD`]).
    pub async fn index(&mut self) -> IndexReport {
        let report = self.request_raw(INDEX_METHOD, Value::Null).await.unwrap();
        serde_json::from_value(report).unwrap()
    }

    /// `shutdown`, then `exit`; waits for the server to stop and returns whether it
    /// reported a clean shutdown.
    pub async fn shutdown(mut self) -> bool {
        self.request::<request::Shutdown>(()).await;
        self.exit().await
    }

    /// `exit` (with or without a `shutdown` before); waits for the server to stop and
    /// returns whether it reported a clean shutdown.
    pub async fn exit(mut self) -> bool {
        self.notify::<notification::Exit>(()).await;
        let clean = tokio::time::timeout(TIMEOUT, &mut self.server)
            .await
            .expect("server did not stop after exit")
            .unwrap();
        self.reader.abort();
        clean
    }
}

/// `initialize` parameters for a client with `roots` as workspace folders that supports
/// dynamic registration of file watchers, and UTF-16 positions only (the LSP default).
pub fn initialize_params(roots: &[&Path]) -> InitializeParams {
    let folders = roots
        .iter()
        .map(|root| WorkspaceFolder {
            uri: uri(root),
            name: root.file_name().unwrap().to_string_lossy().into_owned(),
        })
        .collect();
    InitializeParams {
        workspace_folders: Some(folders),
        capabilities: ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities {
                did_change_watched_files: Some(DidChangeWatchedFilesClientCapabilities {
                    dynamic_registration: Some(true),
                    relative_pattern_support: Some(true),
                }),
                workspace_folders: Some(true),
                ..WorkspaceClientCapabilities::default()
            }),
            ..ClientCapabilities::default()
        },
        ..InitializeParams::default()
    }
}

/// [`initialize_params`] for a client that also takes versioned `documentChanges` in
/// workspace edits.
pub fn initialize_params_with_document_changes(roots: &[&Path]) -> InitializeParams {
    let mut params = initialize_params(roots);
    if let Some(workspace) = params.capabilities.workspace.as_mut() {
        workspace.workspace_edit = Some(WorkspaceEditClientCapabilities {
            document_changes: Some(true),
            ..WorkspaceEditClientCapabilities::default()
        });
    }
    params
}

/// The parameters of `workspace/willRenameFiles` and `didRenameFiles` for `renames`.
fn rename_files_params(renames: &[(&Path, &Path)]) -> RenameFilesParams {
    let files = renames
        .iter()
        .map(|(old, new)| FileRename {
            old_uri: uri(old).as_str().to_owned(),
            new_uri: uri(new).as_str().to_owned(),
        })
        .collect();
    RenameFilesParams { files }
}

/// The text edits of `edit` per document, from `changes` or from `documentChanges`
/// (versions dropped), each document's sorted by position.
pub fn text_edits(edit: &WorkspaceEdit) -> BTreeMap<String, Vec<TextEdit>> {
    let mut found: BTreeMap<String, Vec<TextEdit>> = BTreeMap::new();
    for (uri, edits) in edit.changes.iter().flatten() {
        found
            .entry(uri.as_str().to_owned())
            .or_default()
            .extend(edits.iter().cloned());
    }
    if let Some(DocumentChanges::Edits(changes)) = &edit.document_changes {
        for change in changes {
            let edits = change.edits.iter().map(|edit| match edit {
                OneOf::Left(edit) => edit.clone(),
                OneOf::Right(annotated) => annotated.text_edit.clone(),
            });
            found
                .entry(change.text_document.uri.as_str().to_owned())
                .or_default()
                .extend(edits);
        }
    }
    for edits in found.values_mut() {
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    }
    found
}

/// `text` with the LSP `edits` (UTF-16 positions) applied.
pub fn apply(text: &str, edits: &[TextEdit]) -> String {
    let source = SourceFile::new(None, text);
    let edits: Vec<SourceEdit> = edits
        .iter()
        .map(|edit| SourceEdit {
            range: mrq_lsp::convert::byte_range(&source, edit.range, PositionEncoding::Utf16),
            replacement: edit.new_text.clone(),
        })
        .collect();
    apply_edits(text, &edits).unwrap()
}

/// What `edit` makes of the files of `fixture`: each changed file's relative path and new
/// text, read from disk.
pub fn edited(fixture: &Fixture, edit: &WorkspaceEdit) -> BTreeMap<String, String> {
    text_edits(edit)
        .into_iter()
        .map(|(uri, edits)| {
            let uri: Uri = uri.parse().unwrap();
            let path = uri.to_file_path().unwrap().into_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            let relative = path
                .strip_prefix(fixture.root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (relative, apply(&text, &edits))
        })
        .collect()
}

/// The `file:` URI of the absolute `path`.
pub fn uri(path: &Path) -> Uri {
    Uri::from_file_path(path).unwrap_or_else(|| panic!("no URI for {}", path.display()))
}

/// The range from (line, character) `start` to `end`.
pub fn range(start: (u32, u32), end: (u32, u32)) -> Range {
    Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1))
}

/// The location of `start`..`end` in `uri`.
pub fn location(uri: &Uri, start: (u32, u32), end: (u32, u32)) -> Location {
    Location::new(uri.clone(), range(start, end))
}

/// The position `at` in `uri`, as request parameters.
fn position_params(uri: &Uri, at: (u32, u32)) -> TextDocumentPositionParams {
    TextDocumentPositionParams::new(
        TextDocumentIdentifier::new(uri.clone()),
        Position::new(at.0, at.1),
    )
}

/// The codes of published diagnostics, in order.
pub fn diagnostic_codes(params: &PublishDiagnosticsParams) -> Vec<String> {
    params
        .diagnostics
        .iter()
        .map(|diagnostic| match &diagnostic.code {
            Some(NumberOrString::String(code)) => code.clone(),
            Some(NumberOrString::Number(code)) => code.to_string(),
            None => String::new(),
        })
        .collect()
}

/// A change replacing the whole text.
pub fn full_change(text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: text.into(),
    }
}

/// A change replacing the text from (line, character) `start` to `end`, in the
/// negotiated position encoding (UTF-16 unless the test asked for another).
pub fn range_change(
    start: (u32, u32),
    end: (u32, u32),
    text: &str,
) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: Some(Range::new(
            Position::new(start.0, start.1),
            Position::new(end.0, end.1),
        )),
        range_length: None,
        text: text.into(),
    }
}

/// A temporary directory of files, used as a workspace folder.
pub struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    /// A directory holding `files` (relative path, contents); parent directories are
    /// created as needed.
    pub fn new(files: &[(&str, &str)]) -> Self {
        let fixture = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for (path, text) in files {
            fixture.write(path, text);
        }
        fixture
    }

    /// The absolute path of the directory.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The absolute path of `relative` in the directory.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    /// The `file:` URI of `relative` in the directory.
    pub fn uri(&self, relative: &str) -> Uri {
        uri(&self.path(relative))
    }

    /// Writes `text` to `relative`, creating parent directories; returns its path.
    pub fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Deletes `relative`; returns its path.
    pub fn remove(&self, relative: &str) -> PathBuf {
        let path = self.path(relative);
        std::fs::remove_file(&path).unwrap();
        path
    }
}

/// Writes `message` with its `Content-Length` header.
async fn send(writer: &Writer, message: &Value) {
    let body = serde_json::to_vec(message).unwrap();
    let mut writer = writer.lock().await;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await.unwrap();
    writer.write_all(&body).await.unwrap();
    writer.flush().await.unwrap();
}

/// Reads one framed message; `None` at the end of the stream.
async fn receive(reader: &mut BufReader<tokio::io::ReadHalf<DuplexStream>>) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("message without Content-Length")];
    reader.read_exact(&mut body).await.ok()?;
    Some(serde_json::from_slice(&body).unwrap())
}

/// Dispatches the server's messages: responses to the waiting request, requests are
/// recorded and answered with `null`, notifications go to `notifications`.
async fn read_loop(
    mut reader: BufReader<tokio::io::ReadHalf<DuplexStream>>,
    writer: Writer,
    pending: Pending,
    server_requests: Arc<Mutex<Vec<Value>>>,
    notifications: mpsc::UnboundedSender<Value>,
) {
    while let Some(message) = receive(&mut reader).await {
        match (message.get("id"), message.get("method")) {
            (Some(id), Some(_)) => {
                let response = json!({ "jsonrpc": "2.0", "id": id, "result": null });
                server_requests.lock().unwrap().push(message.clone());
                send(&writer, &response).await;
            }
            (Some(id), None) => {
                let sender = id
                    .as_i64()
                    .and_then(|id| pending.lock().unwrap().remove(&id));
                if let Some(sender) = sender {
                    let _ = sender.send(message);
                }
            }
            (None, _) => {
                let _ = notifications.send(message);
            }
        }
    }
}
