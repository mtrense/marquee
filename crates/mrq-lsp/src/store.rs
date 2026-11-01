//! [`DocumentStore`]: the documents the client has open, with the text it sent.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use mrq_core::source::{PositionEncoding, SourceFile};
use tower_lsp_server::ls_types::{TextDocumentContentChangeEvent, Uri};

use crate::convert;

/// An open document: the client's version and text. The source's path is the file's,
/// `None` for URIs that are not `file:` (`untitled:`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDocument {
    pub version: i32,
    pub source: SourceFile,
}

/// The open documents by URI, kept in sync with `didOpen`, `didChange` (full and
/// incremental) and `didClose`.
#[derive(Debug, Clone, Default)]
pub struct DocumentStore {
    documents: HashMap<Uri, OpenDocument>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens (or reopens) `uri` with `text`; `path` is its file, if any.
    pub fn open(&mut self, uri: Uri, path: Option<PathBuf>, version: i32, text: String) {
        let source = SourceFile::new(path, text);
        self.documents.insert(uri, OpenDocument { version, source });
    }

    /// Applies `changes` in order to the open document `uri` and sets its version:
    /// a change without a range replaces the whole text, one with a range (in `encoding`
    /// units, clamped as [`convert::offset`](crate::convert::offset) does) the text in
    /// it. `None` if `uri` is not open.
    pub fn change(
        &mut self,
        uri: &Uri,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
        encoding: PositionEncoding,
    ) -> Option<&OpenDocument> {
        let doc = self.documents.get_mut(uri)?;
        for change in changes {
            let path = doc.source.path().map(Path::to_path_buf);
            let text = match change.range {
                None => change.text,
                Some(range) => {
                    let range = convert::byte_range(&doc.source, range, encoding);
                    let mut text = doc.source.text().to_owned();
                    text.replace_range(Range::from(range), &change.text);
                    text
                }
            };
            doc.source = SourceFile::new(path, text);
        }
        doc.version = version;
        Some(doc)
    }

    /// Closes `uri` and returns what it held.
    pub fn close(&mut self, uri: &Uri) -> Option<OpenDocument> {
        self.documents.remove(uri)
    }

    /// The open document `uri`.
    pub fn get(&self, uri: &Uri) -> Option<&OpenDocument> {
        self.documents.get(uri)
    }

    /// The open document of the file `path` (normalized, as
    /// [`convert::uri_to_path`](crate::convert::uri_to_path) gives it).
    pub fn get_path(&self, path: &Path) -> Option<&OpenDocument> {
        self.documents
            .values()
            .find(|doc| doc.source.path() == Some(path))
    }

    /// The open documents, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&Uri, &OpenDocument)> {
        self.documents.iter()
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use mrq_core::source::PositionEncoding::Utf16;
    use tower_lsp_server::ls_types::{Position, Range};

    use super::*;

    fn uri() -> Uri {
        "file:///a.md".parse().unwrap()
    }

    fn edit(start: (u32, u32), end: (u32, u32), text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range: Some(Range::new(
                Position::new(start.0, start.1),
                Position::new(end.0, end.1),
            )),
            range_length: None,
            text: text.into(),
        }
    }

    fn text(store: &DocumentStore) -> &str {
        store.get(&uri()).unwrap().source.text()
    }

    #[test]
    fn open_keeps_the_text_and_path() {
        let mut store = DocumentStore::new();
        store.open(uri(), Some("/a.md".into()), 3, "# A\n".into());
        let doc = store.get(&uri()).unwrap();
        assert_eq!((doc.version, doc.source.text()), (3, "# A\n"));
        assert_eq!(store.get_path(Path::new("/a.md")), Some(doc));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn incremental_changes_apply_in_order() {
        let mut store = DocumentStore::new();
        store.open(uri(), None, 1, "one\ntwo\nthree\n".into());
        let changes = vec![
            // Insert a line, then edit a line that moved down because of it.
            edit((1, 0), (1, 0), "new\n"),
            edit((2, 0), (2, 3), "TWO"),
            // Delete across lines.
            edit((0, 1), (1, 2), ""),
        ];
        let doc = store.change(&uri(), 2, changes, Utf16).unwrap();
        assert_eq!(doc.version, 2);
        assert_eq!(text(&store), "ow\nTWO\nthree\n");
    }

    #[test]
    fn utf16_ranges_and_clamping() {
        let mut store = DocumentStore::new();
        store.open(uri(), None, 1, "😀x\n".into());
        store.change(&uri(), 2, vec![edit((0, 2), (0, 3), "y")], Utf16);
        assert_eq!(text(&store), "😀y\n");
        store.change(&uri(), 3, vec![edit((0, 50), (5, 0), "!")], Utf16);
        assert_eq!(text(&store), "😀y!");
    }

    #[test]
    fn full_changes_replace_everything() {
        let mut store = DocumentStore::new();
        store.open(uri(), None, 1, "old".into());
        let full = TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "new".into(),
        };
        store.change(&uri(), 2, vec![full, edit((0, 3), (0, 3), "er")], Utf16);
        assert_eq!(text(&store), "newer");
    }

    #[test]
    fn changes_to_unknown_documents_are_ignored() {
        let mut store = DocumentStore::new();
        assert_eq!(
            store.change(&uri(), 2, vec![edit((0, 0), (0, 0), "x")], Utf16),
            None
        );
        assert!(store.is_empty());
    }

    #[test]
    fn close_removes_the_document() {
        let mut store = DocumentStore::new();
        store.open(uri(), None, 1, "x".into());
        assert_eq!(store.close(&uri()).unwrap().source.text(), "x");
        assert_eq!(store.get(&uri()), None);
    }
}
