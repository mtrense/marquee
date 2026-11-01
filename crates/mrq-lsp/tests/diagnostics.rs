//! Diagnostics: `validate`'s internal checks and `lint`, published for open documents
//! (debounced), re-evaluated when a document they link to changes, and cleared on close.

mod support;

use std::time::Duration;

use mrq_lsp::ServerOptions;
use mrq_lsp::ls_types::notification::PublishDiagnostics;
use mrq_lsp::ls_types::{
    DiagnosticSeverity, DiagnosticTag, FileChangeType, NumberOrString, Position, Range,
};
use support::{Fixture, TestClient, diagnostic_codes, range_change};

const LINKS_AND_SPACES: &str = "# A\n\n[x](#nope) end \n";

fn range(start: (u32, u32), end: (u32, u32)) -> Range {
    Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1))
}

#[tokio::test]
async fn an_opened_document_gets_validate_and_lint_diagnostics() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), LINKS_AND_SPACES).await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(
        diagnostic_codes(&published),
        ["missing-anchor", "MD051", "MD009"]
    );

    let anchor = &published.diagnostics[0];
    assert_eq!(anchor.range, range((2, 0), (2, 10)));
    assert_eq!(anchor.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(anchor.source.as_deref(), Some("mrq"));
    assert_eq!(anchor.message, "no anchor `#nope` in this document");
    assert_eq!(anchor.code_description, None);

    let spaces = &published.diagnostics[2];
    assert_eq!(spaces.range, range((2, 14), (2, 15)));
    assert_eq!(spaces.severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(
        spaces.code_description.as_ref().unwrap().href.as_str(),
        "https://github.com/DavidAnson/markdownlint/blob/main/doc/md009.md"
    );
    // The fix, for code actions, in LSP terms.
    let data = spaces.data.as_ref().unwrap();
    assert_eq!(data["fix"]["title"], "Delete the trailing spaces", "{data}");
    assert_eq!(data["fix"]["edits"][0]["newText"], "");
    assert_eq!(
        data["fix"]["edits"][0]["range"],
        serde_json::to_value(range((2, 14), (2, 15))).unwrap()
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn diagnostics_follow_changes() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), LINKS_AND_SPACES).await;
    client
        .diagnostics_with_codes(&uri, &["missing-anchor", "MD051", "MD009"])
        .await;
    // `#nope` → `#a`, and the trailing space dropped.
    client
        .change(
            &uri,
            vec![
                range_change((2, 14), (2, 15), ""),
                range_change((2, 5), (2, 9), "a"),
            ],
        )
        .await;
    let published = client.diagnostics_with_codes(&uri, &[]).await;
    assert_eq!(published.version, Some(2));
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn help_goes_into_the_message_and_labels_into_related_information() {
    let fixture = Fixture::new(&[(".mrq.yaml", "flavor: mkdocs\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client
        .open(
            &fixture.path("a.md"),
            "# One {#x}\n\n# Two {#x}\n\n[b](b.md)\n",
        )
        .await;
    let published = client.diagnostics(&uri).await;
    let duplicate = published
        .diagnostics
        .iter()
        .find(|d| d.code == Some(NumberOrString::String("duplicate-anchor".into())))
        .unwrap();
    assert_eq!(duplicate.range, range((2, 0), (2, 10)));
    let related = duplicate.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].location.uri, uri);
    assert_eq!(related[0].location.range, range((0, 0), (0, 10)));
    assert_eq!(related[0].message, "first used here");

    let missing = published
        .diagnostics
        .iter()
        .find(|d| d.code == Some(NumberOrString::String("missing-file".into())))
        .unwrap();
    let resolved = fixture.path("b.md");
    assert_eq!(
        missing.message,
        format!(
            "file `b.md` does not exist\n\nhelp: resolved to `{}`",
            resolved.display()
        )
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn unused_definitions_are_tagged_unnecessary() {
    let fixture = Fixture::new(&[(".mrq.yaml", "lint:\n  extends: none\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client
        .open(&fixture.path("a.md"), "# A\n\n[^1]: Unused.\n")
        .await;
    let published = client
        .diagnostics_with_codes(&uri, &["unused-footnote"])
        .await;
    assert_eq!(
        published.diagnostics[0].tags,
        Some(vec![DiagnosticTag::UNNECESSARY])
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn parse_problems_are_reported() {
    let fixture = Fixture::new(&[(".mrq.yaml", "lint:\n  extends: none\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client
        .open(&fixture.path("a.md"), "---\ntitle: [\n---\n# A\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.diagnostics.len(), 1, "{published:?}");
    assert_eq!(
        published.diagnostics[0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn the_lint_configuration_and_inline_comments_apply() {
    let fixture = Fixture::new(&[(".mrq.yaml", "lint:\n  rules:\n    MD051: off\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), LINKS_AND_SPACES).await;
    client
        .diagnostics_with_codes(&uri, &["missing-anchor", "MD009"])
        .await;
    let text = format!("<!-- markdownlint-disable MD009 -->\n{LINKS_AND_SPACES}");
    client.change_full(&uri, &text).await;
    client
        .diagnostics_with_codes(&uri, &["missing-anchor"])
        .await;
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_markdownlint_file_configures_lint() {
    let fixture = Fixture::new(&[(".markdownlint.json", r#"{ "MD009": false }"#)]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), LINKS_AND_SPACES).await;
    client
        .diagnostics_with_codes(&uri, &["missing-anchor", "MD051"])
        .await;
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_configuration_change_republishes() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), LINKS_AND_SPACES).await;
    client
        .diagnostics_with_codes(&uri, &["missing-anchor", "MD051", "MD009"])
        .await;
    let config = fixture.write(".mrq.yaml", "lint:\n  extends: none\n");
    client.watched(&[(&config, FileChangeType::CREATED)]).await;
    client
        .diagnostics_with_codes(&uri, &["missing-anchor"])
        .await;
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn external_links_are_not_checked() {
    let fixture = Fixture::new(&[(
        ".mrq.yaml",
        "validate:\n  external: true\nlint:\n  extends: none\n",
    )]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client
        .open(
            &fixture.path("a.md"),
            "# A\n\n[x](http://127.0.0.1:9/nothing)\n",
        )
        .await;
    client.diagnostics_with_codes(&uri, &[]).await;
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_lint_finding_replaces_the_same_validate_finding() {
    // `stale-include` and MRQ004 report the same thing; the lint one has the fix.
    let fixture = Fixture::new(&[
        (
            ".mrq.yaml",
            "lint:\n  extends: none\n  rules:\n    MRQ004: on\n",
        ),
        ("part.md", "Part.\n"),
    ]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let text = "# A\n\n<!-- mrq:include path=\"part.md\" -->\nold\n<!-- mrq:end -->\n";
    let uri = client.open(&fixture.path("a.md"), text).await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(diagnostic_codes(&published), ["MRQ004"]);
    assert_eq!(
        published.diagnostics[0]
            .code_description
            .as_ref()
            .unwrap()
            .href
            .as_str(),
        "https://github.com/mtrense/marquee/blob/main/docs/rules/MRQ004.md"
    );
    assert!(published.diagnostics[0].data.is_some());
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn closing_a_document_clears_its_diagnostics() {
    let fixture = Fixture::new(&[("a.md", LINKS_AND_SPACES)]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), LINKS_AND_SPACES).await;
    client
        .diagnostics_with_codes(&uri, &["missing-anchor", "MD051", "MD009"])
        .await;
    client.close(&uri).await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.diagnostics, []);
    assert_eq!(published.version, None);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn workspace_files_that_are_not_open_get_no_diagnostics() {
    let fixture = Fixture::new(&[("a.md", LINKS_AND_SPACES), ("b.md", "# B\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let b = client.open(&fixture.path("b.md"), "# B\n").await;
    client.diagnostics(&b).await;
    client.sync().await;
    let others: Vec<_> = client
        .pending_notifications()
        .into_iter()
        .filter(|message| message["method"] == "textDocument/publishDiagnostics")
        .filter(|message| message["params"]["uri"] != b.as_str())
        .collect();
    assert_eq!(others, Vec::<serde_json::Value>::new());
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_link_is_re_evaluated_when_its_target_changes_in_the_editor() {
    let fixture = Fixture::new(&[(".mrq.yaml", "lint:\n  extends: none\n"), ("b.md", "# B\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let a = client.open(&fixture.path("a.md"), "[b](b.md#sec)\n").await;
    client.diagnostics_with_codes(&a, &["missing-anchor"]).await;

    let b = client.open(&fixture.path("b.md"), "# B\n").await;
    client.change_full(&b, "# B\n\n## Sec\n").await;
    client.diagnostics_with_codes(&a, &[]).await;

    // Closing `b.md` brings back the file's text, without the heading.
    client.close(&b).await;
    client.diagnostics_with_codes(&a, &["missing-anchor"]).await;
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_link_is_re_evaluated_when_its_target_changes_on_disk() {
    let fixture = Fixture::new(&[(".mrq.yaml", "lint:\n  extends: none\n"), ("b.md", "# B\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let a = client
        .open(&fixture.path("a.md"), "[b](b.md#sec) [c](c.md)\n")
        .await;
    client
        .diagnostics_with_codes(&a, &["missing-anchor", "missing-file"])
        .await;

    let b = fixture.write("b.md", "# Sec\n");
    let c = fixture.write("c.md", "# C\n");
    client
        .watched(&[(&b, FileChangeType::CHANGED), (&c, FileChangeType::CREATED)])
        .await;
    client.diagnostics_with_codes(&a, &[]).await;

    let c = fixture.remove("c.md");
    client.watched(&[(&c, FileChangeType::DELETED)]).await;
    client.diagnostics_with_codes(&a, &["missing-file"]).await;
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_wikilink_is_re_evaluated_when_its_target_appears() {
    let fixture = Fixture::new(&[(".mrq.yaml", "flavor: obsidian\nlint:\n  extends: none\n")]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let a = client.open(&fixture.path("a.md"), "[[Note]]\n").await;
    client
        .diagnostics_with_codes(&a, &["unresolved-wikilink"])
        .await;
    client.open(&fixture.path("sub/Note.md"), "# Note\n").await;
    client.diagnostics_with_codes(&a, &[]).await;
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn changes_in_quick_succession_are_published_once() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start_with(ServerOptions {
        diagnostics_delay: Duration::from_millis(300),
        ..ServerOptions::default()
    });
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), LINKS_AND_SPACES).await;
    client.change_full(&uri, "# A\n").await;
    client.change_full(&uri, "# A\n\nB\n").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(3));
    assert_eq!(published.diagnostics, []);
    client.sync().await;
    let more = client
        .pending_notifications()
        .into_iter()
        .filter(|message| message["method"] == "textDocument/publishDiagnostics")
        .count();
    assert_eq!(more, 0);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn documents_outside_the_root_folders_get_diagnostics() {
    let root = Fixture::new(&[]);
    let outside = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[root.root()]).await;
    let uri = client
        .open(&outside.path("a.md"), "# A\n\n[x](#nope)\n")
        .await;
    let published = client
        .notification_matching::<PublishDiagnostics>(|p| p.uri == uri)
        .await;
    assert_eq!(diagnostic_codes(&published), ["missing-anchor", "MD051"]);
    assert!(client.shutdown().await);
}
