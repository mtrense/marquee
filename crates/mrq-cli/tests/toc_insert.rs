//! `mrq toc --insert` and `--check` over files, directories and stdin.
//!
//! Every test runs `mrq` in a fresh temp directory with `MRQ_CONFIG_HOME` pointing at an
//! empty one, so no user config leaks in.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

const STALE: &str = "# Guide\n\nIntro.\n\n<!-- mrq:toc -->\n- [Old](#old)\n<!-- mrq:end -->\n\n## Install\n\n### From source\n";
const FRESH: &str = "# Guide\n\nIntro.\n\n<!-- mrq:toc -->\n\n- [Guide](#guide)\n  - [Install](#install)\n    - [From source](#from-source)\n\n<!-- mrq:end -->\n\n## Install\n\n### From source\n";
const PLAIN: &str = "# No toc here\n\ntext\n";

struct Mrq {
    user: tempfile::TempDir,
    dir: tempfile::TempDir,
}

impl Mrq {
    fn new() -> Self {
        Self {
            user: tempfile::tempdir().unwrap(),
            dir: tempfile::tempdir().unwrap(),
        }
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

    fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn sep(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
}

#[test]
fn insert_rewrites_the_file_and_is_idempotent() {
    let mrq = Mrq::new();
    mrq.write("README.md", STALE);
    mrq.cmd(&["toc", "--insert", "README.md"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
    assert_eq!(mrq.read("README.md"), FRESH);
    // No temp files are left behind.
    assert_eq!(mrq.entries(), vec!["README.md"]);

    mrq.cmd(&["toc", "--insert", "README.md"])
        .assert()
        .success();
    assert_eq!(mrq.read("README.md"), FRESH);
}

#[test]
fn insert_honours_depth() {
    let mrq = Mrq::new();
    mrq.write("README.md", STALE);
    mrq.cmd(&["toc", "--insert", "--depth", "2", "README.md"])
        .assert()
        .success();
    assert_eq!(
        mrq.read("README.md"),
        FRESH.replace("    - [From source](#from-source)\n", "")
    );
}

#[test]
fn check_reports_stale_files_without_writing() {
    let mrq = Mrq::new();
    mrq.write("docs/a.md", STALE);
    mrq.write("docs/b.md", FRESH);
    mrq.write("docs/c.md", PLAIN);
    mrq.cmd(&["toc", "--insert", "--check", "docs"])
        .assert()
        .code(1)
        .stdout(format!(
            "{}: table of contents is out of date\n",
            sep("docs/a.md")
        ));
    assert_eq!(mrq.read("docs/a.md"), STALE);

    mrq.cmd(&["toc", "--insert", "docs"]).assert().success();
    assert_eq!(mrq.read("docs/a.md"), FRESH);
    assert_eq!(mrq.read("docs/b.md"), FRESH);
    assert_eq!(mrq.read("docs/c.md"), PLAIN);
    mrq.cmd(&["toc", "--insert", "--check", "docs"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn files_without_a_toc_region_are_left_alone() {
    let mrq = Mrq::new();
    mrq.write("plain.md", PLAIN);
    mrq.cmd(&["toc", "--insert", "plain.md"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
    assert_eq!(mrq.read("plain.md"), PLAIN);
}

#[test]
fn stdin_is_printed_to_stdout() {
    let mrq = Mrq::new();
    mrq.cmd(&["toc", "--insert", "-"])
        .write_stdin(STALE)
        .assert()
        .success()
        .stdout(FRESH);
    mrq.cmd(&["toc", "--insert", "-"])
        .write_stdin(PLAIN)
        .assert()
        .success()
        .stdout(PLAIN);
}

#[test]
fn stdin_check_prints_no_document() {
    let mrq = Mrq::new();
    mrq.cmd(&["toc", "--insert", "--check", "-"])
        .write_stdin(STALE)
        .assert()
        .code(1)
        .stdout("<stdin>: table of contents is out of date\n");
    mrq.cmd(&["toc", "--insert", "--check", "-"])
        .write_stdin(FRESH)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn invalid_depth_attribute_is_reported_and_the_file_kept() {
    let mrq = Mrq::new();
    let src = "<!-- mrq:toc depth=0 -->\n<!-- mrq:end -->\n\n# A\n";
    mrq.write("bad.md", src);
    mrq.cmd(&["toc", "--insert", "bad.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("`depth` must be a heading level"));
    assert_eq!(mrq.read("bad.md"), src);
}

#[test]
fn check_requires_insert() {
    let mrq = Mrq::new();
    mrq.write("README.md", STALE);
    mrq.cmd(&["toc", "--check", "README.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--insert"));
}

#[cfg(unix)]
#[test]
fn insert_keeps_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let mrq = Mrq::new();
    mrq.write("README.md", STALE);
    let path = mrq.dir.path().join("README.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    mrq.cmd(&["toc", "--insert", "README.md"])
        .assert()
        .success();
    let mode = fs::metadata(Path::new(&path)).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o640);
}
