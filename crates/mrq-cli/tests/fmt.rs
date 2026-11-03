//! `mrq fmt` over the current directory, directories, files and stdin, with `--check`,
//! the configuration's `fmt` settings and code formatters, and its exit codes.
//!
//! Read-only tests run `mrq` in `tests/fixtures/fmt`; tests that write run it in a temp
//! directory with copies of the fixtures. `MRQ_CONFIG_HOME` points at an empty temp
//! directory, so no user config leaks in. The fixtures' `.mrq.yaml` sets
//! `fmt.list_marker: "*"`.

// Some helpers are only used by the `cfg(unix)` tests.
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fmt")
}

struct Mrq {
    user: tempfile::TempDir,
    /// Where `mrq` runs: the fixtures, or a temp copy of them.
    dir: PathBuf,
    _copy: Option<tempfile::TempDir>,
}

impl Mrq {
    fn new() -> Self {
        Self {
            user: tempfile::tempdir().unwrap(),
            dir: fixtures(),
            _copy: None,
        }
    }

    /// Runs in a temp directory holding a copy of the fixtures.
    fn writable() -> Self {
        let copy = tempfile::tempdir().unwrap();
        copy_dir(&fixtures(), copy.path());
        Self {
            user: tempfile::tempdir().unwrap(),
            dir: copy.path().to_path_buf(),
            _copy: Some(copy),
        }
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.dir.join(name)).unwrap()
    }

    fn write(&self, name: &str, text: &str) {
        fs::write(self.dir.join(name), text).unwrap();
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("mrq").unwrap();
        cmd.current_dir(&self.dir)
            .env("MRQ_CONFIG_HOME", self.user.path())
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS");
        cmd
    }

    fn output(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = self.cmd();
        cmd.arg("fmt").args(args);
        if let Some(stdin) = stdin {
            cmd.write_stdin(stdin);
        }
        cmd.output().unwrap()
    }

    /// Stdout of `mrq fmt`, asserting the exit code and an empty stderr, with Windows
    /// path separators turned into `/`.
    fn stdout(&self, args: &[&str], stdin: Option<&str>, code: i32) -> String {
        let out = self.output(args, stdin);
        assert_eq!(out.status.code(), Some(code), "mrq fmt {args:?}: {out:?}");
        assert!(out.stderr.is_empty(), "mrq fmt {args:?}: {out:?}");
        normalize(&out.stdout)
    }
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn normalize(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const MESSY: &str = "# Messy\n\nSome *emphasis* and **strong** text,\\\nspread over lines.\n\n* one\n* two\n\n| a         |   b |\n| :-------- | --: |\n| long cell |   x |\n";

#[test]
fn formatted_files_are_left_alone() {
    let out = Mrq::new().output(&["clean.md"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}

#[test]
fn files_are_rewritten_in_place_with_the_configured_settings() {
    let mrq = Mrq::writable();
    let clean = mrq.read("clean.md");
    assert_eq!(mrq.stdout(&[], None, 0), "");
    assert_eq!(mrq.read("docs/messy.md"), MESSY);
    assert_eq!(mrq.read("clean.md"), clean);
    // Atomic writes leave no temp files behind.
    assert_eq!(names(&mrq.dir.join("docs")), ["code.md", "messy.md"]);
    // Formatted files format to themselves.
    assert_eq!(mrq.stdout(&["--check", "."], None, 0), "");
}

#[test]
fn check_prints_a_diff_and_writes_nothing() {
    let mrq = Mrq::writable();
    let before = mrq.read("docs/messy.md");
    let out = mrq.stdout(&["--check", "docs", "clean.md"], None, 1);
    insta::assert_snapshot!("check_diff", out);
    assert_eq!(mrq.read("docs/messy.md"), before);
    assert_eq!(mrq.stdout(&["--check", "clean.md"], None, 0), "");
}

#[test]
fn config_flag_picks_other_settings() {
    let mrq = Mrq::writable();
    mrq.stdout(&["--config", "configs/wrap.yaml", "docs/messy.md"], None, 0);
    let text = mrq.read("docs/messy.md");
    assert!(
        text.contains("text,\\\nspread over lines.\n\n* one"),
        "{text}"
    );
    mrq.write("docs/messy.md", "A\nparagraph\n- a\n");
    mrq.stdout(&["--config", "configs/wrap.yaml", "docs/messy.md"], None, 0);
    assert_eq!(mrq.read("docs/messy.md"), "A paragraph\n\n* a\n");
}

#[test]
fn stdin_is_formatted_to_stdout() {
    let mrq = Mrq::new();
    let messy = fs::read_to_string(fixtures().join("docs/messy.md")).unwrap();
    assert_eq!(mrq.stdout(&["-"], Some(&messy), 0), MESSY);
    assert_eq!(mrq.stdout(&["-"], Some(MESSY), 0), MESSY);
    assert_eq!(mrq.stdout(&["-"], Some(""), 0), "");
}

#[test]
fn stdin_check_prints_a_diff_naming_stdin() {
    let mrq = Mrq::new();
    let diff = mrq.stdout(&["--check", "-"], Some("- a\n"), 1);
    assert_eq!(
        diff,
        "--- a/<stdin>\n+++ b/<stdin>\n@@ -1 +1 @@\n-- a\n+* a\n"
    );
    assert_eq!(mrq.stdout(&["--check", "-"], Some("* a\n"), 0), "");
}

#[test]
fn a_missing_path_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["fmt", "nope.md"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("nope.md"));
}

#[test]
fn unreadable_inputs_are_reported_and_the_others_formatted() {
    let mrq = Mrq::writable();
    fs::write(mrq.dir.join("docs/binary.md"), [0xff, 0xfe, b'\n']).unwrap();
    let out = mrq.output(&["docs"], None);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = normalize(&out.stderr);
    assert!(stderr.contains("docs/binary.md"), "{stderr}");
    assert_eq!(mrq.read("docs/messy.md"), MESSY);
    // With --check, 2 wins over 1.
    mrq.write("docs/messy.md", "- a\n");
    let out = mrq.output(&["--check", "docs"], None);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(normalize(&out.stdout).contains("+* a"), "{out:?}");
}

/// Writes `.mrq.yaml` with `rust` formatted by `command` in a writable copy.
fn with_rust_formatter(command: &str) -> Mrq {
    let mrq = Mrq::writable();
    mrq.write(
        ".mrq.yaml",
        &format!("fmt:\n  list_marker: \"*\"\n  code_formatters:\n    rust: {command}\n"),
    );
    mrq
}

#[cfg(unix)]
#[test]
fn code_formatter_failures_are_warnings_on_stderr() {
    let mrq = with_rust_formatter(r#"["sh", "-c", "echo broken >&2; exit 3"]"#);
    let before = mrq.read("docs/code.md");
    let out = mrq.output(&["--format", "json", "docs"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    let [warning] = json.as_array().unwrap().as_slice() else {
        panic!("{json}")
    };
    assert_eq!(warning["code"], "code-formatter");
    assert_eq!(warning["severity"], "warning");
    assert_eq!(
        normalize(warning["file"].as_str().unwrap().as_bytes()),
        "docs/code.md"
    );
    assert!(
        warning["message"].as_str().unwrap().contains("broken"),
        "{warning}"
    );
    assert_eq!(mrq.read("docs/code.md"), before);
    assert_eq!(mrq.read("docs/messy.md"), MESSY);

    // Stdout holds the formatted stdin text; the warnings stay on stderr.
    let out = mrq.output(&["-"], Some("```rust\nfn  x() {}\n```\n"));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(normalize(&out.stdout), "```rust\nfn  x() {}\n```\n");
    assert!(normalize(&out.stderr).contains("code-formatter"), "{out:?}");
}

/// A relative formatter path is relative to the config file, wherever `mrq` runs.
#[cfg(unix)]
#[test]
fn code_formatters_resolve_against_their_config_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let mrq = with_rust_formatter(r#"["./tools/upper.sh"]"#);
    fs::create_dir_all(mrq.dir.join("tools")).unwrap();
    let script = mrq.dir.join("tools/upper.sh");
    fs::write(&script, "#!/bin/sh\ntr a-z A-Z\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let out = mrq
        .cmd()
        .current_dir(mrq.dir.join("docs"))
        .args(["fmt", "code.md"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(
        mrq.read("docs/code.md"),
        "# Code\n\n```rust\nFN MAIN() {}\n```\n"
    );
}
