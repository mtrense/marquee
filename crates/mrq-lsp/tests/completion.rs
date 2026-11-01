//! Completion: file paths, anchors, footnote and reference labels and wikilink targets,
//! over a fixture workspace, in the text being typed.

mod support;

use mrq_lsp::ls_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, Range};
use support::{Fixture, TestClient, range};

/// A gfm workspace: `a.md` with headings, a footnote and reference definitions, `b.md`,
/// a `docs` directory with a document and an image, and a hidden file.
const A: &str = "# Guide

## Usage

Text[^note] and [ref][r].

[r]: https://example.org
[Other]: b.md

[^note]: The note text.
";

fn workspace() -> Fixture {
    Fixture::new(&[
        ("a.md", A),
        ("b.md", "# B Title\n\n## Setup\n"),
        ("docs/c.md", "# C\n"),
        ("docs/img.png", "png"),
        (".hidden.md", "# Hidden\n"),
    ])
}

async fn client_for(fixture: &Fixture) -> TestClient {
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    client
}

/// Each item as (label, kind, detail).
fn summary(items: &[CompletionItem]) -> Vec<(&str, CompletionItemKind, Option<&str>)> {
    items
        .iter()
        .map(|item| {
            let kind = item.kind.expect("a kind");
            (item.label.as_str(), kind, item.detail.as_deref())
        })
        .collect()
}

