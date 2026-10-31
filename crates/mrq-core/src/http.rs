//! What the HTTP clients of the `external-links` and `remote-include` features share:
//! the reqwest + rustls client on a tokio runtime of its own, error descriptions, and
//! atomic cache writes.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{Client, redirect};

/// The most redirects followed for one URL.
pub(crate) const MAX_REDIRECTS: usize = 10;

/// A current-thread tokio runtime and a client on it with `user_agent` and `timeout` (for
/// each request, redirects included), following up to [`MAX_REDIRECTS`] redirects; the
/// reason if either cannot be set up.
pub(crate) fn client(
    user_agent: &str,
    timeout: Duration,
) -> Result<(tokio::runtime::Runtime, Client), String> {
    // reqwest takes the process-wide rustls provider; ring, unless one is installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    let client = Client::builder()
        .user_agent(user_agent)
        .timeout(timeout)
        .redirect(redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .map_err(|err| describe(&err))?;
    Ok((runtime, client))
}

/// A short reason for `err`, without the URL (the diagnostic names it).
pub(crate) fn describe(err: &reqwest::Error) -> String {
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

/// Seconds since the epoch.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Writes `bytes` to `path`, creating its directory. A sibling is written and renamed, so
/// a concurrent run never reads half a file. Errors are ignored: a cache that cannot be
/// written only costs requests.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let mut temp = path.to_owned().into_os_string();
    temp.push(format!(".{}.tmp", std::process::id()));
    if fs::write(&temp, bytes).is_ok() && fs::rename(&temp, path).is_err() {
        let _ = fs::remove_file(&temp);
    }
}
