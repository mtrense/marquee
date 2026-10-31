//! [`Workspace`]: the documents of one run, and path resolution between them.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use super::{Document, DocumentError, DocumentOptions};
use crate::input::Input;

/// A set of documents under a root directory, keyed by lexically normalized path.
///
/// Documents are added from resolved [`Input`]s ([`from_inputs`](Self::from_inputs)),
/// one by one ([`insert`](Self::insert)), or on demand ([`load`](Self::load), e.g. for
/// the target of a link). All are parsed with the workspace's [`DocumentOptions`] and
/// kept in the order they were added. Each document builds its own
/// [anchor index](Document::anchors) lazily.
///
/// Paths are normalized lexically (`.` dropped, `..` folded into its parent) and never
/// canonicalized: `docs/./a.md` and `docs/x/../a.md` are the same document as
/// `docs/a.md`, but a relative and an absolute path to one file are not. Use one form
/// throughout, as [`InputResolver`](crate::input::InputResolver) does for its base
/// directory. A document without a path (stdin) is kept too, at most one.
///
/// ```
/// use std::path::Path;
/// use mrq_core::document::Workspace;
///
/// let ws = Workspace::new("site");
/// let from = Path::new("site/docs/a.md");
/// assert_eq!(ws.resolve(Some(from), "../img/logo.png"), Path::new("site/img/logo.png"));
/// assert_eq!(ws.resolve(Some(from), "/index.md"), Path::new("site/index.md"));
/// ```
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    options: DocumentOptions,
    documents: Vec<Document>,
    /// Normalized path → index into `documents`.
    by_path: HashMap<PathBuf, usize>,
}

impl Workspace {
    /// An empty workspace under `root` with the default [`DocumentOptions`].
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_options(root, DocumentOptions::new())
    }

    /// An empty workspace under `root` that parses documents with `options`.
    pub fn with_options(root: impl Into<PathBuf>, options: DocumentOptions) -> Self {
        Self {
            root: root.into(),
            options,
            documents: Vec::new(),
            by_path: HashMap::new(),
        }
    }

    /// A workspace holding `inputs`, read and parsed in order.
    ///
    /// # Errors
    ///
    /// The first input that cannot be read or parsed.
    pub fn from_inputs(
        root: impl Into<PathBuf>,
        options: DocumentOptions,
        inputs: &[Input],
    ) -> Result<Self, DocumentError> {
        let mut workspace = Self::with_options(root, options);
        for input in inputs {
            let document = workspace.options.read(input)?;
            workspace.insert(document);
        }
        Ok(workspace)
    }

    /// The root directory, against which links starting with `/` resolve.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// How documents are parsed.
    pub const fn options(&self) -> &DocumentOptions {
        &self.options
    }

    /// Resolves a link's path `target` (already split from any `#fragment`, query string
    /// and percent-encoding) from the document at `from`: relative to `from`'s directory,
    /// or to the [root](Self::root) if `target` starts with `/` or `from` is `None`
    /// (stdin). The result is normalized; `..` beyond the start is kept.
    pub fn resolve(&self, from: Option<&Path>, target: impl AsRef<Path>) -> PathBuf {
        resolve(&self.root, from, target.as_ref())
    }

    /// The document at `path`, read and parsed if it is not in the workspace yet.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or the workspace's flavor options are invalid. Nothing
    /// is added then, so a later call tries again.
    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<&Document, DocumentError> {
        let key = normalize(path.as_ref());
        if let Some(&index) = self.by_path.get(&key) {
            return Ok(&self.documents[index]);
        }
        let document = self.options.load(&key)?;
        self.documents.push(document);
        self.by_path.insert(key, self.documents.len() - 1);
        Ok(&self.documents[self.documents.len() - 1])
    }

    /// Adds `document`, replacing (and returning) the one with the same normalized path,
    /// or the pathless one if `document` has no path. A replacement keeps its position.
    pub fn insert(&mut self, document: Document) -> Option<Document> {
        let key = document.path().map(normalize);
        let existing = match &key {
            Some(key) => self.by_path.get(key).copied(),
            None => self.documents.iter().position(|doc| doc.path().is_none()),
        };
        if let Some(index) = existing {
            return Some(std::mem::replace(&mut self.documents[index], document));
        }
        self.documents.push(document);
        if let Some(key) = key {
            self.by_path.insert(key, self.documents.len() - 1);
        }
        None
    }

    /// The document at `path`, if it is in the workspace.
    pub fn get(&self, path: impl AsRef<Path>) -> Option<&Document> {
        let index = *self.by_path.get(&normalize(path.as_ref()))?;
        Some(&self.documents[index])
    }

    /// Removes and returns the document at `path`.
    pub fn remove(&mut self, path: impl AsRef<Path>) -> Option<Document> {
        let index = self.by_path.remove(&normalize(path.as_ref()))?;
        let document = self.documents.remove(index);
        for later in self.by_path.values_mut().filter(|i| **i > index) {
            *later -= 1;
        }
        Some(document)
    }

    /// The documents in the order they were added.
    pub fn documents(&self) -> impl ExactSizeIterator<Item = &Document> {
        self.documents.iter()
    }

    /// Number of documents.
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

/// [`Workspace::resolve`] for the root `root`.
pub(crate) fn resolve(root: &Path, from: Option<&Path>, target: &Path) -> PathBuf {
    let (base, rest) = match (target.strip_prefix("/"), from) {
        (Ok(rest), _) => (root, rest),
        (Err(_), Some(from)) => (from.parent().unwrap_or(Path::new("")), target),
        (Err(_), None) => (root, target),
    };
    normalize(&base.join(rest))
}

/// `path` with `.` components dropped and `..` folded into a preceding normal component;
/// `..` at the start (or after another `..`) is kept, and `..` right after the root is
/// dropped. An empty result is `.`.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_dots() {
        let cases = [
            ("a/./b/../c.md", "a/c.md"),
            ("./a.md", "a.md"),
            ("../a/../../b", "../../b"),
            ("/../a", "/a"),
            ("a/..", "."),
            ("", "."),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize(Path::new(input)), Path::new(expected), "{input}");
        }
    }
}
