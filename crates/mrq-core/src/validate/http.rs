//! [`HttpChecker`]: the [`ExternalChecker`] over HTTP(S), built on reqwest with rustls
//! (feature `external-links`), with its on-disk cache.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::stream::{self, StreamExt};
use reqwest::{Client, StatusCode, Url, redirect};
use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, SemaphorePermit};

use super::{ExternalChecker, ExternalStatus};
use crate::config::user_cache_dir;

/// How an [`HttpChecker`] checks: retries, timeouts, rate and concurrency limits, and the
/// cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpOptions {
    /// Retries of a failed request (connection errors, timeouts, 408, 429, 5xx), waiting
    /// [`retry_wait`](Self::retry_wait) before the first and doubling the wait for each
    /// further one. Default 3.
    pub max_retries: u64,
    /// The wait before the first retry. Default 1 s.
    pub retry_wait: Duration,
    /// The timeout of each request, redirects included. Default 20 s.
    pub timeout: Duration,
    /// Requests in flight at once, over all hosts. Default 32.
    pub max_concurrency: usize,
    /// Requests in flight at once to one host. Default 4.
    pub host_concurrency: usize,
    /// The minimum time between two requests to one host. Default 50 ms.
    pub host_request_interval: Duration,
    /// The `User-Agent` header. Default `mrq/<version>`.
    pub user_agent: String,
    /// Where and how long working URLs are remembered; `None` (the default) for no cache.
    pub cache: Option<CacheOptions>,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_wait: Duration::from_secs(1),
            timeout: Duration::from_secs(20),
            max_concurrency: 32,
            host_concurrency: 4,
            host_request_interval: Duration::from_millis(50),
            user_agent: concat!("mrq/", env!("CARGO_PKG_VERSION")).to_owned(),
            cache: None,
        }
    }
}

/// The on-disk cache of an [`HttpChecker`]: a JSON file of the URLs found working and
/// when. A URL found working less than `ttl` ago is not requested again; broken URLs are
/// not cached, so a fixed link is seen on the next run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheOptions {
    /// The cache file; its directory is created when the cache is written.
    pub path: PathBuf,
    /// How long a working URL is trusted.
    pub ttl: Duration,
}

impl CacheOptions {
    /// The default time to live: one day.
    pub const DEFAULT_TTL: Duration = Duration::from_hours(24);

    /// `external-links.json` in the [user cache directory](user_cache_dir) with the
    /// [default TTL](Self::DEFAULT_TTL); `None` if there is no such directory.
    pub fn in_user_cache_dir() -> Option<Self> {
        let dir = user_cache_dir(|key| std::env::var(key).ok())?;
        Some(Self {
            path: dir.join("external-links.json"),
            ttl: Self::DEFAULT_TTL,
        })
    }
}

/// The cache file's contents.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    /// Working URLs, with when they were last found working (seconds since the epoch).
    ok: BTreeMap<String, u64>,
}

