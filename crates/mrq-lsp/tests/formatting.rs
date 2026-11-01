//! Formatting: `textDocument/formatting` with `mrq fmt`'s printer and the `fmt` settings
//! of the document's root folder, as edits of the lines that change.

mod support;

use mrq_lsp::ls_types::{OneOf, TextEdit, Uri};
use support::{Fixture, TestClient, apply, range};

const UNFORMATTED: &str = "# Title\n\nSome text.\n\n* one\n* two\n\nMore text.\n";
const FORMATTED: &str = "# Title\n\nSome text.\n\n- one\n- two\n\nMore text.\n";

#[tokio::test]
async fn initialize_advertises_formatting() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let capabilities = client.initialize(&[fixture.root()]).await.capabilities;
    assert_eq!(
        capabilities.document_formatting_provider,
        Some(OneOf::Left(true))
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn formatting_edits_only_the_lines_that_change() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), UNFORMATTED).await;
    let edits = client.formatting(&uri).await.expect("edits");
    assert_eq!(
        edits,
        [TextEdit::new(
            range((4, 0), (6, 0)),
            "- one\n- two\n".into()
        )]
    );
    assert_eq!(apply(UNFORMATTED, &edits), FORMATTED);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn formatting_follows_the_open_text() {
    let fixture = Fixture::new(&[("a.md", FORMATTED)]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), FORMATTED).await;
    assert_eq!(client.formatting(&uri).await, Some(Vec::new()));
    client.change_full(&uri, UNFORMATTED).await;
    let edits = client.formatting(&uri).await.expect("edits");
    assert_eq!(apply(UNFORMATTED, &edits), FORMATTED);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn formatting_uses_the_fmt_settings_of_the_root() {
    let fixture = Fixture::new(&[(".mrq.yaml", "fmt:\n  list_marker: \"*\"\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), FORMATTED).await;
    let edits = client.formatting(&uri).await.expect("edits");
    assert_eq!(apply(FORMATTED, &edits), UNFORMATTED);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn formatting_a_document_that_is_not_indexed_gives_nothing() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let untitled: Uri = "untitled:Untitled-1".parse().unwrap();
    assert_eq!(client.formatting(&untitled).await, None);
    assert!(client.shutdown().await);
}
