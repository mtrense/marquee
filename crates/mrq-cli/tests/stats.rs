//! `mrq stats` over a file, a directory, stdin and the current directory, as a table
//! and as JSON, with `--include-code` and `--wpm`.
//!
//! Every test runs `mrq` in `tests/fixtures/stats` with `MRQ_CONFIG_HOME` pointing at an
//! empty temp directory, so no user config leaks in.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stats")
}

struct Mrq {
    user: tempfile::TempDir,
}

impl Mrq {
    fn new() -> Self {
        Self {
            user: tempfile::tempdir().unwrap(),
        }
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("mrq").unwrap();
        cmd.current_dir(fixtures())
            .env("MRQ_CONFIG_HOME", self.user.path())
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS");
        cmd
    }

    /// Stdout of a successful run, with Windows path separators turned into `/`.
    fn stdout(&self, args: &[&str], stdin: Option<&str>) -> String {
        let mut cmd = self.cmd();
        cmd.args(args);
        if let Some(stdin) = stdin {
            cmd.write_stdin(stdin);
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "mrq {args:?} failed: {out:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        if cfg!(windows) {
            text.replace("\\\\", "/").replace('\\', "/")
        } else {
            text
        }
    }
}

#[test]
fn file_table() {
    let out = Mrq::new().stdout(&["stats", "README.md"], None);
    insta::assert_snapshot!("file_table", out);
}

#[test]
fn file_json() {
    let out = Mrq::new().stdout(&["stats", "--json", "README.md"], None);
    insta::assert_snapshot!("file_json", out);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["files"][0]["file"], "README.md");
    assert_eq!(json["total"]["words"], json["files"][0]["words"]);
    assert_eq!(json["files"][0]["tasks"]["open"], 2);
    assert_eq!(json["files"][0]["tasks"]["done"], 1);
}

#[test]
fn directory_table_has_a_total_row() {
    let out = Mrq::new().stdout(&["stats", "docs"], None);
    insta::assert_snapshot!("directory_table", out);
}

#[test]
fn directory_json() {
    let out = Mrq::new().stdout(&["stats", "--json", "docs"], None);
    insta::assert_snapshot!("directory_json", out);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["total"]["files"], 2);
    assert_eq!(json["total"]["code_blocks"]["total"], 3);
    assert_eq!(json["total"]["code_blocks"]["languages"]["sh"], 1);
}

#[test]
fn no_path_means_the_current_directory() {
    let mrq = Mrq::new();
    let out = mrq.stdout(&["stats", "--json"], None);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["total"]["files"], 3);
    insta::assert_snapshot!("current_directory", mrq.stdout(&["stats"], None));
}

#[test]
fn stdin_table_and_json() {
    let notes = "# Notes\n\nSome words, *some* more.\n\n- [ ] todo\n";
    let mrq = Mrq::new();
    insta::assert_snapshot!("stdin_table", mrq.stdout(&["stats", "-"], Some(notes)));
    let out = mrq.stdout(&["stats", "--json", "-"], Some(notes));
    insta::assert_snapshot!("stdin_json", out);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["files"][0]["file"], serde_json::Value::Null);
    assert_eq!(json["total"]["words"], 6);
}

#[test]
fn include_code_counts_code_block_words() {
    let words = |args: &[&str]| {
        let out = Mrq::new().stdout(args, None);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        json["total"]["words"].as_u64().unwrap()
    };
    let without = words(&["stats", "--json", "docs/notes.md"]);
    let with = words(&["stats", "--json", "--include-code", "docs/notes.md"]);
    // echo one two three
    assert_eq!(with, without + 4);
    insta::assert_snapshot!(
        "include_code",
        Mrq::new().stdout(&["stats", "--include-code", "docs"], None)
    );
}

#[test]
fn wpm_sets_the_reading_speed() {
    let text = format!("{}\n", "word ".repeat(400));
    let mrq = Mrq::new();
    let out = mrq.stdout(&["stats", "--json", "--wpm", "180", "-"], Some(&text));
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["wpm"], 180);
    assert_eq!(json["total"]["reading_minutes"], 3);
    let out = mrq.stdout(&["stats", "--json", "-"], Some(&text));
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["total"]["reading_minutes"], 2);
    insta::assert_snapshot!(
        "wpm",
        mrq.stdout(&["stats", "--wpm", "180", "-"], Some(&text))
    );
}

#[test]
fn zero_wpm_is_rejected() {
    Mrq::new()
        .cmd()
        .args(["stats", "--wpm", "0", "README.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn missing_path_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["stats", "nope.md"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("nope.md"));
}