impl CacheFile {
    /// The cache at `path`; empty if it is missing or unreadable.
    fn load(options: &CacheOptions) -> Self {
        fs::read(&options.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Whether `url` was found working within `ttl` before `now`.
    fn is_fresh(&self, url: &str, now: u64, ttl: Duration) -> bool {
        self.ok
            .get(url)
            .is_some_and(|&checked| now.saturating_sub(checked) < ttl.as_secs())
    }

    /// Writes the cache, without the entries expired at `now`. Errors are ignored: a
    /// cache that cannot be written only costs requests.
    fn save(mut self, options: &CacheOptions, now: u64) {
        self.ok
            .retain(|_, checked| now.saturating_sub(*checked) < options.ttl.as_secs());
        let Ok(json) = serde_json::to_vec(&self) else {
            return;
        };
        if let Some(dir) = options.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // Write a sibling and rename it, so a concurrent run never reads half a file.
        let mut temp = options.path.clone().into_os_string();
        temp.push(format!(".{}.tmp", std::process::id()));
        if fs::write(&temp, json).is_ok() && fs::rename(&temp, &options.path).is_err() {
            let _ = fs::remove_file(&temp);
        }
    }
}

/// A client that cannot be built: the runtime or TLS setup failed.
#[derive(Debug, thiserror::Error)]
#[error("cannot set up the HTTP client: {message}")]
pub struct HttpError {
    message: String,
}

/// The most redirects followed for one URL.
const MAX_REDIRECTS: usize = 10;

/// Checks external URLs over HTTP(S) with reqwest (TLS through rustls with the ring
/// provider, certificates verified by the operating system), on a tokio runtime of its
/// own; [`check`](ExternalChecker::check) blocks until all URLs are done.
///
/// Each URL is requested with `GET` (the body is not read) without its fragment, which
/// is not checked. Up to 10 redirects are followed; a 2xx at the end means working,
/// anything else is [`Broken`](ExternalStatus::Broken). Proxies from the environment
/// (`HTTPS_PROXY`, `NO_PROXY`, …) are used. Building a checker sends no request.
#[derive(Debug)]
pub struct HttpChecker {
    client: Client,
    runtime: tokio::runtime::Runtime,
    options: HttpOptions,
    /// Concurrency and rate limits per host (`host:port`).
    hosts: Mutex<HashMap<String, Arc<Host>>>,
}

/// The limits of one host.
#[derive(Debug)]
struct Host {
    permits: Semaphore,
    /// When the next request to the host may start.
    next: Mutex<Instant>,
}

impl Host {
    /// Waits for a free slot and the host's request interval; the slot is held as long
    /// as the permit.
    async fn turn(&self, interval: Duration) -> Option<SemaphorePermit<'_>> {
        let permit = self.permits.acquire().await.ok();
        let wait = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let start = (*next).max(now);
            *next = start + interval;
            start - now
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        permit
    }
}

impl HttpChecker {
    /// A checker with `options`.
    pub fn new(options: HttpOptions) -> Result<Self, HttpError> {
        let error = |message: String| HttpError { message };
        // reqwest takes the process-wide rustls provider; ring, unless one is installed.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| error(err.to_string()))?;
        let client = Client::builder()
            .user_agent(&options.user_agent)
            .timeout(options.timeout)
            .redirect(redirect::Policy::limited(MAX_REDIRECTS))
            .build()
            .map_err(|err| error(describe(&err)))?;
        Ok(Self {
            client,
            runtime,
            options,
            hosts: Mutex::new(HashMap::new()),
        })
    }

    /// Requests each of `urls`, at most `max_concurrency` at once.
    fn request(&self, urls: &[&str]) -> Vec<ExternalStatus> {
        let concurrency = self.options.max_concurrency.max(1);
        self.runtime.block_on(
            stream::iter(urls.iter().map(|url| self.request_one(url)))
                .buffered(concurrency)
                .collect(),
        )
    }

    /// Requests `url`, retrying with exponential backoff while the failure may pass.
    async fn request_one(&self, url: &str) -> ExternalStatus {
        let mut url = match Url::parse(url) {
            Ok(url) => url,
            Err(err) => return ExternalStatus::Broken(format!("invalid URL: {err}")),
        };
        url.set_fragment(None);
        let host = self.host(&url);
        let mut wait = self.options.retry_wait;
        let mut retries = 0;
        loop {
            let response = {
                let _permit = host.turn(self.options.host_request_interval).await;
                self.client.get(url.clone()).send().await
            };
            let (status, transient) = match response {
                Ok(response) if response.status().is_success() => return ExternalStatus::Ok,
                Ok(response) => (response.status().to_string(), retryable(response.status())),
                Err(err) => (describe(&err), err.is_timeout() || err.is_connect()),
            };
            if !transient || retries >= self.options.max_retries {
                return ExternalStatus::Broken(status);
            }
            retries += 1;
            tokio::time::sleep(wait).await;
            wait = wait.saturating_mul(2);
        }
    }

