//! `mrq toc` over a file, a directory and stdin, in each output mode.
//!
//! Every test runs `mrq` in `tests/fixtures/toc` with `MRQ_CONFIG_HOME` pointing at an
//! empty temp directory, so no user config leaks in.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

const NOTES: &str = "# My Notes\n\n## Open Questions\n\n## Open Questions\n";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toc")
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
fn file_outline() {
    let out = Mrq::new().stdout(&["toc", "README.md"], None);
    insta::assert_snapshot!("file_outline", out);
}

#[test]
fn file_links_with_depth() {
    let out = Mrq::new().stdout(&["toc", "--depth", "3", "--links", "README.md"], None);
    insta::assert_snapshot!("file_links_depth_3", out);
}

#[test]
fn file_json() {
    let out = Mrq::new().stdout(&["toc", "--json", "README.md"], None);
    insta::assert_snapshot!("file_json", out);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json[0]["file"], "README.md");
    assert_eq!(json[0]["headings"][7]["slug"], "usage-1");
}

#[test]
fn directory_outline_has_file_headers() {
    let out = Mrq::new().stdout(&["toc", "docs"], None);
    insta::assert_snapshot!("directory_outline", out);
}

#[test]
fn directory_links() {
    let out = Mrq::new().stdout(&["toc", "--links", "--depth", "1", "docs"], None);
    insta::assert_snapshot!("directory_links_depth_1", out);
}

#[test]
fn directory_json_is_one_entry_per_file() {
    let out = Mrq::new().stdout(&["toc", "--json", "docs"], None);
    insta::assert_snapshot!("directory_json", out);
}

#[test]
fn stdin_outline_links_and_json() {
    let mrq = Mrq::new();
    insta::assert_snapshot!("stdin_outline", mrq.stdout(&["toc", "-"], Some(NOTES)));
    insta::assert_snapshot!(
        "stdin_links",
        mrq.stdout(&["toc", "--links", "-"], Some(NOTES))
    );
    insta::assert_snapshot!(
        "stdin_json",
        mrq.stdout(&["toc", "--json", "-"], Some(NOTES))
    );
}

#[test]
fn stdin_and_files_mix_with_headers() {
    let out = Mrq::new().stdout(&["toc", "docs/intro.md", "-"], Some("# From stdin\n"));
    assert_eq!(
        out,
        "==> docs/intro.md <==\nIntroduction\n  Why mrq?\n  Goals\n\n==> <stdin> <==\nFrom stdin\n"
    );
}

#[test]
fn flavor_flag_changes_the_slugs() {
    let mrq = Mrq::new();
    let gfm = mrq.stdout(&["toc", "--links", "-"], Some(NOTES));
    assert!(gfm.contains("(#open-questions-1)"), "{gfm}");
    let obsidian = mrq.stdout(
        &["--flavor", "obsidian", "toc", "--links", "-"],
        Some(NOTES),
    );
    assert!(obsidian.contains("(#Open%20Questions)"), "{obsidian}");
}

#[test]
fn configured_flavor_applies_and_frontmatter_overrides_it() {
    let mrq = Mrq::new();
    std::fs::write(mrq.user.path().join("config.yaml"), "flavor: mkdocs\n").unwrap();
    let out = mrq.stdout(&["toc", "--json", "-", "docs/guide/notes.md"], Some(NOTES));
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json[0]["flavor"], "mkdocs");
    assert_eq!(json[0]["headings"][2]["slug"], "open-questions_1");
    assert_eq!(json[1]["flavor"], "obsidian");
}

#[test]
fn json_and_links_conflict() {
    Mrq::new()
        .cmd()
        .args(["toc", "--json", "--links", "README.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn depth_must_be_a_heading_level() {
    Mrq::new()
        .cmd()
        .args(["toc", "--depth", "7", "README.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn missing_paths_exit_2() {
    Mrq::new()
        .cmd()
        .args(["toc", "nope.md"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "nope.md: no such file or directory",
        ));
}

#[test]
fn no_paths_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .arg("toc")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no input"));
}

#[test]
fn unreadable_files_are_reported_and_the_rest_is_printed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bad.md"), [0xff, 0xfe, b'#']).unwrap();
    std::fs::write(dir.path().join("good.md"), "# Good\n").unwrap();
    let out = Mrq::new()
        .cmd()
        .current_dir(dir.path())
        .args(["toc", "bad.md", "good.md"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "==> good.md <==\nGood\n"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("mrq: cannot read bad.md"),
        "{out:?}"
    );
}

#[test]
fn config_errors_exit_2() {
    Mrq::new()
        .cmd()
        .args(["--config", "nope.yaml", "toc", "README.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "cannot read config file nope.yaml",
        ));
}

#[test]
fn insert_is_not_implemented_yet() {
    Mrq::new()
        .cmd()
        .args(["toc", "--insert", "README.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not implemented"));
}
