//! The server's lifecycle: `initialize` capabilities, position encodings, file watcher
//! registration and `shutdown`/`exit`.

mod support;

use mrq_lsp::ls_types::{
    ClientCapabilities, GeneralClientCapabilities, PositionEncodingKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
};
use serde_json::Value;
use support::{Fixture, TestClient, initialize_params};

#[tokio::test]
async fn initialize_advertises_incremental_sync() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let result = client.initialize(&[fixture.root()]).await;
    let info = result.server_info.unwrap();
    assert_eq!(info.name, "mrq");
    assert_eq!(info.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    let capabilities = result.capabilities;
    assert_eq!(
        capabilities.text_document_sync,
        Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                ..TextDocumentSyncOptions::default()
            }
        ))
    );
    assert_eq!(
        capabilities.position_encoding,
        Some(PositionEncodingKind::UTF16)
    );
    let folders = capabilities.workspace.unwrap().workspace_folders.unwrap();
    assert_eq!(folders.supported, Some(true));
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn utf8_positions_are_chosen_when_the_client_offers_them() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let mut params = initialize_params(&[fixture.root()]);
    params.capabilities.general = Some(GeneralClientCapabilities {
        position_encodings: Some(vec![
            PositionEncodingKind::UTF16,
            PositionEncodingKind::UTF8,
        ]),
        ..GeneralClientCapabilities::default()
    });
    let result = client.initialize_with(params).await;
    assert_eq!(
        result.capabilities.position_encoding,
        Some(PositionEncodingKind::UTF8)
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn file_watchers_are_registered_when_the_client_supports_it() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let requests = client.server_requests("client/registerCapability");
    assert_eq!(requests.len(), 1, "{requests:?}");
    let registration = &requests[0]["params"]["registrations"][0];
    assert_eq!(registration["method"], "workspace/didChangeWatchedFiles");
    let globs: Vec<&str> = registration["registerOptions"]["watchers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|watcher| watcher["globPattern"].as_str().unwrap())
        .collect();
    assert_eq!(
        globs,
        [
            "**/*.{md,markdown}",
            "**/.mrq.{yaml,yml}",
            "**/.markdownlint*"
        ]
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn no_file_watchers_without_dynamic_registration() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let mut params = initialize_params(&[fixture.root()]);
    params.capabilities = ClientCapabilities::default();
    client.initialize_with(params).await;
    let requests = client.server_requests("client/registerCapability");
    assert_eq!(requests, Vec::<serde_json::Value>::new());
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn exit_without_shutdown_is_not_clean() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    assert!(!client.exit().await);
}

#[tokio::test]
async fn requests_before_initialize_are_rejected() {
    let mut client = TestClient::start();
    let error = client
        .request_raw(mrq_lsp::INDEX_METHOD, Value::Null)
        .await
        .unwrap_err();
    // ServerNotInitialized.
    assert_eq!(error["code"], -32002);
}
