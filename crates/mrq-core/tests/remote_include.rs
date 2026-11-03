//! Acceptance tests for remote includes (feature `remote-include`): `HttpLoader` and the
//! `Includer` against a local mock HTTP server, a `std::net::TcpListener` on `127.0.0.1`
//! answering by path.
#![cfg(feature = "remote-include")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mrq_core::diag::Diagnostic;
use mrq_core::document::{Document, DocumentOptions};
use mrq_core::source::SourceFile;
use mrq_core::transform::{
    Expanded, HttpLoader, HttpLoaderOptions, INCLUDE_CODE, IncludeCache, Includer,
};
use mrq_core::validate::codes;

/// SHA-256 of `Part\n`, the body of `/docs/part.md`.
const PART_SHA256: &str = "1e2315908e1fe3cc0b0bb744cb45f26b5601d0804642191f95128d20c4206d73";

/// A mock HTTP server on a free local port, recording the path of every request.
///
/// `/docs/guide.md` links to `next.md` and includes `part.md`, `/docs/part.md` is
/// `Part\n`, `/moved.md` redirects to `/docs/part.md`, `/missing.md` is 404,
/// `/slow.md` answers after a second, `/latin1.md` is not UTF-8, and anything else 500.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let log = Arc::clone(&log);
                thread::spawn(move || respond(stream, &log));
            }
        });
        Self { base, requests }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The paths requested so far.
    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// Answers the requests of one connection (keep-alive).
fn respond(stream: TcpStream, log: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut stream = stream;
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        // Skip the headers; requests carry no body.
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                return;
            }
            if header == "\r\n" || header == "\n" {
                break;
            }
        }
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        log.lock().unwrap().push(path.clone());
        let ok = "200 OK";
        let (status, extra, body): (&str, &str, &[u8]) = match path.as_str() {
            "/docs/guide.md" => (
                ok,
                "",
                b"# Guide\n\n[next](next.md)\n\n<!-- mrq:include path=part.md -->\n",
            ),
            "/docs/part.md" => (ok, "", b"Part\n"),
            "/moved.md" => ("301 Moved Permanently", "Location: /docs/part.md\r\n", b""),
            "/missing.md" => ("404 Not Found", "", b""),
            "/slow.md" => {
                thread::sleep(Duration::from_secs(1));
                (ok, "", b"Slow\n")
            }
            "/latin1.md" => (ok, "", b"caf\xe9\n"),
            _ => ("500 Internal Server Error", "", b""),
        };
        let head = format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\n\r\n",
            body.len()
        );
        if stream.write_all(head.as_bytes()).is_err() || stream.write_all(body).is_err() {
            return;
        }
    }
}

/// Fast options for tests: a short timeout, no cache.
fn options() -> HttpLoaderOptions {
    HttpLoaderOptions {
        timeout: Duration::from_millis(500),
        ..HttpLoaderOptions::default()
    }
}

fn host(text: &str) -> Document {
    let source = SourceFile::new(Some(PathBuf::from("host.md")), text);
    DocumentOptions::new().parse_source(source).unwrap()
}

/// `text` (in `host.md`) expanded with remote includes allowed, fetched with `options`.
fn expand(text: &str, options: HttpLoaderOptions) -> Expanded {
    Includer::new()
        .allow_remote(true)
        .loader(HttpLoader::new(options))
        .expand(&host(text))
}

fn include(url: &str) -> String {
    format!("<!-- mrq:include path=\"{url}\" -->\n")
}

fn codes_and_messages(diagnostics: &[Diagnostic]) -> Vec<(&str, &str)> {
    diagnostics
        .iter()
        .map(|d| (d.code.as_str(), d.message.as_str()))
        .collect()
}

#[test]
fn remote_content_is_fetched_with_its_links_and_includes_resolved_against_the_url() {
    let server = Server::start();
    let expanded = expand(&include(&server.url("/docs/guide.md")), options());
    assert_eq!(expanded.diagnostics, []);
    assert_eq!(
        expanded.text,
        format!(
            "# Guide\n\n[next]({})\n\nPart\n",
            server.url("/docs/next.md")
        )
    );
    assert_eq!(server.requests(), ["/docs/guide.md", "/docs/part.md"]);

    // Redirects are followed.
    let expanded = expand(&include(&server.url("/moved.md")), options());
    assert_eq!(expanded.diagnostics, []);
    assert_eq!(expanded.text, "Part\n");
}

