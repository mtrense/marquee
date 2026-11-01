//! `mrq lsp`: the language server on stdio. Its behaviour is tested in mrq-lsp; this
//! checks the wiring: JSON-RPC over stdin/stdout and the exit code.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const INITIALIZE: &str =
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
const SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#;
const EXIT: &str = r#"{"jsonrpc":"2.0","method":"exit"}"#;

/// A running `mrq lsp` with `args`, spoken to like a client: one request at a time,
/// waiting for its response (a server may drop requests still queued at `exit`).
struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Server {
    fn start(args: &[&str]) -> Self {
        let mut child = Command::new(assert_cmd::cargo::cargo_bin("mrq"))
            .arg("lsp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    /// Writes `body` with its `Content-Length` header.
    fn send(&mut self, body: &str) {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    /// Reads the next message, checking its framing.
    fn receive(&mut self) -> String {
        let mut length = None;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
            assert!(line.ends_with("\r\n"), "bad header line {line:?}");
            match line.trim_end().split_once(": ") {
                Some(("Content-Length", value)) => length = Some(value.parse().unwrap()),
                Some(_) => {}
                None => break,
            }
        }
        let mut body = vec![0; length.expect("no Content-Length")];
        self.stdout.read_exact(&mut body).unwrap();
        String::from_utf8(body).unwrap()
    }

    /// Sends `exit` and returns the exit code.
    fn exit(mut self) -> Option<i32> {
        self.send(EXIT);
        self.child.wait().unwrap().code()
    }
}

#[test]
fn lsp_speaks_json_rpc_on_stdio() {
    let mut server = Server::start(&[]);
    server.send(INITIALIZE);
    let response = server.receive();
    assert!(response.contains(r#""id":1"#), "{response}");
    assert!(response.contains(r#""name":"mrq""#), "{response}");
    server.send(INITIALIZED);
    server.send(SHUTDOWN);
    let response = server.receive();
    assert!(response.contains(r#""id":2"#), "{response}");
    assert_eq!(server.exit(), Some(0));
}

#[test]
fn lsp_accepts_the_stdio_flag() {
    let mut server = Server::start(&["--stdio"]);
    server.send(INITIALIZE);
    server.receive();
    server.send(SHUTDOWN);
    server.receive();
    assert_eq!(server.exit(), Some(0));
}

#[test]
fn exit_without_shutdown_exits_1() {
    let mut server = Server::start(&[]);
    server.send(INITIALIZE);
    server.receive();
    assert_eq!(server.exit(), Some(1));
}