    /// The limits of `url`'s host.
    fn host(&self, url: &Url) -> Arc<Host> {
        let key = format!(
            "{}:{}",
            url.host_str().unwrap_or_default(),
            url.port_or_known_default().unwrap_or_default()
        );
        let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        let host = hosts.entry(key).or_insert_with(|| {
            Arc::new(Host {
                permits: Semaphore::new(self.options.host_concurrency.max(1)),
                next: Mutex::new(Instant::now()),
            })
        });
        Arc::clone(host)
    }
}

impl ExternalChecker for HttpChecker {
    fn check(&self, urls: &[&str]) -> Vec<ExternalStatus> {
        let Some(options) = &self.options.cache else {
            return self.request(urls);
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let mut cache = CacheFile::load(options);
        let stale: Vec<&str> = urls
            .iter()
            .copied()
            .filter(|url| !cache.is_fresh(url, now, options.ttl))
            .collect();
        let requested: HashMap<&str, ExternalStatus> =
            stale.iter().copied().zip(self.request(&stale)).collect();
        for (url, status) in &requested {
            if *status == ExternalStatus::Ok {
                cache.ok.insert((*url).to_owned(), now);
            }
        }
        cache.save(options, now);
        // Fresh in the cache, hence working, if not requested.
        urls.iter()
            .map(|url| requested.get(url).cloned().unwrap_or(ExternalStatus::Ok))
            .collect()
    }
}

/// Whether a response with `status` may succeed when asked again.
fn retryable(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
}

/// A short reason for `err`, without the URL (the diagnostic names it).
fn describe(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "timed out".to_owned();
    }
    if err.is_redirect() {
        return format!("more than {MAX_REDIRECTS} redirects");
    }
    let mut cause: &dyn Error = err;
    while let Some(source) = cause.source() {
        cause = source;
    }
    if err.is_connect() {
        format!("connection failed: {cause}")
    } else {
        cause.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_errors_and_rate_limits_are_retried() {
        for code in [500, 502, 503, 408, 429] {
            assert!(retryable(StatusCode::from_u16(code).unwrap()), "{code}");
        }
        for code in [400, 401, 403, 404, 410] {
            assert!(!retryable(StatusCode::from_u16(code).unwrap()), "{code}");
        }
        assert_eq!(StatusCode::NOT_FOUND.to_string(), "404 Not Found");
    }

    #[test]
    fn invalid_urls_are_broken_without_a_request() {
        let checker = HttpChecker::new(HttpOptions::default()).unwrap();
        let statuses = checker.check(&["http://"]);
        assert!(
            matches!(&statuses[0], ExternalStatus::Broken(reason) if reason.starts_with("invalid URL")),
            "{statuses:?}"
        );
    }

    #[test]
    fn the_cache_keeps_fresh_entries_and_drops_expired_ones() {
        let dir = tempfile::tempdir().unwrap();
        let options = CacheOptions {
            path: dir.path().join("sub/external-links.json"),
            ttl: Duration::from_secs(100),
        };
        // Missing and unreadable files are empty caches.
        assert!(CacheFile::load(&options).ok.is_empty());
        let mut cache = CacheFile::default();
        cache.ok.insert("https://old".into(), 1_000);
        cache.ok.insert("https://new".into(), 1_050);
        cache.save(&options, 1_120);
        let cache = CacheFile::load(&options);
        assert_eq!(cache.ok.keys().collect::<Vec<_>>(), ["https://new"]);
        assert!(cache.is_fresh("https://new", 1_149, options.ttl));
        assert!(!cache.is_fresh("https://new", 1_150, options.ttl));
        assert!(!cache.is_fresh("https://other", 1_100, options.ttl));

        fs::write(&options.path, "not json").unwrap();
        assert!(CacheFile::load(&options).ok.is_empty());
    }
}
