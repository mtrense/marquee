//! The document store and the workspace index: what the server knows about the files
//! under its root folders, kept current by `didOpen`/`didChange`/`didClose`, watched file
//! events, workspace folder changes and config reloads.

mod support;

use mrq_lsp::ls_types::notification::{DidChangeWorkspaceFolders, ShowMessage};
use mrq_lsp::ls_types::{
    DidChangeWorkspaceFoldersParams, FileChangeType, MessageType, WorkspaceFolder,
    WorkspaceFoldersChangeEvent,
};
use mrq_lsp::{DocumentReport, IndexReport};
use support::{Fixture, TestClient, full_change, range_change, uri};

/// The report of `relative` in `fixture`; panics if it is not indexed.
fn document<'r>(report: &'r IndexReport, fixture: &Fixture, relative: &str) -> &'r DocumentReport {
    let uri = fixture.uri(relative);
    report
        .documents
        .iter()
        .find(|doc| doc.uri == uri)
        .unwrap_or_else(|| panic!("{relative} is not indexed: {report:#?}"))
}

/// The indexed documents of `fixture`, as relative paths.
fn indexed(report: &IndexReport, fixture: &Fixture) -> Vec<String> {
    report
        .documents
        .iter()
        .map(|doc| {
            let path = doc.uri.to_file_path().unwrap();
            let relative = path.strip_prefix(fixture.root()).unwrap_or(&path);
            relative.to_string_lossy().replace('\\', "/")
        })
        .collect()
}

