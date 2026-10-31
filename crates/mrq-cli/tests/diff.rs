//! `mrq diff` over the fixtures in `tests/fixtures/diff`: human output with and without
//! colors, `--word`, `--json`, exit codes 0/1/2, stdin, and `git difftool -x 'mrq diff'`
//! style invocations (temp files, `/dev/null` and empty sides).
//!
//! Every test runs `mrq` in `tests/fixtures/diff` with `MRQ_CONFIG_HOME` pointing at an
//! empty temp directory, so no user config leaks in.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diff")
}

struct Mrq {
    user: tempfile::TempDir,
}

/// What a run printed and its exit code.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
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

    fn run(&self, args: &[&str], stdin: Option<&str>) -> Run {
        let mut cmd = self.cmd();
        cmd.arg("diff").args(args);
        if let Some(stdin) = stdin {
            cmd.write_stdin(stdin);
        }
        let out = cmd.output().unwrap();
        Run {
            code: out.status.code().unwrap(),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }

    /// Stdout of a run that found differences (exit 1).
    fn changed(&self, args: &[&str]) -> String {
        let run = self.run(args, None);
        assert_eq!(run.code, 1, "mrq diff {args:?}: {}", run.stderr);
        assert!(run.stderr.is_empty(), "{}", run.stderr);
        run.stdout
    }
}

#[test]
fn human_without_colors() {
    let out = Mrq::new().changed(&["--color", "never", "old.md", "new.md"]);
    assert!(!out.contains('\x1b'), "{out}");
    insta::assert_snapshot!("human", out);
}

#[test]
fn human_is_plain_when_not_a_terminal() {
    let mrq = Mrq::new();
    let auto = mrq.changed(&["old.md", "new.md"]);
    assert_eq!(auto, mrq.changed(&["--color", "never", "old.md", "new.md"]));
}

#[test]
fn human_with_colors() {
    let out = Mrq::new().changed(&["--color", "always", "old.md", "new.md"]);
    assert!(
        out.contains("\x1b[31m") && out.contains("\x1b[32m"),
        "{out}"
    );
    insta::assert_snapshot!("human_color", out);
}

#[test]
fn word_without_colors() {
    let out = Mrq::new().changed(&["--word", "--color", "never", "old.md", "new.md"]);
    assert!(out.contains("[-a-]{+one or more+}"), "{out}");
    insta::assert_snapshot!("word", out);
}

#[test]
fn word_with_colors() {
    let out = Mrq::new().changed(&["--word", "--color", "always", "old.md", "new.md"]);
    assert!(!out.contains("{+"), "{out}");
    insta::assert_snapshot!("word_color", out);
}

#[test]
fn json() {
    let out = Mrq::new().changed(&["--json", "old.md", "new.md"]);
    insta::assert_snapshot!("json", out);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["old"], "old.md");
    assert_eq!(json["new"], "new.md");
    let sections = json["sections"].as_array().unwrap();
    assert!(sections.iter().all(|section| {
        section["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|op| op["op"] != "equal")
    }));
}

#[test]
fn json_with_words() {
    let out = Mrq::new().changed(&["--json", "--word", "old.md", "new.md"]);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let words = json["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|section| section["blocks"].as_array().unwrap())
        .find_map(|op| op.get("words"))
        .expect("a modified block with words");
    assert!(
        words
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["op"] == "insert")
    );
}

#[test]
fn formatting_only_changes_exit_0() {
    let run = Mrq::new().run(&["old.md", "reformatted.md"], None);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, "");
}

#[test]
fn json_without_changes_has_no_sections() {
    let run = Mrq::new().run(&["--json", "old.md", "reformatted.md"], None);
    assert_eq!(run.code, 0, "{}", run.stderr);
    insta::assert_snapshot!("json_unchanged", run.stdout);
}

#[test]
fn missing_file_exits_2() {
    let run = Mrq::new().run(&["old.md", "missing.md"], None);
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    insta::assert_snapshot!("missing", run.stderr.replace('\\', "/"));
}

#[test]
fn a_directory_exits_2() {
    let run = Mrq::new().run(&["old.md", "."], None);
    assert_eq!(run.code, 2);
    assert!(run.stderr.starts_with("mrq: "), "{}", run.stderr);
}

#[test]
fn missing_argument_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["diff", "old.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("<NEW>"));
}

