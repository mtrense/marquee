//! `mrq merge` over the README's example files: to stdout and to `-o`, with `--shift`,
//! `--title`, `--no-wrap` and `--frontmatter`, its exit codes, and the merged documents
//! passing `mrq validate`.
//!
//! Every test runs `mrq` in a temp directory holding a copy of `tests/fixtures/merge`,
//! with `MRQ_CONFIG_HOME` and `MRQ_CACHE_HOME` pointing at empty temp directories, so no
//! user config or cache leaks in.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use assert_cmd::assert::OutputAssertExt;
use predicates::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge")
}

struct Mrq {
    user: tempfile::TempDir,
    cache: tempfile::TempDir,
    dir: tempfile::TempDir,
}

impl Mrq {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(&fixtures(), dir.path());
        Self {
            user: tempfile::tempdir().unwrap(),
            cache: tempfile::tempdir().unwrap(),
            dir,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path(name)).unwrap()
    }

    fn output(&self, args: &[&str]) -> Output {
        Command::cargo_bin("mrq")
            .unwrap()
            .current_dir(self.dir.path())
            .env("MRQ_CONFIG_HOME", self.user.path())
            .env("MRQ_CACHE_HOME", self.cache.path())
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS")
            .args(args)
            .output()
            .unwrap()
    }

    /// Stdout of `mrq merge`, asserting success and an empty stderr.
    fn merge(&self, args: &[&str]) -> String {
        let out = self.output(&[&["merge"], args].concat());
        assert_eq!(out.status.code(), Some(0), "mrq merge {args:?}: {out:?}");
        assert!(out.stderr.is_empty(), "mrq merge {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    /// Asserts that `mrq validate --strict` finds nothing in `name`.
    fn assert_valid(&self, name: &str) {
        let out = self.output(&["validate", "--strict", name]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "mrq validate {name}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            self.read(name)
        );
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

#[test]
fn readme_files_merge_to_stdout_into_a_valid_document() {
    let mrq = Mrq::new();
    let out = mrq.merge(&["intro.md", "install.md", "usage.md"]);
    insta::assert_snapshot!("readme_stdout", out);
    fs::write(mrq.path("guide.md"), &out).unwrap();
    mrq.assert_valid("guide.md");
}

#[test]
fn chapters_merge_under_a_title_into_a_valid_file() {
    let mrq = Mrq::new();
    let out = mrq.merge(&[
        "--shift",
        "2",
        "--title",
        "User Guide",
        "chapters/",
        "-o",
        "guide.md",
    ]);
    assert_eq!(out, "");
    insta::assert_snapshot!("chapters_title", mrq.read("guide.md"));
    mrq.assert_valid("guide.md");
    // `--shift 2` is the default under a title.
    mrq.merge(&["--title", "User Guide", "chapters/", "-o", "default.md"]);
    assert_eq!(mrq.read("default.md"), mrq.read("guide.md"));
}

#[test]
fn links_are_rebased_for_an_output_in_another_directory() {
    let mrq = Mrq::new();
    mrq.merge(&[
        "intro.md",
        "install.md",
        "usage.md",
        "chapters",
        "-o",
        "out/guide.md",
    ]);
    let text = mrq.read("out/guide.md");
    assert!(text.contains("![Logo](../img/logo.png)"), "{text}");
    assert!(text.contains("[FAQ](../notes/faq.md#questions)"), "{text}");
    assert!(text.contains("[introduction](#introduction)"), "{text}");
    mrq.assert_valid("out/guide.md");
}

#[test]
fn no_wrap_concatenates_and_frontmatter_can_be_stripped() {
    let mrq = Mrq::new();
    let out = mrq.merge(&[
        "--no-wrap",
        "--frontmatter",
        "strip",
        "install.md",
        "chapters",
    ]);
    assert!(
        out.starts_with("# Installation\n\n## Requirements\n"),
        "{out}"
    );
    assert!(out.contains("\n# Basics\n"), "{out}");
    assert!(out.contains("[advanced chapter](#advanced)"), "{out}");
    assert!(!out.contains("---"), "{out}");
}

#[test]
fn keep_first_keeps_the_first_frontmatter_as_written() {
    let mrq = Mrq::new();
    let out = mrq.merge(&[
        "--frontmatter",
        "keep-first",
        "install.md",
        "usage.md",
        "intro.md",
    ]);
    assert!(
        out.starts_with(
            "---\ntitle: Using mrq\nauthors: [Bob]\ntags: [cli]\n---\n\n# Installation\n"
        ),
        "{out}"
    );
}

#[test]
fn the_output_file_is_never_an_input() {
    let mrq = Mrq::new();
    mrq.merge(&["chapters", "-o", "chapters/all.md"]);
    let first = mrq.read("chapters/all.md");
    mrq.merge(&["chapters", "-o", "chapters/all.md"]);
    assert_eq!(mrq.read("chapters/all.md"), first);
}

#[test]
fn clamped_headings_are_warnings_on_stderr() {
    let mrq = Mrq::new();
    let out = mrq.output(&["merge", "--shift", "5", "--no-wrap", "install.md"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("###### Installation\n"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("heading-clamp"), "{stderr}");
}

#[test]
fn a_missing_input_writes_nothing() {
    let mrq = Mrq::new();
    mrq.output(&["merge", "intro.md", "nope.md", "-o", "guide.md"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("nope.md"));
    assert!(!mrq.path("guide.md").exists());
}

#[test]
fn no_input_is_a_usage_error() {
    Mrq::new()
        .output(&["merge"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no input given"));
}