#[test]
fn nothing_is_fetched_unless_remote_includes_are_allowed() {
    let server = Server::start();
    let text = include(&server.url("/docs/part.md"));
    let expanded = Includer::new()
        .loader(HttpLoader::new(options()))
        .expand(&host(&text));
    assert_eq!(expanded.text, text);
    assert_eq!(expanded.diagnostics.len(), 1);
    assert!(
        expanded.diagnostics[0].message.ends_with("is not allowed"),
        "{:?}",
        expanded.diagnostics
    );
    // Nor by the default loader.
    Includer::new().expand(&host(&text));
    assert_eq!(server.requests(), Vec::<String>::new());
}

#[test]
fn failed_fetches_are_reported_and_leave_the_include() {
    let server = Server::start();
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/a.md", listener.local_addr().unwrap())
    };
    let urls = [
        server.url("/missing.md"),
        server.url("/broken.md"),
        server.url("/slow.md"),
        server.url("/latin1.md"),
        closed.clone(),
    ];
    let text: Vec<String> = urls.iter().map(|url| include(url)).collect();
    let text = text.join("\n");
    let expanded = expand(&text, options());
    assert_eq!(expanded.text, text);
    let messages = codes_and_messages(&expanded.diagnostics);
    assert_eq!(
        messages[..4],
        [
            (
                codes::MISSING_INCLUDE,
                &*format!("included URL `{}` does not exist", urls[0])
            ),
            (
                INCLUDE_CODE,
                &*format!("cannot include `{}`: 500 Internal Server Error", urls[1])
            ),
            (
                INCLUDE_CODE,
                &*format!("cannot include `{}`: timed out", urls[2])
            ),
            (
                INCLUDE_CODE,
                &*format!("cannot include `{}`: not valid UTF-8", urls[3])
            ),
        ]
    );
    let (code, refused) = messages[4];
    assert_eq!(code, INCLUDE_CODE);
    // Windows retries refused connections for seconds, past the test's short timeout.
    assert!(
        refused.starts_with(&format!("cannot include `{closed}`: connection failed"))
            || (cfg!(windows) && refused == format!("cannot include `{closed}`: timed out")),
        "{refused}"
    );
}

#[test]
fn fetched_content_is_cached_on_disk_until_the_ttl_expires() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let cached = |ttl| HttpLoaderOptions {
        cache: Some(IncludeCache {
            dir: dir.path().join("remote-includes"),
            ttl,
        }),
        ..options()
    };
    let text = include(&server.url("/docs/part.md"));
    let hour = Duration::from_secs(3600);
    assert_eq!(expand(&text, cached(hour)).text, "Part\n");
    // A new loader (a new run) reads the cache.
    assert_eq!(expand(&text, cached(hour)).text, "Part\n");
    assert_eq!(server.requests(), ["/docs/part.md"]);
    // Expired entries are fetched again.
    assert_eq!(expand(&text, cached(Duration::ZERO)).text, "Part\n");
    assert_eq!(server.requests(), ["/docs/part.md", "/docs/part.md"]);
}

#[test]
fn pinned_content_is_checked_and_served_from_the_cache() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    // Unpinned content is never trusted beyond the TTL, pinned content always.
    let cached = || HttpLoaderOptions {
        cache: Some(IncludeCache {
            dir: dir.path().to_owned(),
            ttl: Duration::ZERO,
        }),
        ..options()
    };
    let url = server.url("/docs/part.md");
    let pinned = format!("<!-- mrq:include path=\"{url}\" sha256={PART_SHA256} -->\n");
    for _ in 0..2 {
        let expanded = expand(&pinned, cached());
        assert_eq!(expanded.diagnostics, []);
        assert_eq!(expanded.text, "Part\n");
    }
    assert_eq!(server.requests(), ["/docs/part.md"]);

    let wrong = "0".repeat(64);
    let text = format!("<!-- mrq:include path=\"{url}\" sha256={wrong} -->\n");
    let expanded = expand(&text, cached());
    assert_eq!(expanded.text, text);
    assert_eq!(
        codes_and_messages(&expanded.diagnostics),
        [(
            INCLUDE_CODE,
            &*format!("the content of `{url}` does not match its `sha256`: it is `{PART_SHA256}`")
        )]
    );
    assert_eq!(server.requests().len(), 2);
}
