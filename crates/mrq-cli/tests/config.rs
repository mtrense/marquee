//! `mrq config show [--origin]` and `mrq config schema`.
//!
//! Every test runs `mrq` inside a temp tree with `MRQ_CONFIG_HOME` pointing at
//! `<tmp>/user`, so the real user config never leaks in. Temp paths are redacted to
//! `[TMP]` in snapshots.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

struct Tree {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // Windows canonicalizes to a verbatim `\\?\C:\…` path; `mrq` gets the plain one.
        let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
        fs::create_dir_all(root.join("user")).unwrap();
        fs::create_dir_all(root.join("proj/docs")).unwrap();
        Self { _dir: dir, root }
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::write(&path, text).unwrap();
        path
    }

    /// `mrq` run from `<tmp>/proj/docs`.
    fn mrq(&self) -> Command {
        let mut cmd = Command::cargo_bin("mrq").unwrap();
        cmd.current_dir(self.root.join("proj/docs"))
            .env("MRQ_CONFIG_HOME", self.root.join("user"))
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS");
        cmd
    }

    fn redact(&self, text: &[u8]) -> String {
        let text = String::from_utf8_lossy(text);
        redact_path(&text, &self.root)
    }

    fn stdout(&self, args: &[&str]) -> String {
        let out = self.mrq().args(args).output().unwrap();
        assert!(out.status.success(), "mrq {args:?} failed: {out:?}");
        self.redact(&out.stdout)
    }
}

/// Replaces the temp root by `[TMP]` and, on those lines, `\\` by `/` (Windows paths).
///
/// On Windows the canonical root is a verbatim path (`\\?\C:\…`) while `mrq` prints
/// `C:\…` or `C:/…`, so the root is matched without that prefix and with `/` separators.
fn redact_path(text: &str, root: &Path) -> String {
    let root = root.display().to_string();
    let root = root
        .strip_prefix(r"\\?\")
        .unwrap_or(&root)
        .replace('\\', "/");
    text.split_inclusive('\n')
        .map(|line| {
            let slashed = line.replace('\\', "/");
            if slashed.contains(&root) {
                slashed.replace(&root, "[TMP]")
            } else {
                line.to_owned()
            }
        })
        .collect()
}

fn layered() -> Tree {
    let tree = Tree::new();
    tree.write(
        "user/config.yaml",
        "flavor: mkdocs\nfmt:\n  width: 72\n  emphasis: _\n",
    );
    tree.write(
        "proj/.mrq.yaml",
        "flavor: obsidian\nignore: [\"vendor/**\", CHANGELOG.md]\nlint:\n  rules:\n    MD012: { maximum: 2 }\n    MD047: off\nvalidate:\n  frontmatter:\n    - files: \"blog/**/*.md\"\n      schema: ./post.schema.json\n",
    );
    tree.write("proj/docs/.mrq.yaml", "fmt:\n  wrap: sentence\n");
    tree
}

#[test]
fn show_prints_the_effective_config() {
    let tree = layered();
    insta::assert_snapshot!("show", tree.stdout(&["config", "show"]));
}

#[test]
fn show_origin_annotates_every_value() {
    let tree = layered();
    insta::assert_snapshot!(
        "show_origin",
        tree.stdout(&["--flavor", "commonmark", "config", "show", "--origin"])
    );
}

#[test]
fn show_without_any_config_prints_defaults() {
    let tree = Tree::new();
    let out = tree.stdout(&["config", "show", "--origin"]);
    assert!(out.contains("flavor: gfm  # default\n"), "{out}");
    assert!(out.contains("  wrap: preserve  # default\n"), "{out}");
    assert!(!out.contains("[TMP]"), "{out}");
}

#[test]
fn explicit_config_replaces_discovery() {
    let tree = layered();
    tree.write("other.yaml", "fmt:\n  wrap: never\n");
    let out = tree.stdout(&["--config", "../../other.yaml", "config", "show", "--origin"]);
    assert!(
        out.contains("flavor: mkdocs  # [TMP]/user/config.yaml:1\n"),
        "{out}"
    );
    assert!(
        out.contains("  wrap: never  # ../../other.yaml:2\n"),
        "{out}"
    );
    assert!(!out.contains("obsidian"), "{out}");
}

#[test]
fn missing_explicit_config_exits_2() {
    let tree = Tree::new();
    tree.mrq()
        .args(["--config", "nope.yaml", "config", "show"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "cannot read config file nope.yaml",
        ));
}

#[test]
fn unknown_key_reports_file_and_line_and_exits_2() {
    let tree = Tree::new();
    tree.write(
        "proj/.mrq.yaml",
        "flavor: gfm\nfmt:\n  width: 80\n  wrapp: never\n",
    );
    let out = tree.mrq().args(["config", "show"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    insta::assert_snapshot!("unknown_key", tree.redact(&out.stderr));
}

#[test]
fn config_errors_honour_the_diagnostics_format() {
    let tree = Tree::new();
    tree.write("proj/.mrq.yaml", "fmt:\n  wrap: sometimes\n");
    let out = tree
        .mrq()
        .args(["--format", "github", "config", "show"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = tree.redact(&out.stderr);
    assert!(
        stderr.starts_with("::error file=[TMP]/proj/.mrq.yaml,line=2,"),
        "{stderr}"
    );
    assert!(stderr.contains("sometimes"), "{stderr}");
}

#[test]
fn yaml_syntax_errors_exit_2() {
    let tree = Tree::new();
    tree.write("proj/.mrq.yaml", "fmt: [\n");
    tree.mrq()
        .args(["config", "show"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid YAML"));
}

#[test]
fn schema_prints_the_json_schema() {
    let tree = Tree::new();
    // Even a broken config does not stop `config schema`.
    tree.write("proj/.mrq.yaml", "bogus: 1\n");
    let out = tree.stdout(&["config", "schema"]);
    assert_eq!(out, mrq_core::config::schema_json());
    let schema: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(schema["title"], "mrq configuration (.mrq.yaml)");
}
