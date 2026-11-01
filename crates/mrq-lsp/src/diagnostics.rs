//! Diagnostics: what `mrq validate` (its internal checks) and `mrq lint` find in the open
//! documents, published by a task of the server once the documents stop changing for
//! [`ServerOptions::diagnostics_delay`](crate::ServerOptions::diagnostics_delay).
//!
//! Only open documents get diagnostics; closing one clears them. A document's checks
//! read other files (link targets and their anchors, wikilink names, included files),
//! so every open document is checked again after any change: a document edited or
//! closed, a watched file, the configuration or the root folders. Each round shares one
//! [`Checker`] per root, so link targets are read once. External links are never checked.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::Document;
use mrq_core::source::{PositionEncoding, SourceFile};
use mrq_core::validate::{Checker, codes};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tower_lsp_server::Client;
use tower_lsp_server::ls_types::{
    self, CodeDescription, DiagnosticRelatedInformation, DiagnosticSeverity, DiagnosticTag,
    Location, NumberOrString, TextEdit, Uri,
};

use crate::convert;
use crate::index::Root;
use crate::server::State;

/// Where mrq's own lint rules (`MRQ001`, …) are documented.
const RULES_DOCS: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/blob/main/docs/rules");
/// Where the markdownlint rules (`MD001`, …) are documented.
const MARKDOWNLINT_DOCS: &str = "https://github.com/DavidAnson/markdownlint/blob/main/doc";

/// The codes of findings about something unused, which editors may fade out.
const UNNECESSARY: [&str; 3] = [codes::UNUSED_FOOTNOTE, codes::UNUSED_REFERENCE, "MD053"];

/// The `data` of a published diagnostic whose finding has a fix, for code actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticData {
    pub fix: FixData,
}

/// A lint fix in LSP terms: edits of the document at the published version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixData {
    pub title: String,
    pub edits: Vec<TextEdit>,
}

/// The findings for `document` of `root`: problems found when parsing it, the internal
/// checks of `checker` (a checker of `root`) and the configured lint rules, ordered by
/// position. A lint finding with the same range and message as a check's (MRQ004 and
/// `stale-include`, MRQ001 and `missing-anchor`) replaces it, since it may carry a fix.
pub fn check(root: &Root, checker: &mut Checker<'_>, document: &Document) -> Vec<Diagnostic> {
    let mut found = document.diagnostics().to_vec();
    found.extend(checker.check(document));
    if let Some(linter) = root.linter() {
        let lint = linter.check(document);
        found.retain(|d| {
            !lint
                .iter()
                .any(|l| l.range == d.range && l.message == d.message)
        });
        found.extend(lint);
    }
    // Stable, so a check comes before a lint finding at the same range.
    found.sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.range.end));
    found
}

/// `diagnostic`, a finding in `source` (the document at `uri`), as an LSP diagnostic:
/// the help after the message, labels as related information, a link to the rule's
/// documentation, and the fix in [`DiagnosticData`].
pub fn to_lsp(
    diagnostic: &Diagnostic,
    source: &SourceFile,
    uri: &Uri,
    encoding: PositionEncoding,
) -> ls_types::Diagnostic {
    let range = |range| convert::range(source, range, encoding);
    let message = match &diagnostic.help {
        Some(help) => format!("{}\n\nhelp: {help}", diagnostic.message),
        None => diagnostic.message.clone(),
    };
    let related: Vec<DiagnosticRelatedInformation> = diagnostic
        .labels
        .iter()
        .map(|label| DiagnosticRelatedInformation {
            location: Location::new(uri.clone(), range(label.range)),
            message: label.message.clone(),
        })
        .collect();
    let data = diagnostic.fix.as_ref().map(|fix| DiagnosticData {
        fix: FixData {
            title: fix.message.clone(),
            edits: fix
                .edits
                .iter()
                .map(|edit| TextEdit::new(range(edit.range), edit.replacement.clone()))
                .collect(),
        },
    });
    ls_types::Diagnostic {
        range: range(diagnostic.range),
        severity: Some(severity(diagnostic.severity)),
        code: Some(NumberOrString::String(diagnostic.code.clone())),
        code_description: docs_url(diagnostic)
            .and_then(|url| url.parse().ok())
            .map(|href| CodeDescription { href }),
        source: Some("mrq".into()),
        message,
        related_information: (!related.is_empty()).then_some(related),
        tags: UNNECESSARY
            .contains(&diagnostic.code.as_str())
            .then(|| vec![DiagnosticTag::UNNECESSARY]),
        data: data.and_then(|data| serde_json::to_value(data).ok()),
    }
}

