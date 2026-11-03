//! `mrq split`: the default directory, `--out`, `-d`, `--dry-run`, stdin, its warnings
//! and exit codes, and `mrq interpolate` giving the file back.
//!
//! Every test runs `mrq` in a fresh temp directory with `MRQ_CONFIG_HOME` pointing at an
//! empty one, so no user config leaks in.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

const GUIDE: &str = "# Guide\n\n![logo](img/logo.png) Start with [the intro](#introduction).\n\n\
                     ## Introduction\n\nRead [usage](#usage) and [its setup](#setup-1).\n\n\
                     ## Setup\n\nRun it.\n\n## Usage\n\n### Setup\n\nBack to [setup](#setup).\n";
const INDEX: &str = "# Guide\n\n![logo](../img/logo.png) Start with [the intro](01-introduction.md#introduction).\n\n\
                     <!-- mrq:include path=\"01-introduction.md\" shift=1 -->\n\n\
                     <!-- mrq:include path=\"02-setup.md\" shift=1 -->\n\n\
                     <!-- mrq:include path=\"03-usage.md\" shift=1 -->\n";
const USAGE: &str = "# Usage\n\n## Setup\n\nBack to [setup](02-setup.md#setup).\n";

struct Mrq {
    user: tempfile::TempDir,
    dir: tempfile::TempDir,
}

impl Mrq {
    /// A directory with `guide.md`.
    fn new() -> Self {
        let mrq = Self {
            user: tempfile::tempdir().unwrap(),
            dir: tempfile::tempdir().unwrap(),
        };
        mrq.write("guide.md", GUIDE);
        mrq
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap()
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("mrq").unwrap();
        cmd.current_dir(self.dir.path())
            .env("MRQ_CONFIG_HOME", self.user.path())
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS")
            .args(args);
        cmd
    }

    /// The names in the directory `name`, sorted.
    fn entries(&self, name: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.path().join(name))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Stdout of `mrq interpolate`, asserting success.
    fn interpolate(&self, name: &str) -> String {
        let out = self.cmd(&["interpolate", name]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert!(out.stderr.is_empty(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    }
}

#[test]
fn a_file_splits_into_its_directory_and_interpolates_back() {
    let mrq = Mrq::new();
    mrq.cmd(&["split", "-d", "2", "guide.md"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
    assert_eq!(
        mrq.entries("guide"),
        [
            "01-introduction.md",
            "02-setup.md",
            "03-usage.md",
            "index.md"
        ]
    );
    assert_eq!(mrq.read("guide/index.md"), INDEX);
    assert_eq!(
        mrq.read("guide/01-introduction.md"),
        "# Introduction\n\nRead [usage](03-usage.md#usage) and [its setup](03-usage.md#setup).\n"
    );
    assert_eq!(mrq.read("guide/03-usage.md"), USAGE);
    assert_eq!(mrq.read("guide.md"), GUIDE);

    // The links are relative to `guide/index.md` now.
    let expanded = GUIDE.replace("](img/", "](../img/");
    assert_eq!(mrq.interpolate("guide/index.md"), expanded);
    // Splitting again replaces the files.
    mrq.cmd(&["split", "-d", "2", "guide.md"])
        .assert()
        .success();
    assert_eq!(mrq.read("guide/index.md"), INDEX);
}

#[test]
fn out_sets_the_directory_and_the_files_own_one_gives_it_back_exactly() {
    let mrq = Mrq::new();
    mrq.cmd(&["split", "--depth", "2", "--out", ".", "guide.md"])
        .assert()
        .success();
    assert_eq!(
        mrq.entries("."),
        [
            "01-introduction.md",
            "02-setup.md",
            "03-usage.md",
            "guide.md",
            "index.md"
        ]
    );
    assert_eq!(mrq.interpolate("index.md"), GUIDE);

    mrq.cmd(&["split", "-o", "parts/guide", "guide.md"])
        .assert()
        .success();
    assert_eq!(mrq.entries("parts/guide"), ["01-guide.md", "index.md"]);
    assert_eq!(
        mrq.read("parts/guide/index.md"),
        "<!-- mrq:include path=\"01-guide.md\" -->\n"
    );
}

#[test]
fn dry_run_lists_the_files_and_writes_nothing() {
    let mrq = Mrq::new();
    mrq.cmd(&["split", "-d", "2", "--dry-run", "guide.md"])
        .assert()
        .success()
        .stdout(
            [
                "guide/index.md",
                "guide/01-introduction.md",
                "guide/02-setup.md",
                "guide/03-usage.md",
                "",
            ]
            .join("\n"),
        )
        .stderr(predicate::str::is_empty());
    assert_eq!(mrq.entries("."), ["guide.md"]);
}

#[test]
fn stdin_needs_out() {
    let mrq = Mrq::new();
    mrq.cmd(&["split", "-"])
        .write_stdin(GUIDE)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("needs --out"));
    mrq.cmd(&["split", "-d", "2", "--out", "guide", "-"])
        .write_stdin(GUIDE)
        .assert()
        .success();
    assert_eq!(mrq.read("guide/03-usage.md"), USAGE);
}

#[test]
fn unpromoted_sections_are_warnings() {
    let mrq = Mrq::new();
    mrq.write("setext.md", "# T\n\nPart\n----\n\ntext\n");
    mrq.cmd(&["split", "-d", "2", "setext.md"])
        .assert()
        .success()
        .stderr(predicate::str::contains("split-promote"));
    assert_eq!(mrq.read("setext/01-part.md"), "Part\n----\n\ntext\n");
    assert_eq!(
        mrq.interpolate("setext/index.md"),
        "# T\n\nPart\n----\n\ntext\n"
    );
}

#[test]
fn nothing_to_split_unreadable_files_and_overwriting_the_file_fail() {
    let mrq = Mrq::new();
    mrq.cmd(&["split", "-d", "5", "guide.md"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no level-5 headings"));
    mrq.cmd(&["split", "missing.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("missing.md"));
    mrq.write("docs/index.md", "# A\n\n# B\n");
    mrq.cmd(&["split", "--out", "docs", "docs/index.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("overwrite"));
    assert_eq!(mrq.read("docs/index.md"), "# A\n\n# B\n");
    assert_eq!(mrq.entries("."), ["docs", "guide.md"]);
}
