//! [`HttpLoader`]: the [`Loader`] of remote includes, over HTTP(S) with reqwest and
//! rustls (feature `remote-include`), with its on-disk cache.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

use super::{FileLoader, IncludeTarget, Loader};
use crate::config::user_cache_dir;
use crate::http::{describe, now, write_atomic};

/// How an [`HttpLoader`] fetches: the timeout, the `User-Agent` and the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpLoaderOptions {
    /// The timeout of each request, redirects and the body included. Default 20 s.
    pub timeout: Duration,
    /// The `User-Agent` header. Default `mrq/<version>`.
    pub user_agent: String,
    /// Where fetched content is kept; `None` (the default) for no cache.
    pub cache: Option<IncludeCache>,
}

impl Default for HttpLoaderOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            user_agent: concat!("mrq/", env!("CARGO_PKG_VERSION")).to_owned(),
            cache: None,
        }
    }
}

/// The on-disk cache of an [`HttpLoader`]: a directory with one file per content, named
/// by its SHA-256, and `index.json`, which URL had which content when.
///
/// Content fetched less than `ttl` ago is not fetched again. Pinned content
/// ([`Loader::load_pinned`]) is served from the cache whenever it has a file with that
/// hash, however old, since the pin says what the content is. Files whose content does
/// not match their name are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeCache {
    /// The cache directory; created when the cache is written.
    pub dir: PathBuf,
    /// How long fetched content of an unpinned include is trusted.
    pub ttl: Duration,
}

impl IncludeCache {
    /// The default time to live: one hour.
    pub const DEFAULT_TTL: Duration = Duration::from_hours(1);

    /// `remote-includes/` in the [user cache directory](user_cache_dir) with the
    /// [default TTL](Self::DEFAULT_TTL); `None` if there is no such directory.
    pub fn in_user_cache_dir() -> Option<Self> {
        let dir = user_cache_dir(|key| std::env::var(key).ok())?;
        Some(Self {
            dir: dir.join("remote-includes"),
            ttl: Self::DEFAULT_TTL,
        })
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    /// The cached text of `url`: the content `sha256` pins, else the content fetched
    /// within the TTL before `now`.
    fn get(&self, url: &str, sha256: Option<&str>, now: u64) -> Option<String> {
        let hash = if let Some(hash) = sha256 {
            hash.to_owned()
        } else {
            let index = Index::load(self);
            let entry = index.urls.get(url)?;
            if now.saturating_sub(entry.fetched) >= self.ttl.as_secs() {
                return None;
            }
            entry.sha256.clone()
        };
        if !is_sha256(&hash) {
            return None;
        }
        let text = fs::read_to_string(self.dir.join(&hash)).ok()?;
        (self::sha256(&text) == hash).then_some(text)
    }

    /// Keeps `text`, fetched from `url` at `now`. Errors are ignored: a cache that cannot
    /// be written only costs requests.
    fn put(&self, url: &str, text: &str, now: u64) {
        let hash = sha256(text);
        let file = self.dir.join(&hash);
        if !file.is_file() {
            write_atomic(&file, text.as_bytes());
        }
        let mut index = Index::load(self);
        index
            .urls
            .retain(|_, entry| now.saturating_sub(entry.fetched) < self.ttl.as_secs());
        index.urls.insert(
            url.to_owned(),
            Entry {
                sha256: hash,
                fetched: now,
            },
        );
        if let Ok(json) = serde_json::to_vec(&index) {
            write_atomic(&self.index_path(), &json);
        }
    }
}

/// The cache's `index.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    urls: BTreeMap<String, Entry>,
}

/// What a URL had, and when.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    sha256: String,
    /// Seconds since the epoch.
    fetched: u64,
}

