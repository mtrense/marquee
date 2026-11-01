//! Code actions on a document: [`code_actions`] for `textDocument/codeAction`.
//!
//! - a quick fix (`quickfix`) for each lint finding with a fix in the requested range;
//! - "Refresh include region" (`refactor.rewrite`) for each `mrq:include` region the range
//!   touches (its markers included) whose content is out of date;
//! - "Fix all mrq lint findings" ([`FIX_ALL`]): what `mrq lint --fix` writes, fixed to a
//!   fixpoint ([`Linter::fix`](mrq_core::lint::Linter::fix));
//! - "Refresh all include regions" ([`REFRESH_INCLUDES`]): what `mrq interpolate
//!   --in-place` writes.
//!
//! Actions are computed for the current text of the document, never from the fixes in
//! the `data` of published diagnostics, which may be for an older version. Includes are
//! refreshed as `mrq validate` checks them: files are read from disk, `/…` from the root
//! folder, and remote includes (also in included files) fail, so the server never
//! touches the network. A region whose include fails somewhere is not refreshed, and a
//! bare start marker gets an `<!-- mrq:end -->`.

use std::path::Path;

use mrq_core::Document;
use mrq_core::directive::DirectiveKind;
use mrq_core::source::{ByteRange, PositionEncoding, TextEdit};
use mrq_core::transform::{FileLoader, Includer};
use tower_lsp_server::ls_types::{self, CodeActionKind, Uri};

use crate::diagnostics::to_lsp;
use crate::format;
use crate::index::{Root, WorkspaceIndex};

/// The kind of the action fixing every lint finding of a document.
pub const FIX_ALL: CodeActionKind = CodeActionKind::new("source.fixAll.mrq");
/// The kind of the action refreshing every include region of a document.
pub const REFRESH_INCLUDES: CodeActionKind = CodeActionKind::new("source.refreshIncludes.mrq");

/// The kinds of the actions the server offers, in the order they come.
pub const KINDS: [CodeActionKind; 4] = [
    CodeActionKind::QUICKFIX,
    CodeActionKind::REFACTOR_REWRITE,
    FIX_ALL,
    REFRESH_INCLUDES,
];

/// A code action: edits of the document it was asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub title: String,
    pub kind: CodeActionKind,
    /// Of the document's indexed text.
    pub edits: Vec<TextEdit>,
    /// The findings it fixes, as published.
    pub diagnostics: Vec<ls_types::Diagnostic>,
    /// Whether it is the fix to apply for its findings.
    pub preferred: bool,
}

/// What a client asks code actions for, in a document.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The selection, or the cursor.
    pub range: ByteRange,
    /// The diagnostics the client has there (`context.diagnostics`), as published for
    /// some version: a current finding with the same code and range gets its quick fix
    /// also where `range` does not touch it (a client may send only the cursor, with the
    /// diagnostics of its line).
    pub diagnostics: &'a [ls_types::Diagnostic],
    /// The kinds wanted (`context.only`); all if `None`.
    pub only: Option<&'a [CodeActionKind]>,
}