const fn severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
        Severity::Hint => DiagnosticSeverity::HINT,
    }
}

/// The documentation of `diagnostic`'s code: its own URL, else that of an mrq or a
/// markdownlint rule; the internal checks have none.
fn docs_url(diagnostic: &Diagnostic) -> Option<String> {
    if let Some(url) = &diagnostic.url {
        return Some(url.clone());
    }
    let code = diagnostic.code.as_str();
    let numbered = |prefix: &str| {
        code.strip_prefix(prefix)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    if numbered("MRQ") {
        Some(format!("{RULES_DOCS}/{code}.md"))
    } else if numbered("MD") {
        Some(format!("{MARKDOWNLINT_DOCS}/{}.md", code.to_lowercase()))
    } else {
        None
    }
}

/// What a handler tells the [`Publisher`]; either way, every open document is checked.
#[derive(Debug)]
pub(crate) enum Change {
    /// A document, a file, the configuration or the root folders changed.
    Updated,
    /// The document was closed: also clear its diagnostics (unless it is open again).
    Closed(Uri),
}

/// Publishes diagnostics from a task of its own: it waits until no [`Change`] came for
/// the delay, then checks the open documents and publishes for each of them (and clears
/// the closed ones), one round at a time, so a newer round never goes out before an
/// older one.
#[derive(Debug, Clone)]
pub(crate) struct Publisher {
    changes: mpsc::UnboundedSender<Change>,
}

impl Publisher {
    /// Starts the task on the current runtime; it ends with the last `Publisher`.
    pub(crate) fn spawn(client: Client, state: Arc<Mutex<State>>, delay: Duration) -> Self {
        let (changes, received) = mpsc::unbounded_channel();
        tokio::spawn(publish(client, state, delay, received));
        Self { changes }
    }

    pub(crate) fn changed(&self, change: Change) {
        // The task only stops when the server does.
        let _ = self.changes.send(change);
    }
}

async fn publish(
    client: Client,
    state: Arc<Mutex<State>>,
    delay: Duration,
    mut changes: mpsc::UnboundedReceiver<Change>,
) {
    while let Some(first) = changes.recv().await {
        let mut closed = HashSet::new();
        let mut note = |change| {
            if let Change::Closed(uri) = change {
                closed.insert(uri);
            }
        };
        note(first);
        loop {
            match tokio::time::timeout(delay, changes.recv()).await {
                Ok(Some(change)) => note(change),
                Ok(None) => return,
                Err(_) => break,
            }
        }
        let state = Arc::clone(&state);
        let round = tokio::task::spawn_blocking(move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            diagnose(&state, &closed)
        })
        .await;
        for (uri, version, diagnostics) in round.unwrap_or_default() {
            client.publish_diagnostics(uri, diagnostics, version).await;
        }
    }
}

/// The diagnostics of every open document with its version, and none for the `closed`
/// documents that are not open again.
fn diagnose(
    state: &State,
    closed: &HashSet<Uri>,
) -> Vec<(Uri, Option<i32>, Vec<ls_types::Diagnostic>)> {
    let mut checkers: Vec<(&Root, Checker<'_>)> = Vec::new();
    let mut round = Vec::new();
    for (uri, open) in state.store.iter() {
        let Some(path) = open.source.path() else {
            continue;
        };
        let Some(root) = state.index.root_of(path) else {
            continue;
        };
        let Some(document) = root.workspace().get(path) else {
            continue;
        };
        let i = checkers
            .iter()
            .position(|(r, _)| std::ptr::eq(*r, root))
            .unwrap_or_else(|| {
                let checker =
                    Checker::new(root.workspace()).with_frontmatter_schemas(root.schemas());
                checkers.push((root, checker));
                checkers.len() - 1
            });
        let diagnostics = check(root, &mut checkers[i].1, document)
            .iter()
            .map(|diagnostic| to_lsp(diagnostic, &open.source, uri, state.encoding))
            .collect();
        round.push((uri.clone(), Some(open.version), diagnostics));
    }
    for uri in closed {
        if state.store.get(uri).is_none() {
            round.push((uri.clone(), None, Vec::new()));
        }
    }
    round
}
