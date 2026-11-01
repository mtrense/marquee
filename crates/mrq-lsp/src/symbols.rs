//! Symbols: the headings of a document as an outline (`textDocument/documentSymbol`,
//! [`document_symbols`]) and those of every indexed document (`workspace/symbol`,
//! [`workspace_symbols`]).
//!
//! Headings are `String` symbols, as VS Code's Markdown support makes them. A heading
//! nests under the closest preceding heading of a lower level, as in
//! [`toc`](mrq_core::toc#nesting), and its range is its section: up to the next heading
//! of the same or a lower level.

use mrq_core::Document;
use mrq_core::ast::Span;
use mrq_core::document::TocEntry;
use mrq_core::source::PositionEncoding;
use tower_lsp_server::ls_types::{
    DocumentSymbol, Location, OneOf, Position, SymbolKind, WorkspaceSymbol,
};

use crate::convert::{self, path_to_uri};
use crate::index::WorkspaceIndex;

/// The headings of `document`, nested.
pub fn document_symbols(document: &Document, encoding: PositionEncoding) -> Vec<DocumentSymbol> {
    let entries = document.toc();
    let source = document.source();
    let mut top = Vec::new();
    let mut open: Vec<(u8, DocumentSymbol)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let end = entries[i + 1..]
            .iter()
            .find(|next| next.level <= entry.level)
            .map_or(document.text().len(), |next| next.span.start);
        #[allow(deprecated, reason = "the field must be set")]
        let symbol = DocumentSymbol {
            name: name(entry),
            detail: None,
            kind: SymbolKind::STRING,
            tags: None,
            deprecated: None,
            range: convert::range(source, Span::new(entry.span.start, end), encoding),
            selection_range: convert::range(source, entry.span, encoding),
            children: None,
        };
        while open.last().is_some_and(|(level, _)| *level >= entry.level) {
            close(&mut open, &mut top);
        }
        open.push((entry.level, symbol));
    }
    while !open.is_empty() {
        close(&mut open, &mut top);
    }
    top
}

/// Moves the innermost open symbol into its parent, or to `top`.
fn close(open: &mut Vec<(u8, DocumentSymbol)>, top: &mut Vec<DocumentSymbol>) {
    let Some((_, symbol)) = open.pop() else {
        return;
    };
    match open.last_mut() {
        Some((_, parent)) => parent.children.get_or_insert_with(Vec::new).push(symbol),
        None => top.push(symbol),
    }
}

/// The headings of every indexed document whose text contains the characters of `query`
/// in order, ignoring case (all for an empty query), sorted by URI and position. The
/// container is the file, relative to its root.
pub fn workspace_symbols(
    index: &WorkspaceIndex,
    query: &str,
    encoding: PositionEncoding,
) -> Vec<WorkspaceSymbol> {
    let query = query.to_lowercase();
    let mut found = Vec::new();
    for (root, document) in index.documents() {
        let Some(path) = document.path() else {
            continue;
        };
        let Some(uri) = path_to_uri(path) else {
            continue;
        };
        let container = path.strip_prefix(root.path()).unwrap_or(path);
        let container = container.to_string_lossy().replace('\\', "/");
        for entry in document.toc() {
            if !matches_fuzzily(&entry.text, &query) {
                continue;
            }
            let range = convert::range(document.source(), entry.span, encoding);
            found.push(WorkspaceSymbol {
                name: name(&entry),
                kind: SymbolKind::STRING,
                tags: None,
                container_name: Some(container.clone()),
                location: OneOf::Left(Location::new(uri.clone(), range)),
                data: None,
            });
        }
    }
    let key = |symbol: &WorkspaceSymbol| match &symbol.location {
        OneOf::Left(location) => (location.uri.as_str().to_owned(), location.range.start),
        OneOf::Right(location) => (location.uri.as_str().to_owned(), Position::default()),
    };
    found.sort_by_key(key);
    found
}

/// Whether `text` contains the characters of the lowercased `query` in order.
fn matches_fuzzily(text: &str, query: &str) -> bool {
    let mut text = text.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .all(|wanted| text.any(|c| c == wanted))
}

/// A heading's symbol name: its text, or its `#`s if it has none (names must not be
/// empty).
fn name(entry: &TocEntry) -> String {
    if entry.text.trim().is_empty() {
        "#".repeat(usize::from(entry.level))
    } else {
        entry.text.clone()
    }
}