/// The actions for `request` in the document at `path` (`uri`), in the order of
/// [`KINDS`], of the kinds `request.only` (and their sub-kinds) if given; none for a
/// document that is not indexed.
pub fn code_actions(
    index: &WorkspaceIndex,
    path: &Path,
    uri: &Uri,
    request: Request<'_>,
    encoding: PositionEncoding,
) -> Vec<Action> {
    let (Some(root), Some(document)) = (index.root_of(path), index.document(path)) else {
        return Vec::new();
    };
    let range = request.range;
    let wanted = |kind: &CodeActionKind| is_wanted(kind, request.only);
    let mut actions = Vec::new();
    if wanted(&CodeActionKind::QUICKFIX) {
        actions.extend(quick_fixes(root, document, uri, request, encoding));
    }
    let refresh = if wanted(&CodeActionKind::REFACTOR_REWRITE) || wanted(&REFRESH_INCLUDES) {
        refreshable(root, document)
    } else {
        Vec::new()
    };
    if wanted(&CodeActionKind::REFACTOR_REWRITE) {
        actions.extend(
            refresh
                .iter()
                .filter(|(span, _)| touches(*span, range))
                .map(|(_, edit)| Action {
                    title: "Refresh include region".into(),
                    kind: CodeActionKind::REFACTOR_REWRITE,
                    edits: vec![edit.clone()],
                    diagnostics: Vec::new(),
                    preferred: false,
                }),
        );
    }
    if wanted(&FIX_ALL) {
        actions.extend(fix_all(root, document));
    }
    if wanted(&REFRESH_INCLUDES) && !refresh.is_empty() {
        actions.push(Action {
            title: "Refresh all include regions".into(),
            kind: REFRESH_INCLUDES,
            edits: refresh.into_iter().map(|(_, edit)| edit).collect(),
            diagnostics: Vec::new(),
            preferred: false,
        });
    }
    actions
}

/// Whether actions of `kind` are asked for: all are without `only`, else those of a
/// kind in it or under one (`source` takes `source.fixAll.mrq`).
fn is_wanted(kind: &CodeActionKind, only: Option<&[CodeActionKind]>) -> bool {
    let Some(only) = only else {
        return true;
    };
    let kind = kind.as_str();
    only.iter().any(|wanted| {
        kind.strip_prefix(wanted.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    })
}

/// Whether `span` and `range` overlap or touch (a cursor at either end of a span is on
/// it).
const fn touches(span: ByteRange, range: ByteRange) -> bool {
    span.start <= range.end && range.start <= span.end
}

/// A quick fix for each lint finding with a fix that the request's range touches or
/// that is one of its diagnostics.
fn quick_fixes(
    root: &Root,
    document: &Document,
    uri: &Uri,
    request: Request<'_>,
    encoding: PositionEncoding,
) -> Vec<Action> {
    let Some(linter) = root.linter() else {
        return Vec::new();
    };
    linter
        .check(document)
        .into_iter()
        .filter_map(|finding| {
            let fix = finding.fix.as_ref()?;
            let diagnostic = to_lsp(&finding, document.source(), uri, encoding);
            let asked = request
                .diagnostics
                .iter()
                .any(|d| d.code == diagnostic.code && d.range == diagnostic.range);
            (asked || touches(finding.range, request.range)).then(|| Action {
                title: fix.message.clone(),
                kind: CodeActionKind::QUICKFIX,
                edits: fix.edits.clone(),
                diagnostics: vec![diagnostic],
                preferred: true,
            })
        })
        .collect()
}

/// "Fix all", if fixing changes the document.
fn fix_all(root: &Root, document: &Document) -> Option<Action> {
    let fixed = root
        .linter()?
        .fix(document.clone(), root.workspace().options())
        .ok()?;
    let edits = format::diff(document.text(), fixed.document.text());
    (!edits.is_empty()).then(|| Action {
        title: "Fix all mrq lint findings".into(),
        kind: FIX_ALL,
        edits,
        diagnostics: Vec::new(),
        preferred: false,
    })
}

/// The include regions of `document` that are out of date, with the edit refreshing
/// each: its span (from the start marker to the end marker) and the edit.
fn refreshable(root: &Root, document: &Document) -> Vec<(ByteRange, TextEdit)> {
    let ast = document.ast();
    let spans: Vec<_> = document
        .directives()
        .iter()
        .filter(|region| region.kind == DirectiveKind::Include)
        .map(|region| region.span(ast))
        .collect();
    Includer::new()
        .loader(FileLoader)
        .root(root.path())
        .document_options(root.workspace().options().clone())
        .allow_remote(false)
        .refresh(document)
        .into_iter()
        .filter(|region| !region.has_errors())
        .filter_map(|region| {
            let span = spans
                .iter()
                .find(|span| span.start == region.marker.start)
                .copied()
                .unwrap_or(region.marker);
            Some((span, region.edit?))
        })
        .collect()
}
