//! Code actions: a quick fix per lint finding with a fix, "fix all" to a fixpoint, and
//! refreshing `mrq:include` regions, always against the current text of the document.

mod support;

use mrq_core::config::Config;
use mrq_core::document::DocumentOptions;
use mrq_core::lint::{Linter, Registry};
use mrq_lsp::ls_types::{
    CodeAction, CodeActionContext, CodeActionKind, CodeActionOptions, CodeActionProviderCapability,
    DocumentChanges, NumberOrString, Range, Uri,
};
use support::{
    Fixture, TestClient, apply, diagnostic_codes, initialize_params_with_document_changes, range,
    text_edits,
};

/// Trailing spaces (MD009, line 2), two blank lines (MD012, line 4) and no final newline
/// (MD047, line 7).
const FIXABLE: &str = "# A\n\nText \n\n\n## B\n\nEnd";
const FIXED: &str = "# A\n\nText\n\n## B\n\nEnd\n";

const FIX_ALL: CodeActionKind = CodeActionKind::new("source.fixAll.mrq");
const REFRESH_INCLUDES: CodeActionKind = CodeActionKind::new("source.refreshIncludes.mrq");

/// The whole of a document of fewer than 100 lines.
fn everything() -> Range {
    range((0, 0), (100, 0))
}

/// `text` with the edits of `action` to `uri` applied.
fn applied(text: &str, uri: &Uri, action: &CodeAction) -> String {
    let edits = text_edits(action.edit.as_ref().expect("an edit"));
    assert_eq!(edits.len(), 1, "edits of one document: {edits:?}");
    apply(text, &edits[uri.as_str()])
}

fn titles(actions: &[CodeAction]) -> Vec<(&str, &str)> {
    actions
        .iter()
        .map(|action| {
            let kind = action.kind.as_ref().map_or("", CodeActionKind::as_str);
            (action.title.as_str(), kind)
        })
        .collect()
}

#[tokio::test]
async fn initialize_advertises_code_actions() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let capabilities = client.initialize(&[fixture.root()]).await.capabilities;
    let Some(CodeActionProviderCapability::Options(CodeActionOptions {
        code_action_kinds: Some(kinds),
        resolve_provider,
        ..
    })) = capabilities.code_action_provider
    else {
        panic!("no code action options: {capabilities:?}");
    };
    assert_eq!(
        kinds,
        [
            CodeActionKind::QUICKFIX,
            CodeActionKind::REFACTOR_REWRITE,
            FIX_ALL,
            REFRESH_INCLUDES
        ]
    );
    assert_ne!(resolve_provider, Some(true));
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_finding_with_a_fix_gets_a_quick_fix() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), FIXABLE).await;
    let actions = client.code_actions(&uri, range((2, 4), (2, 4)), None).await;
    assert_eq!(
        titles(&actions),
        [
            ("Delete the trailing spaces", "quickfix"),
            ("Fix all mrq lint findings", "source.fixAll.mrq"),
        ]
    );
    let quickfix = &actions[0];
    assert_eq!(quickfix.is_preferred, Some(true));
    let diagnostics = quickfix.diagnostics.as_ref().expect("the finding");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code,
        Some(NumberOrString::String("MD009".into()))
    );
    assert_eq!(diagnostics[0].range, range((2, 4), (2, 5)));
    assert_eq!(
        applied(FIXABLE, &uri, quickfix),
        "# A\n\nText\n\n\n## B\n\nEnd"
    );
    // A range over several findings gets a quick fix for each of them.
    let actions = client.code_actions(&uri, everything(), None).await;
    assert_eq!(
        titles(&actions)
            .iter()
            .filter(|(_, kind)| *kind == "quickfix")
            .count(),
        3
    );
    // Where nothing is fixable, only "fix all" for the document.
    let actions = client.code_actions(&uri, range((0, 0), (0, 3)), None).await;
    assert_eq!(
        titles(&actions),
        [("Fix all mrq lint findings", "source.fixAll.mrq")]
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn the_diagnostics_of_the_context_get_their_quick_fixes() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), FIXABLE).await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(diagnostic_codes(&published), ["MD009", "MD012", "MD047"]);
    // As Neovim asks: the cursor elsewhere on the line, the line's diagnostics given.
    let context = CodeActionContext {
        diagnostics: vec![published.diagnostics[0].clone()],
        only: Some(vec![CodeActionKind::QUICKFIX]),
        ..CodeActionContext::default()
    };
    let actions = client
        .code_actions_in(&uri, range((2, 1), (2, 1)), context)
        .await;
    assert_eq!(
        titles(&actions),
        [("Delete the trailing spaces", "quickfix")]
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn fix_all_fixes_to_a_fixpoint_as_lint_fix_does() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    // Fixing the list's indentation (MD007) leaves blanks (MD012) for a second pass.
    let text = "# A\n\n\n\n\nText \n\n  - one\n  - two\n";
    let uri = client.open(&fixture.path("a.md"), text).await;
    let actions = client
        .code_actions(&uri, everything(), Some(&[CodeActionKind::SOURCE_FIX_ALL]))
        .await;
    assert_eq!(
        titles(&actions),
        [("Fix all mrq lint findings", "source.fixAll.mrq")]
    );
    let linter = Linter::new(&Registry::builtin(), &Config::default().lint).unwrap();
    let options = DocumentOptions::new();
    let document = options.parse(text).unwrap();
    let fixed = linter.fix(document, &options).unwrap();
    assert_eq!(applied(text, &uri, &actions[0]), fixed.document.text());

    let uri = client.open(&fixture.path("b.md"), FIXABLE).await;
    let actions = client
        .code_actions(&uri, range((0, 0), (0, 0)), Some(&[FIX_ALL]))
        .await;
    assert_eq!(applied(FIXABLE, &uri, &actions[0]), FIXED);
    // Nothing left to fix: no action.
    client.change_full(&uri, FIXED).await;
    let actions = client.code_actions(&uri, everything(), None).await;
    assert_eq!(titles(&actions), []);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn only_the_requested_kinds_are_returned() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), FIXABLE).await;
    let at = range((2, 4), (2, 4));
    let quickfixes = client
        .code_actions(&uri, at, Some(&[CodeActionKind::QUICKFIX]))
        .await;
    assert_eq!(
        titles(&quickfixes),
        [("Delete the trailing spaces", "quickfix")]
    );
    // A parent kind takes its children.
    let source = client
        .code_actions(&uri, at, Some(&[CodeActionKind::SOURCE]))
        .await;
    assert_eq!(
        titles(&source),
        [("Fix all mrq lint findings", "source.fixAll.mrq")]
    );
    let none = client
        .code_actions(&uri, at, Some(&[CodeActionKind::REFACTOR]))
        .await;
    assert_eq!(titles(&none), []);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn actions_are_computed_for_the_current_text() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client
        .initialize_with(initialize_params_with_document_changes(&[fixture.root()]))
        .await;
    let uri = client.open(&fixture.path("a.md"), FIXABLE).await;
    client.diagnostics(&uri).await;
    // Two lines in front of everything: the published fixes are two lines off now.
    let text = format!("Intro\n\n{FIXABLE}");
    client.change_full(&uri, &text).await;
    let actions = client
        .code_actions(
            &uri,
            range((4, 4), (4, 4)),
            Some(&[CodeActionKind::QUICKFIX]),
        )
        .await;
    assert_eq!(
        titles(&actions),
        [("Delete the trailing spaces", "quickfix")]
    );
    assert_eq!(
        applied(&text, &uri, &actions[0]),
        "Intro\n\n# A\n\nText\n\n\n## B\n\nEnd"
    );
    // The edit is for that version of the document.
    let Some(DocumentChanges::Edits(changes)) = &actions[0].edit.as_ref().unwrap().document_changes
    else {
        panic!("no document changes");
    };
    assert_eq!(changes[0].text_document.version, Some(2));
    assert!(client.shutdown().await);
}