impl Index {
    /// The index of `cache`; empty if it is missing or unreadable.
    fn load(cache: &IncludeCache) -> Self {
        fs::read(cache.index_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

/// Reads local files like [`FileLoader`] and fetches URLs over HTTP(S) with reqwest (TLS
/// through rustls with the ring provider, certificates verified by the operating
/// system), on a tokio runtime of its own; loading blocks until the content is there.
///
/// A URL is fetched with `GET`, following up to 10 redirects; a 2xx at the end gives the
/// body, which must be UTF-8. A 404 or 410 is [`NotFound`](io::ErrorKind::NotFound),
/// a timeout [`TimedOut`](io::ErrorKind::TimedOut). There are no retries. Proxies from
/// the environment (`HTTPS_PROXY`, `NO_PROXY`, …) are used. The client is set up on the
/// first fetch, so a loader that only reads files never starts a runtime.
#[derive(Debug, Default)]
pub struct HttpLoader {
    options: HttpLoaderOptions,
    client: OnceLock<Result<(tokio::runtime::Runtime, Client), String>>,
}

impl HttpLoader {
    /// A loader with `options`.
    pub fn new(options: HttpLoaderOptions) -> Self {
        Self {
            options,
            client: OnceLock::new(),
        }
    }

    /// The text of `target`, served from the cache if it may be.
    fn fetch(&self, target: &IncludeTarget, sha256: Option<&str>) -> io::Result<String> {
        let IncludeTarget::Url(url) = target else {
            return FileLoader.load(target);
        };
        let now = now();
        let cache = self.options.cache.as_ref();
        if let Some(text) = cache.and_then(|cache| cache.get(url, sha256, now)) {
            return Ok(text);
        }
        let text = String::from_utf8(self.get(url)?)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not valid UTF-8"))?;
        if let Some(cache) = cache {
            cache.put(url, &text, now);
        }
        Ok(text)
    }

    /// The body of `url`.
    fn get(&self, url: &str) -> io::Result<Vec<u8>> {
        let (runtime, client) = self
            .client
            .get_or_init(|| crate::http::client(&self.options.user_agent, self.options.timeout))
            .as_ref()
            .map_err(|message| {
                io::Error::other(format!("cannot set up the HTTP client: {message}"))
            })?;
        let error = |err: reqwest::Error| {
            let kind = if err.is_timeout() {
                io::ErrorKind::TimedOut
            } else {
                io::ErrorKind::Other
            };
            io::Error::new(kind, describe(&err))
        };
        runtime.block_on(async {
            let response = client.get(url).send().await.map_err(error)?;
            let status = response.status();
            if !status.is_success() {
                let kind = if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
                    io::ErrorKind::NotFound
                } else {
                    io::ErrorKind::Other
                };
                return Err(io::Error::new(kind, status.to_string()));
            }
            Ok(response.bytes().await.map_err(error)?.to_vec())
        })
    }
}

impl Loader for HttpLoader {
    fn load(&self, target: &IncludeTarget) -> io::Result<String> {
        self.fetch(target, None)
    }

    fn load_pinned(&self, target: &IncludeTarget, sha256: &str) -> io::Result<String> {
        self.fetch(target, Some(sha256))
    }
}

/// The SHA-256 of `text`, in lowercase hexadecimal digits.
pub(super) fn sha256(text: &str) -> String {
    use std::fmt::Write as _;
    let digest = ring::digest::digest(&ring::digest::SHA256, text.as_bytes());
    digest
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Whether `text` is a SHA-256 in lowercase hexadecimal digits.
fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A URL nothing listens on: a fetch fails (or is refused by a sandbox).
    const NOWHERE: &str = "http://127.0.0.1:9/a.md";

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256("A\n"),
            "06f961b802bc46ee168555f066d28f4f0e9afdf3f88174c1ee6f9de004fc30a0"
        );
        assert!(is_sha256(&sha256("")));
        assert!(!is_sha256(&sha256("").to_uppercase()));
    }

    #[test]
    fn the_cache_serves_pinned_content_by_hash_and_unpinned_content_within_the_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IncludeCache {
            dir: dir.path().join("remote-includes"),
            ttl: Duration::from_secs(100),
        };
        let pin = sha256("A\n");
        assert_eq!(cache.get(NOWHERE, None, 1_000), None);
        assert_eq!(cache.get(NOWHERE, Some(&pin), 1_000), None);
        cache.put(NOWHERE, "A\n", 1_000);
        assert_eq!(cache.get(NOWHERE, None, 1_099).as_deref(), Some("A\n"));
        assert_eq!(cache.get(NOWHERE, None, 1_100), None);
        // Pinned content does not expire, and does not depend on the URL.
        assert_eq!(
            cache.get("https://other", Some(&pin), 9_999).as_deref(),
            Some("A\n")
        );
        assert_eq!(cache.get(NOWHERE, Some(&sha256("B\n")), 1_000), None);
        // Expired entries are dropped from the index when it is written.
        cache.put("https://b", "B\n", 1_200);
        assert_eq!(
            Index::load(&cache).urls.keys().collect::<Vec<_>>(),
            ["https://b"]
        );

        // A file that does not match its name is not served, and an unreadable index is
        // empty.
        fs::write(cache.dir.join(&pin), "changed\n").unwrap();
        assert_eq!(cache.get(NOWHERE, Some(&pin), 1_000), None);
        fs::write(cache.index_path(), "not json").unwrap();
        assert!(Index::load(&cache).urls.is_empty());
    }

    #[test]
    fn a_loader_serves_pinned_content_from_the_cache_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IncludeCache {
            dir: dir.path().to_owned(),
            ttl: Duration::ZERO,
        };
        cache.put(NOWHERE, "A\n", 1_000);
        let loader = HttpLoader::new(HttpLoaderOptions {
            timeout: Duration::from_millis(200),
            cache: Some(cache),
            ..HttpLoaderOptions::default()
        });
        let target = IncludeTarget::Url(NOWHERE.to_owned());
        assert_eq!(loader.load_pinned(&target, &sha256("A\n")).unwrap(), "A\n");
        assert!(loader.load(&target).is_err());
        assert!(loader.load_pinned(&target, &sha256("B\n")).is_err());
    }

    #[test]
    fn local_files_are_read_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, "A\n").unwrap();
        let loader = HttpLoader::default();
        assert_eq!(loader.load(&IncludeTarget::File(path)).unwrap(), "A\n");
        assert!(loader.client.get().is_none());
    }
}
