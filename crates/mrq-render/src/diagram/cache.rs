//! [`DiagramCache`]: rendered diagrams on disk, named by a content hash.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use ring::digest::{Context, SHA256};

use super::{DiagramFormat, DiagramOutput};

/// Bumped when the key or the entry layout changes, so old entries are never read.
const VERSION: &str = "mrq-diagram-cache-1";

/// A directory of rendered diagrams, one file per render, named by the SHA-256 of what
/// the render depends on (see the [module docs](super#cache)) with the format's
/// extension. Entries are written atomically (a sibling, then renamed); an entry that
/// cannot be read or does not look like its format is a miss, and a cache that cannot be
/// written only costs renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramCache {
    dir: PathBuf,
}

impl DiagramCache {
    /// A cache in `dir`, created on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The cache in `<user cache>/diagrams`, with the user cache directory from
    /// [`user_cache_dir`](mrq_core::config::user_cache_dir) and the environment `env`
    /// (`|key| std::env::var(key).ok()`); `None` when there is none.
    pub fn in_user_cache(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        mrq_core::config::user_cache_dir(env).map(|dir| Self::new(dir.join("diagrams")))
    }

    /// The directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The entry for `key` in `format`, if there is a usable one.
    pub(super) fn get(&self, key: &str, format: DiagramFormat) -> Option<DiagramOutput> {
        let bytes = fs::read(self.path(key, format)).ok()?;
        DiagramOutput::from_bytes(format, bytes).ok()
    }

    /// Stores `output` as the entry for `key`.
    pub(super) fn put(&self, key: &str, output: &DiagramOutput) {
        let path = self.path(key, output.format());
        if fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let mut temp = path.clone().into_os_string();
        temp.push(format!(".{}.tmp", std::process::id()));
        if fs::write(&temp, output.as_bytes()).is_ok() && fs::rename(&temp, &path).is_err() {
            let _ = fs::remove_file(&temp);
        }
    }

    fn path(&self, key: &str, format: DiagramFormat) -> PathBuf {
        self.dir.join(format!("{key}.{}", format.extension()))
    }
}

/// The hex SHA-256 of the cache version, `fingerprint`, `lang`, `format` and `src`, each
/// length-prefixed so that no two different inputs run together.
pub(super) fn key(fingerprint: &str, lang: &str, format: DiagramFormat, src: &str) -> String {
    let mut context = Context::new(&SHA256);
    for part in [VERSION, fingerprint, lang, format.as_str(), src] {
        context.update(&(part.len() as u64).to_le_bytes());
        context.update(part.as_bytes());
    }
    let mut hex = String::with_capacity(64);
    for byte in context.finish().as_ref() {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_hex_sha256_and_separate_their_parts() {
        let a = key("r", "x", DiagramFormat::Svg, "ab");
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(a, key("r", "x", DiagramFormat::Svg, "ab"));
        assert_ne!(a, key("rx", "", DiagramFormat::Svg, "ab"));
        assert_ne!(a, key("r", "x", DiagramFormat::Png, "ab"));
        assert_ne!(a, key("r", "x", DiagramFormat::Svg, "ab\n"));
    }
}