/// `part.md` and a document including it twice, with an outdated first region.
fn includes() -> (Fixture, &'static str) {
    let fixture = Fixture::new(&[
        (
            ".mrq.yaml",
            "lint:\n  extends: none\n  rules:\n    MD009: on\n",
        ),
        ("part.md", "Part.\n"),
    ]);
    let text = "# A\n\n<!-- mrq:include path=\"part.md\" -->\nold\n<!-- mrq:end -->\n\nText\n\n<!-- mrq:include path=\"part.md\" -->\n";
    (fixture, text)
}

#[tokio::test]
async fn an_include_region_can_be_refreshed() {
    let (fixture, text) = includes();
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let uri = client.open(&fixture.path("a.md"), text).await;
    let refreshed = "# A\n\n<!-- mrq:include path=\"part.md\" -->\n\nPart.\n\n<!-- mrq:end -->\n\nText\n\n<!-- mrq:include path=\"part.md\" -->\n";
    // Anywhere in the region, its markers included.
    for at in [(2, 3), (3, 1), (4, 16)] {
        let actions = client.code_actions(&uri, range(at, at), None).await;
        assert_eq!(
            titles(&actions),
            [
                ("Refresh include region", "refactor.rewrite"),
                ("Refresh all include regions", "source.refreshIncludes.mrq"),
            ],
            "at {at:?}"
        );
        assert_eq!(applied(text, &uri, &actions[0]), refreshed);
    }
    // The bare region gets its content and an end marker.
    let actions = client.code_actions(&uri, range((8, 5), (8, 5)), None).await;
    assert_eq!(
        applied(text, &uri, &actions[0]),
        "# A\n\n<!-- mrq:include path=\"part.md\" -->\nold\n<!-- mrq:end -->\n\nText\n\n<!-- mrq:include path=\"part.md\" -->\n\nPart.\n\n<!-- mrq:end -->\n"
    );
    // Outside the regions, only the whole document.
    let actions = client.code_actions(&uri, range((6, 1), (6, 1)), None).await;
    assert_eq!(
        titles(&actions),
        [("Refresh all include regions", "source.refreshIncludes.mrq")]
    );
    assert_eq!(
        applied(text, &uri, &actions[0]),
        "# A\n\n<!-- mrq:include path=\"part.md\" -->\n\nPart.\n\n<!-- mrq:end -->\n\nText\n\n<!-- mrq:include path=\"part.md\" -->\n\nPart.\n\n<!-- mrq:end -->\n"
    );
    // Up to date: nothing to refresh.
    let current = applied(text, &uri, &actions[0]);
    client.change_full(&uri, &current).await;
    let actions = client.code_actions(&uri, everything(), None).await;
    assert_eq!(titles(&actions), []);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn a_failing_include_is_not_refreshed() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    let text = "<!-- mrq:include path=\"gone.md\" -->\nold\n<!-- mrq:end -->\n";
    let uri = client.open(&fixture.path("a.md"), text).await;
    let actions = client.code_actions(&uri, everything(), None).await;
    assert_eq!(titles(&actions), []);
    assert!(client.shutdown().await);
}