#[test]
fn stdin_as_one_side() {
    let new = std::fs::read_to_string(fixtures().join("new.md")).unwrap();
    let mrq = Mrq::new();
    let run = mrq.run(&["--color", "never", "old.md", "-"], Some(&new));
    assert_eq!(run.code, 1, "{}", run.stderr);
    let from_file = mrq.changed(&["--color", "never", "old.md", "new.md"]);
    assert_eq!(
        run.stdout,
        from_file.replace("+++ new.md", "+++ <stdin>"),
        "only the name differs"
    );
}

#[test]
fn stdin_on_both_sides_exits_2() {
    let run = Mrq::new().run(&["-", "-"], Some(""));
    assert_eq!(run.code, 2);
    insta::assert_snapshot!("stdin_twice", run.stderr);
}

/// `git difftool -x 'mrq diff'` runs `mrq diff "$LOCAL" "$REMOTE"` on temp copies of
/// both sides, from the repository root.
#[test]
fn difftool_temp_files() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("abc123_README.md");
    let remote = temp.path().join("def456_README.md");
    std::fs::copy(fixtures().join("old.md"), &local).unwrap();
    std::fs::copy(fixtures().join("new.md"), &remote).unwrap();
    let mrq = Mrq::new();
    let run = mrq.run(
        &[
            "--color",
            "never",
            local.to_str().unwrap(),
            remote.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(run.code, 1, "{}", run.stderr);
    let expected = mrq.changed(&["--color", "never", "old.md", "new.md"]);
    let body = |out: &str| out.lines().skip(2).collect::<Vec<_>>().join("\n");
    assert_eq!(body(&run.stdout), body(&expected));
}

/// A file added in the diffed range: git passes `/dev/null` as `$LOCAL`.
#[cfg(unix)]
#[test]
fn difftool_new_file() {
    let out = Mrq::new().changed(&["--color", "never", "/dev/null", "new.md"]);
    insta::assert_snapshot!("dev_null_old", out);
}

/// A deleted file: `/dev/null` as `$REMOTE`.
#[cfg(unix)]
#[test]
fn difftool_deleted_file() {
    let out = Mrq::new().changed(&["--color", "never", "old.md", "/dev/null"]);
    assert!(out.contains("- ## Legacy"), "{out}");
    assert!(!out.contains("\n+ "), "{out}");
}

/// `git difftool -y -x 'mrq diff' -- README.md` in a fresh repository, over a commit that
/// adds the file and one that edits it. Skipped if git is not installed.
#[test]
fn git_difftool() {
    let git_ok = std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !git_ok {
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    let mrq = Mrq::new();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("MRQ_CONFIG_HOME", mrq.user.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    let readme = repo.path().join("README.md");
    git(&["init", "-q"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["commit", "-q", "--allow-empty", "-m", "empty"]);
    std::fs::copy(fixtures().join("old.md"), &readme).unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-q", "-m", "add"]);
    std::fs::copy(fixtures().join("new.md"), &readme).unwrap();
    git(&["commit", "-q", "-am", "edit"]);

    let bin = assert_cmd::cargo::cargo_bin("mrq");
    let tool = format!("'{}' diff --color never", bin.display());
    let edit = git(&[
        "difftool",
        "-y",
        "-x",
        &tool,
        "HEAD~1",
        "HEAD",
        "--",
        "README.md",
    ]);
    let expected = mrq.changed(&["--color", "never", "old.md", "new.md"]);
    let body = |out: &str| out.lines().skip(2).collect::<Vec<_>>().join("\n");
    assert_eq!(body(&edit), body(&expected));

    let added = git(&[
        "difftool",
        "-y",
        "-x",
        &tool,
        "HEAD~2",
        "HEAD~1",
        "--",
        "README.md",
    ]);
    assert!(added.contains("# Guide  [added]"), "{added}");
}

#[test]
fn empty_old_side() {
    let temp = tempfile::tempdir().unwrap();
    let empty = temp.path().join("empty.md");
    std::fs::write(&empty, "").unwrap();
    let mrq = Mrq::new();
    let out = mrq.changed(&["--color", "never", empty.to_str().unwrap(), "new.md"]);
    assert!(out.contains("+ # Guide"), "{out}");
    let run = mrq.run(&[empty.to_str().unwrap(), empty.to_str().unwrap()], None);
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
}
