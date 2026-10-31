//! Where included text comes from: [`IncludeTarget`] and [`Loader`].

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::document::normalize;

/// What an `mrq:include`'s `path` names, resolved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IncludeTarget {
    /// A local file: the `path` resolved from the including file's directory (or the
    /// root, for `/…`), lexically normalized.
    File(PathBuf),
    /// An `http(s)://` URL, as written.
    Url(String),
}

impl IncludeTarget {
    /// What identifies the target for cycle detection: a file's canonical path (its
    /// absolute, normalized path if it cannot be canonicalized), or the URL.
    pub fn identity(&self) -> String {
        match self {
            Self::File(path) => identity(path).display().to_string(),
            Self::Url(url) => url.clone(),
        }
    }

    /// The local file, if the target is one.
    pub fn file(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path),
            Self::Url(_) => None,
        }
    }
}

impl fmt::Display for IncludeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Url(url) => f.write_str(url),
        }
    }
}

/// The canonical path of `path`, else its absolute normalized path.
pub(super) fn identity(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path).map(|path| normalize(&path)))
        .unwrap_or_else(|_| normalize(path))
}

/// Reads the text of include targets for an [`Includer`](super::Includer).
///
/// [`FileLoader`] reads files from disk; other loaders can serve unsaved editor buffers
/// or fetch URLs. A target that does not exist is an error of kind
/// [`io::ErrorKind::NotFound`] (reported as a missing include), and one the loader cannot
/// handle is [`io::ErrorKind::Unsupported`].
pub trait Loader: Send + Sync {
    /// The text of `target`.
    ///
    /// # Errors
    ///
    /// The target cannot be read, or is not valid UTF-8.
    fn load(&self, target: &IncludeTarget) -> io::Result<String>;
}

/// Reads local files; URLs are [`Unsupported`](io::ErrorKind::Unsupported).
#[derive(Debug, Clone, Copy, Default)]
pub struct FileLoader;

impl Loader for FileLoader {
    fn load(&self, target: &IncludeTarget) -> io::Result<String> {
        match target {
            IncludeTarget::File(path) => fs::read_to_string(path),
            IncludeTarget::Url(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "remote includes are not supported",
            )),
        }
    }
}
