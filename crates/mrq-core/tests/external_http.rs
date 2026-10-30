//! Acceptance tests for `HttpChecker` (feature `external-links`) against a local mock HTTP
//! server: a `std::net::TcpListener` on `127.0.0.1` answering by path.
#![cfg(feature = "external-links")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mrq_core::diag::Diagnostic;
use mrq_core::document::{Document, DocumentOptions, Workspace};
use mrq_core::source::SourceFile;
use mrq_core::validate::{
    CacheOptions, Checker, ExternalChecker, ExternalExclude, ExternalStatus, HttpChecker,
    HttpOptions, codes,
};

/// A mock HTTP server on a free local port, recording the path of every request.
///
/// `/ok` is 200, `/missing` 404, `/moved` redirects to `/ok`, `/flaky` is 503 on its
/// first request and 200 afterwards, and anything else 500.
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

    fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|p| *p == path).count()
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
        let seen = {
            let mut log = log.lock().unwrap();
            log.push(path.clone());
            log.iter().filter(|p| **p == path).count()
        };
        let (status, extra) = match path.as_str() {
            "/missing" => ("404 Not Found", String::new()),
            "/moved" => ("301 Moved Permanently", "Location: /ok\r\n".to_owned()),
            "/flaky" if seen == 1 => ("503 Service Unavailable", String::new()),
            "/ok" | "/flaky" => ("200 OK", String::new()),
            _ => ("500 Internal Server Error", String::new()),
        };
        let response = format!("HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\n\r\n");
        if stream.write_all(response.as_bytes()).is_err() {
            return;
        }
    }
}

/// Fast options for tests: short retry waits and timeouts, no cache.
fn options() -> HttpOptions {
    HttpOptions {
        retry_wait: Duration::from_millis(10),
        timeout: Duration::from_secs(5),
        host_request_interval: Duration::ZERO,
        ..HttpOptions::default()
    }
}

fn doc_at(path: &str, text: &str) -> Document {
    let source = SourceFile::new(Some(PathBuf::from(path)), text);
    DocumentOptions::new().parse_source(source).unwrap()
}

#[test]
fn statuses_follow_the_responses() {
    let server = Server::start();
    let checker = HttpChecker::new(options()).unwrap();
    let urls = [
        server.url("/ok"),
        server.url("/missing"),
        server.url("/moved"),
        server.url("/ok#section"),
    ];
    let urls: Vec<&str> = urls.iter().map(String::as_str).collect();
    let statuses = checker.check(&urls);
    assert_eq!(statuses[0], ExternalStatus::Ok);
    assert!(
        matches!(&statuses[1], ExternalStatus::Broken(reason) if reason.contains("404")),
        "{:?}",
        statuses[1]
    );
    assert_eq!(statuses[2], ExternalStatus::Ok);
    assert_eq!(statuses[3], ExternalStatus::Ok);
}

#[test]
fn failed_requests_are_retried() {
    let server = Server::start();
    let checker = HttpChecker::new(options()).unwrap();
    assert_eq!(
        checker.check(&[&server.url("/flaky")]),
        [ExternalStatus::Ok]
    );
    assert_eq!(server.count("/flaky"), 2);

    let server = Server::start();
    let checker = HttpChecker::new(HttpOptions {
        max_retries: 0,
        ..options()
    })
    .unwrap();
    let statuses = checker.check(&[&server.url("/flaky")]);
    assert!(
        matches!(&statuses[0], ExternalStatus::Broken(_)),
        "{statuses:?}"
    );
    assert_eq!(server.count("/flaky"), 1);
}

#[test]
fn working_urls_are_cached_on_disk_until_the_ttl_expires() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let cache = CacheOptions {
        path: dir.path().join("external-links.json"),
        ttl: Duration::from_secs(3600),
    };
    let cached = || HttpOptions {
        cache: Some(cache.clone()),
        ..options()
    };
    let (ok, missing) = (server.url("/ok"), server.url("/missing"));
    let urls = [ok.as_str(), missing.as_str()];

    let first = HttpChecker::new(cached()).unwrap().check(&urls);
    assert_eq!(first[0], ExternalStatus::Ok);
    assert!(cache.path.is_file());
    // A new checker (a new run) reads the cache: only the broken URL is requested again.
    let second = HttpChecker::new(cached()).unwrap().check(&urls);
    assert_eq!(first, second);
    assert_eq!(server.count("/ok"), 1);
    assert_eq!(server.count("/missing"), 2);

    // Expired entries are checked again.
    let expired = HttpOptions {
        cache: Some(CacheOptions {
            ttl: Duration::ZERO,
            ..cache.clone()
        }),
        ..options()
    };
    HttpChecker::new(expired).unwrap().check(&urls);
    assert_eq!(server.count("/ok"), 2);
}

#[test]
fn the_checker_reports_broken_links_through_the_http_checker() {
    let server = Server::start();
    let ws = Workspace::new("site");
    let text = format!(
        "[ok]({}) [gone]({}) [skip]({})\n",
        server.url("/ok"),
        server.url("/missing"),
        server.url("/excluded"),
    );
    let doc = doc_at("site/a.md", &text);
    let http = HttpChecker::new(options()).unwrap();
    let exclude = ExternalExclude::new(["/excluded$"]).unwrap();
    let diagnostics = Checker::new(&ws)
        .with_external_links(&http, &exclude)
        .check(&doc);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, codes::BROKEN_EXTERNAL_LINK);
    let source = &doc.text()[diagnostics[0].range.start..diagnostics[0].range.end];
    assert!(source.starts_with("[gone]("), "{source}");
    let mut requests = server.requests();
    requests.sort();
    assert_eq!(requests, ["/missing", "/ok"]);
}

#[test]
fn nothing_hits_the_network_by_default() {
    let server = Server::start();
    let ws = Workspace::new("site");
    let doc = doc_at(
        "site/a.md",
        &format!(
            "[gone]({}) ![img]({})\n",
            server.url("/missing"),
            server.url("/ok")
        ),
    );
    // Building an `HttpChecker` sends nothing, and a checker without one checks nothing.
    let _http = HttpChecker::new(options()).unwrap();
    assert_eq!(Checker::new(&ws).check(&doc), Vec::<Diagnostic>::new());
    let mut ws = ws;
    ws.insert(doc);
    assert_eq!(
        Checker::new(&ws).check_workspace(),
        Vec::<Diagnostic>::new()
    );
    assert_eq!(server.requests(), Vec::<String>::new());
}