/// Each item's text edit as (range, new text).
fn edits(items: &[CompletionItem]) -> Vec<(Range, &str)> {
    items
        .iter()
        .map(|item| match &item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (edit.range, edit.new_text.as_str()),
            other => panic!("not a plain text edit: {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn initialize_advertises_completion_with_trigger_characters() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let capabilities = client.initialize(&[fixture.root()]).await.capabilities;
    let completion = capabilities.completion_provider.expect("completion");
    assert_eq!(
        completion.trigger_characters,
        Some(["(", "#", "/", "^", "["].map(String::from).to_vec())
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn paths_complete_relative_to_the_document() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    // Markdown files (with their title), then directories, then other files; no hidden
    // files.
    let new = client.open(&fixture.path("new.md"), "[x](").await;
    let items = client.completion(&new, (0, 4)).await.unwrap();
    assert_eq!(
        summary(&items),
        [
            ("a.md", CompletionItemKind::FILE, Some("Guide")),
            ("b.md", CompletionItemKind::FILE, Some("B Title")),
            ("docs/", CompletionItemKind::FOLDER, None),
        ]
    );
    let empty = range((0, 4), (0, 4));
    assert_eq!(
        edits(&items),
        [(empty, "a.md"), (empty, "b.md"), (empty, "docs/")]
    );
    // In a directory, replacing what is typed of the name.
    client.change_full(&new, "![i](docs/i").await;
    let items = client.completion(&new, (0, 11)).await.unwrap();
    assert_eq!(
        summary(&items),
        [
            ("c.md", CompletionItemKind::FILE, Some("C")),
            ("img.png", CompletionItemKind::FILE, None),
        ]
    );
    let typed = range((0, 10), (0, 11));
    assert_eq!(edits(&items), [(typed, "c.md"), (typed, "img.png")]);
    // Up from a subdirectory.
    let sub = client.open(&fixture.path("docs/new.md"), "[x](../b").await;
    let items = client.completion(&sub, (0, 8)).await.unwrap();
    let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(labels, ["a.md", "b.md", "docs/"]);
    assert_eq!(edits(&items)[0], (range((0, 7), (0, 8)), "a.md"));
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn anchors_complete_for_the_document_and_other_files() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    // The document's own headings, in the text being typed (two UTF-16 units for the
    // emoji).
    let text = format!("{A}\n😀 [x](#");
    let a = client.open(&fixture.path("a.md"), &text).await;
    let items = client.completion(&a, (11, 8)).await.unwrap();
    assert_eq!(
        summary(&items),
        [
            ("guide", CompletionItemKind::REFERENCE, Some("Guide")),
            ("usage", CompletionItemKind::REFERENCE, Some("Usage")),
        ]
    );
    let empty = range((11, 8), (11, 8));
    assert_eq!(edits(&items), [(empty, "guide"), (empty, "usage")]);
    // Another file's, replacing the typed part of the anchor.
    client.change_full(&a, "[x](b.md#se").await;
    let items = client.completion(&a, (0, 11)).await.unwrap();
    let typed = range((0, 9), (0, 11));
    assert_eq!(edits(&items), [(typed, "b-title"), (typed, "setup")]);
    assert_eq!(items[1].detail.as_deref(), Some("Setup"));
    // A file in a directory; nothing for a missing one.
    client.change_full(&a, "[x](docs/c.md#").await;
    let items = client.completion(&a, (0, 14)).await.unwrap();
    assert_eq!(edits(&items), [(range((0, 14), (0, 14)), "c")]);
    client.change_full(&a, "[x](nope.md#").await;
    assert_eq!(client.completion(&a, (0, 12)).await.unwrap_or_default(), []);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn anchors_include_heading_attributes() {
    let fixture = Fixture::new(&[
        (".mrq.yaml", "flavor: mkdocs\n"),
        ("a.md", "# Intro\n\n## Install {#setup}\n"),
    ]);
    let mut client = client_for(&fixture).await;
    let new = client.open(&fixture.path("new.md"), "[x](a.md#").await;
    let items = client.completion(&new, (0, 9)).await.unwrap();
    assert_eq!(
        summary(&items),
        [
            ("intro", CompletionItemKind::REFERENCE, Some("Intro")),
            ("setup", CompletionItemKind::REFERENCE, Some("Install")),
        ]
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn footnote_and_reference_labels_complete() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let text = format!("{A}\nMore[^no");
    let a = client.open(&fixture.path("a.md"), &text).await;
    let items = client.completion(&a, (11, 8)).await.unwrap();
    assert_eq!(
        summary(&items),
        [(
            "note",
            CompletionItemKind::REFERENCE,
            Some("The note text.")
        )]
    );
    assert_eq!(edits(&items), [(range((11, 6), (11, 8)), "note")]);
    // Reference links, by label, with the destination.
    let text = format!("{A}\nA [link][");
    client.change_full(&a, &text).await;
    let items = client.completion(&a, (11, 9)).await.unwrap();
    assert_eq!(
        summary(&items),
        [
            (
                "r",
                CompletionItemKind::REFERENCE,
                Some("https://example.org")
            ),
            ("Other", CompletionItemKind::REFERENCE, Some("b.md")),
        ]
    );
    let empty = range((11, 9), (11, 9));
    assert_eq!(edits(&items), [(empty, "r"), (empty, "Other")]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn wikilinks_complete_names_and_headings() {
    let fixture = workspace();
    fixture.write("docs/b.md", "# Other B\n");
    let mut client = client_for(&fixture).await;
    // Every other document by its shortest name, the way the vault finds it.
    let new = client.open(&fixture.path("new.md"), "[[").await;
    let items = client.completion(&new, (0, 2)).await.unwrap();
    assert_eq!(
        summary(&items),
        [
            ("a", CompletionItemKind::FILE, Some("a.md")),
            ("b", CompletionItemKind::FILE, Some("b.md")),
            ("docs/b", CompletionItemKind::FILE, Some("docs/b.md")),
            ("c", CompletionItemKind::FILE, Some("docs/c.md")),
        ]
    );
    assert_eq!(edits(&items)[0], (range((0, 2), (0, 2)), "a"));
    // A note's headings, by text.
    client.change_full(&new, "See [[b#Se").await;
    let items = client.completion(&new, (0, 10)).await.unwrap();
    let typed = range((0, 8), (0, 10));
    assert_eq!(edits(&items), [(typed, "B Title"), (typed, "Setup")]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn nothing_completes_outside_links() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let new = client
        .open(
            &fixture.path("new.md"),
            "plain [x](a.md) [w](https://ex `[x](`\n",
        )
        .await;
    for at in [(0, 3), (0, 15), (0, 30), (0, 36)] {
        assert_eq!(client.completion(&new, at).await, None, "at {at:?}");
    }
    assert!(client.shutdown().await);
}
