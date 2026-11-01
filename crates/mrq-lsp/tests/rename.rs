//! Rename: headings, with every link to their anchors, and files and directories
//! (`workspace/willRenameFiles`), with every link to them, over fixture workspaces of
//! several files.

mod support;

use std::collections::BTreeMap;

use mrq_lsp::ls_types::{
    DocumentChanges, FileOperationPatternKind, OneOf, PrepareRenameResponse, RenameOptions,
    WorkDoneProgressOptions,
};
use support::{
    Fixture, TestClient, edited, initialize_params_with_document_changes, range, text_edits,
};

/// A gfm workspace: `a.md` links to the first `Setup` of `b.md` in every way a link can
/// (with `./` and a query, from the root, by reference) and to its own heading; `b.md`
/// links to both of its `Setup`s (`setup` and `setup-1`) and back.
const A: &str = "# Guide

See [setup](b.md#setup), [again](./b.md?x=1#setup) and [top](b.md).
A [ref][r], [root](/b.md#setup) and [own](#usage).

## Usage

[r]: b.md#setup
";

const B: &str = "# B

## Setup

Text [self](#setup) and [dup](#setup-1).

## Setup

See [back](a.md#usage).
";

fn workspace() -> Fixture {
    Fixture::new(&[("a.md", A), ("b.md", B)])
}

async fn client_for(fixture: &Fixture) -> TestClient {
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    client
}

/// `(path, text)` pairs as the map [`edited`] returns.
fn texts(files: &[(&str, &str)]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect()
}

#[tokio::test]
async fn initialize_advertises_rename() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let capabilities = client.initialize(&[fixture.root()]).await.capabilities;
    assert_eq!(
        capabilities.rename_provider,
        Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        }))
    );
    let operations = capabilities.workspace.unwrap().file_operations.unwrap();
    let will = operations.will_rename.unwrap();
    assert_eq!(operations.did_rename, Some(will.clone()));
    let kinds: Vec<_> = will
        .filters
        .iter()
        .map(|filter| {
            assert_eq!(filter.scheme.as_deref(), Some("file"));
            (filter.pattern.glob.as_str(), filter.pattern.matches.clone())
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("**/*", Some(FileOperationPatternKind::File)),
            ("**/*", Some(FileOperationPatternKind::Folder)),
        ]
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn prepare_rename_offers_the_heading_text() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let b = fixture.uri("b.md");
    let expected = Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: range((2, 3), (2, 8)),
        placeholder: "Setup".into(),
    });
    // Anywhere on the heading line, the `##` included.
    assert_eq!(client.prepare_rename(&b, (2, 4)).await, expected);
    assert_eq!(client.prepare_rename(&b, (2, 0)).await, expected);
    // Not on text, a link or past the end.
    assert_eq!(client.prepare_rename(&b, (4, 1)).await, None);
    assert_eq!(client.prepare_rename(&b, (4, 7)).await, None);
    assert_eq!(client.prepare_rename(&b, (20, 0)).await, None);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn renaming_a_heading_updates_the_links_to_it() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let edit = client
        .rename(&fixture.uri("b.md"), (2, 4), "Install guide")
        .await
        .unwrap()
        .expect("an edit");
    // The second `Setup` becomes `setup`, so the link to `setup-1` follows it.
    let a = "# Guide

