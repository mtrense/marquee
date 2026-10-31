//! `mrq show` up to the point where it needs a terminal: input and theme errors, and the
//! refusal to start without a terminal on stdout. The TUI itself is tested in mrq-tui.
//!
//! Every test runs `mrq` in a temp directory with `MRQ_CONFIG_HOME` pointing at an empty
//! one, so no user config leaks in.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

struct Mrq {
    dir: tempfile::TempDir,
    user: tempfile::TempDir,
}

impl Mrq {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("guide.md"), "# Guide\n\nText.\n").unwrap();
        fs::create_dir(dir.path().join("empty")).unwrap();
        Self {
            dir,
            user: tempfile::tempdir().unwrap(),
        }
    }

    fn show(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        Command::cargo_bin("mrq")
            .unwrap()
            .current_dir(self.dir.path())
            .env("MRQ_CONFIG_HOME", self.user.path())
            .arg("show")
            .args(args)
            .assert()
    }
}

#[test]
fn needs_an_input() {
    Mrq::new()
        .show(&[])
        .code(2)
        .stderr(predicate::str::contains("no input given"));
}

#[test]
fn reports_missing_inputs() {
    Mrq::new()
        .show(&["guide.md", "gone.md"])
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("gone.md"));
}

#[test]
fn reports_a_directory_without_markdown() {
    Mrq::new()
        .show(&["empty"])
        .code(2)
        .stderr(predicate::str::contains("no Markdown files to show"));
}

#[test]
fn reports_an_unknown_style() {
    let mrq = Mrq::new();
    fs::write(mrq.dir.path().join(".mrq.yaml"), "render:\n  style: nope\n").unwrap();
    mrq.show(&["guide.md"])
        .code(2)
        .stderr(predicate::str::contains("nope"));
}

#[test]
fn refuses_to_start_without_a_terminal() {
    Mrq::new()
        .show(&["guide.md"])
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("needs a terminal"));
}
