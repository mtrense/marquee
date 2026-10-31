//! The Kroki renderer (feature `kroki`) against a local mock Kroki server, a
//! `std::net::TcpListener` on `127.0.0.1` answering by path.
#![cfg(feature = "kroki")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mrq_core::config::{DiagramsConfig, KrokiConfig};
use mrq_render::diagram::{
    DiagramCache, DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer, KrokiRenderer,
    Registry,
};

/// A request: method, path, `Content-Type` and body.
type Request = (String, String, String, String);

/// A mock Kroki server on a free local port, recording every request.
///
/// `/svgbob/svg` and `/graphviz/svg` answer `<svg>` around the body, `/plantuml/txt`
/// the body in a box, `/mermaid/svg` a 400 with Kroki's error text, `/slow/svg` after a
/// second, and anything else 404.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
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

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

/// Answers the requests of one connection (keep-alive).
fn respond(stream: TcpStream, log: &Mutex<Vec<Request>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut stream = stream;
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        let (mut length, mut content_type) = (0, String::new());
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                return;
            }
            if header == "\r\n" || header == "\n" {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => length = value.trim().parse().unwrap(),
                    "content-type" => value.trim().clone_into(&mut content_type),
                    _ => {}
                }
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let body = String::from_utf8(body).unwrap();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        log.lock()
            .unwrap()
            .push((method, path.clone(), content_type, body.clone()));
        let (status, reply) = match path.as_str() {
            "/svgbob/svg" | "/graphviz/svg" => ("200 OK", format!("<svg>{body}</svg>")),
            "/plantuml/txt" => ("200 OK", format!("[{body}]")),
            "/mermaid/svg" => (
                "400 Bad Request",
                "Error 400: Syntax error in graph mermaid version 10".to_owned(),
            ),
            "/slow/svg" => {
                thread::sleep(Duration::from_secs(1));
                ("200 OK", "<svg/>".to_owned())
            }
            _ => ("404 Not Found", String::new()),
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{reply}",
            reply.len()
        );
        if stream.write_all(response.as_bytes()).is_err() {
            return;
        }
    }
}

fn kroki_config(server: &Server, enabled: bool) -> DiagramsConfig {
    DiagramsConfig {
        kroki: KrokiConfig {
            enabled,
            url: format!("{}/", server.base),
        },
        ..DiagramsConfig::default()
    }
}

#[test]
fn kroki_renders_by_posting_the_source() {
    let server = Server::start();
    let kroki = KrokiRenderer::new(&server.base).unwrap();
    assert_eq!(
        kroki
            .render("svgbob", "a --> b", DiagramFormat::Svg)
            .unwrap(),
        DiagramOutput::Svg("<svg>a --> b</svg>".to_owned())
    );
    assert_eq!(
        kroki
            .render("dot", "digraph {}", DiagramFormat::Svg)
            .unwrap(),
        DiagramOutput::Svg("<svg>digraph {}</svg>".to_owned()),
        "dot is graphviz"
    );
    assert_eq!(
        kroki
            .render("plantuml", "A -> B", DiagramFormat::Text)
            .unwrap(),
        DiagramOutput::Text("[A -> B]".to_owned())
    );
    let requests = server.requests();
    assert_eq!(requests[0].0, "POST");
    assert_eq!(requests[0].1, "/svgbob/svg");
    assert!(requests[0].2.starts_with("text/plain"), "{:?}", requests[0]);
    assert_eq!(requests[1].1, "/graphviz/svg");
    assert_eq!(requests[2].1, "/plantuml/txt");
    assert_eq!(kroki.formats("svgbob"), [DiagramFormat::Svg]);
    assert!(kroki.formats("plantuml").contains(&DiagramFormat::Text));
}

#[test]
fn kroki_errors_carry_the_status_and_message() {
    let server = Server::start();
    let kroki = KrokiRenderer::with_timeout(&server.base, Duration::from_millis(200)).unwrap();
    let failed = |lang: &str| match kroki.render(lang, "x", DiagramFormat::Svg) {
        Err(DiagramError::Failed(message)) => message,
        other => panic!("{lang}: {other:?}"),
    };
    let syntax = failed("mermaid");
    assert!(syntax.contains("400"), "{syntax}");
    assert!(syntax.contains("Syntax error in graph"), "{syntax}");
    assert!(failed("slow").contains("timed out"));
    assert!(failed("nomnoml").contains("404"));
}

#[test]
fn kroki_is_contacted_only_when_enabled() {
    let server = Server::start();
    let disabled = Registry::from_config(&kroki_config(&server, false));
    assert!(
        disabled
            .render("svgbob", "-->", &[DiagramFormat::Svg])
            .is_none()
    );
    assert_eq!(server.requests(), Vec::<Request>::new());
    let enabled = Registry::from_config(&kroki_config(&server, true));
    let rendered = enabled
        .render("svgbob", "-->", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert_eq!(rendered.renderer, "kroki");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn kroki_renders_are_cached_per_url() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let registry = || {
        Registry::from_config(&kroki_config(&server, true))
            .with_cache(DiagramCache::new(dir.path()))
    };
    assert!(
        !registry()
            .render("svgbob", "-->", &[DiagramFormat::Svg])
            .unwrap()
            .unwrap()
            .cached
    );
    assert!(
        registry()
            .render("svgbob", "-->", &[DiagramFormat::Svg])
            .unwrap()
            .unwrap()
            .cached
    );
    assert_eq!(server.requests().len(), 1);
    let other = Server::start();
    let elsewhere = Registry::from_config(&kroki_config(&other, true))
        .with_cache(DiagramCache::new(dir.path()));
    assert!(
        !elsewhere
            .render("svgbob", "-->", &[DiagramFormat::Svg])
            .unwrap()
            .unwrap()
            .cached
    );
    assert_eq!(other.requests().len(), 1);
}