See [setup](b.md#install-guide), [again](./b.md?x=1#install-guide) and [top](b.md).
A [ref][r], [root](/b.md#install-guide) and [own](#usage).

## Usage

[r]: b.md#install-guide
";
    let b = "# B

## Install guide

Text [self](#install-guide) and [dup](#setup).

## Setup

See [back](a.md#usage).
";
    assert_eq!(edited(&fixture, &edit), texts(&[("a.md", a), ("b.md", b)]));
    // Renaming the second `Setup` leaves the first one's links alone.
    let edit = client
        .rename(&fixture.uri("b.md"), (6, 4), "Other")
        .await
        .unwrap()
        .expect("an edit");
    let b = B
        .replace("(#setup-1)", "(#other)")
        .replace("\n## Setup\n\nSee", "\n## Other\n\nSee");
    assert_eq!(edited(&fixture, &edit), texts(&[("b.md", &b)]));
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn renaming_rejects_other_places_and_bad_names() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let b = fixture.uri("b.md");
    assert_eq!(client.rename(&b, (4, 1), "X").await, Ok(None));
    for name in ["", "  ", "Two\nlines"] {
        let error = client.rename(&b, (2, 4), name).await.unwrap_err();
        assert!(
            error["message"].as_str().unwrap().contains("heading"),
            "{error}"
        );
    }
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn renaming_a_heading_with_an_id_keeps_its_links() {
    let fixture = Fixture::new(&[
        (".mrq.yaml", "flavor: mkdocs\n"),
        ("a.md", "# A\n\n## Install {#setup}\n\n[x](#setup)\n"),
        ("b.md", "[a](a.md#setup)\n"),
    ]);
    let mut client = client_for(&fixture).await;
    let a = fixture.uri("a.md");
    assert_eq!(
        client.prepare_rename(&a, (2, 4)).await,
        Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: range((2, 3), (2, 10)),
            placeholder: "Install".into(),
        })
    );
    let edit = client.rename(&a, (2, 4), "Set up").await.unwrap().unwrap();
    assert_eq!(
        edited(&fixture, &edit),
        texts(&[("a.md", "# A\n\n## Set up {#setup}\n\n[x](#setup)\n")])
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn renaming_a_heading_updates_wikilinks_by_text() {
    let fixture = Fixture::new(&[
        (".mrq.yaml", "flavor: obsidian\n"),
        (
            "a.md",
            "See [[b#Setup]], [[b#Setup|the setup]] and [[b]].\n",
        ),
        ("b.md", "# B\n\n## Setup\n\nSee [[#Setup]].\n"),
    ]);
    let mut client = client_for(&fixture).await;
    let edit = client
        .rename(&fixture.uri("b.md"), (2, 4), "Install guide")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        edited(&fixture, &edit),
        texts(&[
            (
                "a.md",
                "See [[b#Install guide]], [[b#Install guide|the setup]] and [[b]].\n"
            ),
            (
                "b.md",
                "# B\n\n## Install guide\n\nSee [[#Install guide]].\n"
            ),
        ])
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn rename_edits_are_versioned_when_the_client_supports_it() {
    let fixture = workspace();
    let mut client = TestClient::start();
    let params = initialize_params_with_document_changes(&[fixture.root()]);
    client.initialize_with(params).await;
    let b = client.open(&fixture.path("b.md"), B).await;
    client.change_full(&b, B).await;
    let edit = client.rename(&b, (2, 4), "Install").await.unwrap().unwrap();
    assert_eq!(edit.changes, None);
    let Some(DocumentChanges::Edits(changes)) = &edit.document_changes else {
        panic!("no document edits: {edit:?}");
    };
    let versions: Vec<_> = changes
        .iter()
        .map(|change| {
            (
                change.text_document.uri.clone(),
                change.text_document.version,
            )
        })
        .collect();
    assert_eq!(versions, [(fixture.uri("a.md"), None), (b, Some(2))]);
    assert_eq!(text_edits(&edit).len(), 2);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn rename_edits_use_the_open_text() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let a = client
        .open(&fixture.path("a.md"), "Moved: [s](b.md#setup)\n")
        .await;
    let edit = client
        .rename(&fixture.uri("b.md"), (2, 4), "Install")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edit.document_changes, None);
    let edits = text_edits(&edit);
    let a_edits = &edits[a.as_str()];
    assert_eq!(a_edits.len(), 1);
    assert_eq!(a_edits[0].range, range((0, 16), (0, 21)));
    assert_eq!(a_edits[0].new_text, "install");
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn renaming_a_file_rewrites_the_links_to_it_and_in_it() {
    let fixture = Fixture::new(&[
        (
            "a.md",
            "# A\n\n[b](b.md#setup), [q](./b.md?x=1), [root](/b.md) and [img](img/logo.png).\n\n[r]: b.md\n",
        ),
        (
            "b.md",
            "# B\n\n## Setup\n\n[a](a.md#a), [self](#setup), [me](b.md#setup) and [logo](img/logo.png).\n",
        ),
        ("c.md", "# C\n"),
        ("img/logo.png", "png"),
    ]);
    let mut client = client_for(&fixture).await;
    let (old, new) = (fixture.path("b.md"), fixture.path("sub/c.md"));
    let edit = client
        .will_rename_files(&[(&old, &new)])
        .await
        .expect("an edit");
    assert_eq!(
        edited(&fixture, &edit),
        texts(&[
            (
                "a.md",
                "# A\n\n[b](sub/c.md#setup), [q](./sub/c.md?x=1), [root](/sub/c.md) and [img](img/logo.png).\n\n[r]: sub/c.md\n",
            ),
            (
                "b.md",
                "# B\n\n## Setup\n\n[a](../a.md#a), [self](#setup), [me](c.md#setup) and [logo](../img/logo.png).\n",
            ),
        ])
    );
    // Nothing links to `c.md`, nor does it link anywhere.
    let (old, new) = (fixture.path("c.md"), fixture.path("z.md"));
    assert_eq!(client.will_rename_files(&[(&old, &new)]).await, None);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn renaming_a_directory_rewrites_the_links_into_it() {
    let fixture = Fixture::new(&[
        (
            "index.md",
            "[x](docs/x.md#x), [logo](docs/img/logo.png), [dir](docs/) and [y](docs/y.md).\n",
        ),
        (
            "docs/x.md",
            "# X\n\n[y](./y.md), [up](../index.md), [logo](img/logo.png) and [me](x.md#x).\n",
        ),
        ("docs/y.md", "# Y\n"),
        ("docs/img/logo.png", "png"),
    ]);
    let mut client = client_for(&fixture).await;
    let (old, new) = (fixture.path("docs"), fixture.path("nested/manual"));
    let edit = client
        .will_rename_files(&[(&old, &new)])
        .await
        .expect("an edit");
    assert_eq!(
        edited(&fixture, &edit),
        texts(&[
            (
                "docs/x.md",
                "# X\n\n[y](./y.md), [up](../../index.md), [logo](img/logo.png) and [me](x.md#x).\n",
            ),
            (
                "index.md",
                "[x](nested/manual/x.md#x), [logo](nested/manual/img/logo.png), [dir](nested/manual/) and [y](nested/manual/y.md).\n",
            ),
        ])
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn renaming_a_file_rewrites_wikilinks_to_it() {
    let fixture = Fixture::new(&[
        (".mrq.yaml", "flavor: obsidian\n"),
        (
            "a.md",
            "See [[b]], [[b#Setup|setup]], [[notes/b]] and ![[pic.png]].\n",
        ),
        ("notes/b.md", "# B\n\n## Setup\n"),
        ("pic.png", "png"),
    ]);
    let mut client = client_for(&fixture).await;
    let renames = [
        (fixture.path("notes/b.md"), fixture.path("notes/c.md")),
        (fixture.path("pic.png"), fixture.path("img/photo.png")),
    ];
    let renames: Vec<_> = renames
        .iter()
        .map(|(o, n)| (o.as_path(), n.as_path()))
        .collect();
    let edit = client.will_rename_files(&renames).await.expect("an edit");
    assert_eq!(
        edited(&fixture, &edit),
        texts(&[(
            "a.md",
            "See [[c]], [[c#Setup|setup]], [[notes/c]] and ![[photo.png]].\n"
        )])
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn did_rename_files_updates_the_index() {
    let fixture = Fixture::new(&[("a.md", "[b](b.md)\n"), ("b.md", "# B\n")]);
    let mut client = client_for(&fixture).await;
    let (old, new) = (fixture.path("b.md"), fixture.path("sub/c.md"));
    std::fs::create_dir_all(new.parent().unwrap()).unwrap();
    std::fs::rename(&old, &new).unwrap();
    client.did_rename_files(&[(&old, &new)]).await;
    let uris: Vec<_> = client
        .index()
        .await
        .documents
        .into_iter()
        .map(|document| document.uri)
        .collect();
    assert_eq!(uris, [fixture.uri("a.md"), fixture.uri("sub/c.md")]);
    assert!(client.shutdown().await);
}