#[tokio::test]
async fn markdown_files_under_the_root_are_indexed() {
    let fixture = Fixture::new(&[
        (".mrq.yaml", "ignore: [drafts]\n"),
        (
            "a.md",
            "# Alpha\n\nSee [b](sub/b.md#beta) and [top](#alpha).\n",
        ),
        (
            "sub/b.md",
            "# Beta\n\n## Beta two\n\n<https://example.org>\n",
        ),
        ("sub/c.markdown", "text\n"),
        ("drafts/x.md", "# Draft\n"),
        ("notes.txt", "# Not Markdown\n"),
    ]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let report = client.index().await;

    assert_eq!(report.roots.len(), 1);
    assert_eq!(report.roots[0].uri, uri(fixture.root()));
    assert_eq!(report.roots[0].config_files, [fixture.path(".mrq.yaml")]);
    assert_eq!(
        indexed(&report, &fixture),
        ["a.md", "sub/b.md", "sub/c.markdown"]
    );
    let a = document(&report, &fixture, "a.md");
    assert_eq!(a.version, None);
    assert_eq!(a.flavor, "gfm");
    assert_eq!(a.anchors, ["alpha"]);
    assert_eq!(a.links, ["sub/b.md#beta", "#alpha"]);
    let b = document(&report, &fixture, "sub/b.md");
    assert_eq!(b.anchors, ["beta", "beta-two"]);
    assert_eq!(b.links, ["https://example.org"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn several_root_folders_are_indexed() {
    let one = Fixture::new(&[("a.md", "# A\n")]);
    let two = Fixture::new(&[("b.md", "# B\n"), (".mrq.yaml", "flavor: obsidian\n")]);
    let mut client = TestClient::start();
    client.initialize(&[one.root(), two.root()]).await;
    let report = client.index().await;
    assert_eq!(report.roots.len(), 2);
    assert_eq!(document(&report, &one, "a.md").flavor, "gfm");
    assert_eq!(document(&report, &two, "b.md").flavor, "obsidian");
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn incremental_changes_update_the_document() {
    let fixture = Fixture::new(&[("a.md", "# Old\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let text = "# Größe 😀 one\n\n## Two\n";
    let uri = client.open(&fixture.path("a.md"), text).await;
    let report = client.index().await;
    let a = document(&report, &fixture, "a.md");
    assert_eq!(a.version, Some(1));
    assert_eq!(a.anchors, ["größe--one", "two"]);

    // UTF-16 columns: the emoji is two code units, so "one" starts at 11. The second
    // change applies to the result of the first.
    client
        .change(
            &uri,
            vec![
                range_change((0, 11), (0, 14), "eins"),
                range_change((2, 3), (2, 6), "Zwei [x](#two)"),
            ],
        )
        .await;
    let report = client.index().await;
    let a = document(&report, &fixture, "a.md");
    assert_eq!(a.version, Some(2));
    assert_eq!(a.anchors, ["größe--eins", "zwei-x"]);
    assert_eq!(a.links, ["#two"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn utf8_changes_count_bytes() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let mut params = support::initialize_params(&[fixture.root()]);
    params.capabilities.general = Some(mrq_lsp::ls_types::GeneralClientCapabilities {
        position_encodings: Some(vec![mrq_lsp::ls_types::PositionEncodingKind::UTF8]),
        ..Default::default()
    });
    client.initialize_with(params).await;
    let uri = client.open(&fixture.path("a.md"), "# 😀 one\n").await;
    // The emoji is four bytes, so "one" starts at byte 7.
    client
        .change(&uri, vec![range_change((0, 7), (0, 10), "two")])
        .await;
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").anchors, ["-two"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn positions_past_the_end_are_clamped() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), "# A\n").await;
    // Column past the line's end, and a line past the text's end.
    client
        .change(&uri, vec![range_change((0, 99), (7, 3), "B\n")])
        .await;
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").anchors, ["ab"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn full_changes_replace_the_text() {
    let fixture = Fixture::new(&[("a.md", "# A\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), "# A\n").await;
    client
        .change(
            &uri,
            vec![range_change((0, 2), (0, 3), "X"), full_change("# B\n")],
        )
        .await;
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").anchors, ["b"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn closing_reverts_to_the_file_on_disk() {
    let fixture = Fixture::new(&[("a.md", "# Disk\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), "# Editor\n").await;
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").anchors, ["editor"]);
    client.close(&uri).await;
    let report = client.index().await;
    let a = document(&report, &fixture, "a.md");
    assert_eq!(a.anchors, ["disk"]);
    assert_eq!(a.version, None);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn unsaved_files_are_dropped_on_close() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("new.md"), "# New\n").await;
    assert_eq!(indexed(&client.index().await, &fixture), ["new.md"]);
    client.close(&uri).await;
    assert_eq!(
        indexed(&client.index().await, &fixture),
        Vec::<String>::new()
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn files_outside_the_roots_are_indexed_while_open() {
    let root = Fixture::new(&[("a.md", "# A\n")]);
    let elsewhere = Fixture::new(&[("x.md", "# X\n"), ("y.md", "# Y\n")]);
    let mut client = TestClient::start();
    client.initialize(&[root.root()]).await;
    let uri = client
        .open(&elsewhere.path("x.md"), "# X\n\n[y](y.md#y)\n")
        .await;
    let report = client.index().await;
    // Only the open file, not its directory.
    assert_eq!(report.documents.len(), 2);
    assert_eq!(document(&report, &elsewhere, "x.md").links, ["y.md#y"]);
    client.close(&uri).await;
    assert_eq!(client.index().await.documents.len(), 1);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn watched_files_are_reindexed() {
    let fixture = Fixture::new(&[("a.md", "# A\n"), ("b.md", "# B\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;

    let new = fixture.write("sub/new.md", "# New\n");
    let a = fixture.write("a.md", "# A changed\n");
    let b = fixture.remove("b.md");
    client
        .watched(&[
            (&new, FileChangeType::CREATED),
            (&a, FileChangeType::CHANGED),
            (&b, FileChangeType::DELETED),
        ])
        .await;
    let report = client.index().await;
    assert_eq!(indexed(&report, &fixture), ["a.md", "sub/new.md"]);
    assert_eq!(document(&report, &fixture, "a.md").anchors, ["a-changed"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn watched_changes_to_open_files_are_ignored() {
    let fixture = Fixture::new(&[("a.md", "# Disk\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    client.open(&fixture.path("a.md"), "# Editor\n").await;
    let a = fixture.write("a.md", "# Disk changed\n");
    client.watched(&[(&a, FileChangeType::CHANGED)]).await;
    let a = fixture.remove("a.md");
    client.watched(&[(&a, FileChangeType::DELETED)]).await;
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").anchors, ["editor"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn created_files_in_ignored_directories_are_not_indexed() {
    let fixture = Fixture::new(&[(".mrq.yaml", "ignore: [drafts]\n"), ("a.md", "# A\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let draft = fixture.write("drafts/x.md", "# X\n");
    let text = fixture.write("notes.txt", "# X\n");
    client
        .watched(&[
            (&draft, FileChangeType::CREATED),
            (&text, FileChangeType::CREATED),
        ])
        .await;
    assert_eq!(indexed(&client.index().await, &fixture), ["a.md"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn config_changes_are_reloaded() {
    let fixture = Fixture::new(&[("a.md", "# A\n\n[[b]]\n"), ("b.md", "# B\n"), ("c.md", "")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let report = client.index().await;
    assert_eq!(
        document(&report, &fixture, "a.md").links,
        Vec::<String>::new()
    );
    client.open(&fixture.path("b.md"), "# B\n\n[[a#A]]\n").await;

    let config = fixture.write(".mrq.yaml", "flavor: obsidian\nignore: [c.md]\n");
    client.watched(&[(&config, FileChangeType::CREATED)]).await;
    let report = client.index().await;
    assert_eq!(report.roots[0].config_files, std::slice::from_ref(&config));
    assert_eq!(indexed(&report, &fixture), ["a.md", "b.md"]);
    let a = document(&report, &fixture, "a.md");
    assert_eq!(a.flavor, "obsidian");
    assert_eq!(a.links, ["[[b]]"]);
    // Open documents are re-parsed from the editor's text.
    let b = document(&report, &fixture, "b.md");
    assert_eq!((b.flavor.as_str(), b.version), ("obsidian", Some(1)));
    assert_eq!(b.links, ["[[a#A]]"]);

    let config = fixture.remove(".mrq.yaml");
    client.watched(&[(&config, FileChangeType::DELETED)]).await;
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").flavor, "gfm");
    assert_eq!(indexed(&report, &fixture), ["a.md", "b.md", "c.md"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn invalid_config_is_reported_and_the_previous_one_kept() {
    let fixture = Fixture::new(&[(".mrq.yaml", "flavor: obsidian\n"), ("a.md", "# A\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let config = fixture.write(".mrq.yaml", "flavor: [nope\n");
    client.watched(&[(&config, FileChangeType::CHANGED)]).await;
    let message = client.notification::<ShowMessage>().await;
    assert_eq!(message.typ, MessageType::ERROR);
    assert!(message.message.contains(".mrq.yaml"), "{}", message.message);
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").flavor, "obsidian");
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn invalid_config_at_startup_falls_back_to_the_defaults() {
    let fixture = Fixture::new(&[(".mrq.yaml", "flavor: nope\n"), ("a.md", "# A\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let message = client.notification::<ShowMessage>().await;
    assert_eq!(message.typ, MessageType::ERROR);
    let report = client.index().await;
    assert_eq!(document(&report, &fixture, "a.md").flavor, "gfm");
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn workspace_folders_can_be_added_and_removed() {
    let one = Fixture::new(&[("a.md", "# A\n")]);
    let two = Fixture::new(&[("b.md", "# B\n")]);
    let mut client = TestClient::start();
    client.initialize(&[one.root()]).await;
    let folder = |fixture: &Fixture| WorkspaceFolder {
        uri: uri(fixture.root()),
        name: "folder".into(),
    };
    client
        .notify::<DidChangeWorkspaceFolders>(DidChangeWorkspaceFoldersParams {
            event: WorkspaceFoldersChangeEvent {
                added: vec![folder(&two)],
                removed: vec![folder(&one)],
            },
        })
        .await;
    let report = client.index().await;
    assert_eq!(report.roots.len(), 1);
    assert_eq!(report.roots[0].uri, uri(two.root()));
    assert_eq!(report.documents.len(), 1);
    document(&report, &two, "b.md");
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn root_uri_is_used_without_workspace_folders() {
    let fixture = Fixture::new(&[("a.md", "# A\n")]);
    let mut client = TestClient::start();
    let mut params = support::initialize_params(&[]);
    params.workspace_folders = None;
    #[allow(deprecated, reason = "older clients only send rootUri")]
    {
        params.root_uri = Some(uri(fixture.root()));
    }
    client.initialize_with(params).await;
    let report = client.index().await;
    assert_eq!(indexed(&report, &fixture), ["a.md"]);
    assert!(client.shutdown().await);
}
