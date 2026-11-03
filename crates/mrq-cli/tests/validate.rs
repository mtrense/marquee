//! `mrq validate` over the current directory, a directory, files and stdin, in every
//! output format, with its exit codes and its configuration (frontmatter schemas,
//! external links).
//!
//! Every test runs `mrq` in `tests/fixtures/validate` with `MRQ_CONFIG_HOME` and
//! `MRQ_CACHE_HOME` pointing at empty temp directories, so no user config or cache leaks
//! in. Nothing here touches the network: external links are either not checked or all
//! excluded.

use std::path::PathBuf;
use std::process::Output;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/validate")
}

struct Mrq {
    user: tempfile::TempDir,
    cache: tempfile::TempDir,
}

impl Mrq {
    fn new() -> Self {
        Self {
            user: tempfile::tempdir().unwrap(),
            cache: tempfile::tempdir().unwrap(),
        }
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("mrq").unwrap();
        cmd.current_dir(fixtures())
            .env("MRQ_CONFIG_HOME", self.user.path())
            .env("MRQ_CACHE_HOME", self.cache.path())
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS");
        cmd
    }

    fn output(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = self.cmd();
        cmd.arg("validate").args(args);
        if let Some(stdin) = stdin {
            cmd.write_stdin(stdin);
        }
        cmd.output().unwrap()
    }

    /// Stdout of `mrq validate`, asserting the exit code, with Windows path separators
    /// turned into `/`, and the version and the fixture directory redacted.
    fn stdout(&self, args: &[&str], code: i32) -> String {
        let out = self.output(args, None);
        assert_eq!(
            out.status.code(),
            Some(code),
            "mrq validate {args:?}: {out:?}"
        );
        normalize(&out.stdout)
    }
}

fn normalize(bytes: &[u8]) -> String {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    // Schema paths in the help are absolute, and `mrq` prints them with `/` separators.
    let dir = fixtures().display().to_string().replace('\\', "/");
    let text = text.replace(&dir, "[fixtures]");
    text.replace(env!("CARGO_PKG_VERSION"), "[version]")
}

#[test]
fn clean_files_exit_0_and_print_nothing() {
    let out = Mrq::new().output(&["clean"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}

#[test]
fn warnings_alone_exit_0() {
    let out = Mrq::new().stdout(&["--format", "human", "warn"], 0);
    insta::assert_snapshot!("warnings_human", out);
}

#[test]
fn strict_makes_warnings_fail() {
    let out = Mrq::new().stdout(&["--strict", "--format", "github", "warn"], 1);
    assert!(out.contains("::warning"), "{out}");
}

#[test]
fn no_path_validates_the_current_directory() {
    let mrq = Mrq::new();
    let out = mrq.stdout(&["--format", "json"], 1);
    assert_eq!(out, mrq.stdout(&["--format", "json", "."], 1));
    for file in [
        "blog/post.md",
        "docs/guide.md",
        "docs/malformed.md",
        "warn/toc.md",
    ] {
        assert!(out.contains(file), "{file} missing in {out}");
    }
}

#[test]
fn human_format_shows_source_snippets() {
    // Not `blog`: its schema help has an absolute path, which wraps differently per machine.
    let out = Mrq::new().stdout(&["--format", "human", "docs", "warn"], 1);
    insta::assert_snapshot!("docs_warn_human", out);
}

#[test]
fn every_check_reports_once_in_json() {
    let out = Mrq::new().stdout(&["--format", "json", "."], 1);
    insta::assert_snapshot!("current_directory_json", out);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let codes: Vec<(&str, &str)> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["file"].as_str().unwrap(), d["code"].as_str().unwrap()))
        .collect();
    for expected in [
        ("blog/post.md", "frontmatter-schema"),
        ("docs/guide.md", "missing-file"),
        ("docs/guide.md", "missing-anchor"),
        ("docs/guide.md", "missing-asset"),
        ("docs/guide.md", "unused-footnote"),
        ("docs/guide.md", "directive"),
        ("docs/malformed.md", "frontmatter"),
        ("warn/toc.md", "stale-toc"),
    ] {
        assert!(
            codes.contains(&expected),
            "{expected:?} missing in {codes:?}"
        );
    }
    // The invalid `depth` is reported by the checks, not again from parsing.
    let depth = json
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["message"].as_str().unwrap().contains("`depth`"))
        .count();
    assert_eq!(depth, 1, "{out}");
    // External links are not checked by default.
    assert!(!out.contains("broken-external-link"));
}

#[test]
fn ndjson_prints_one_object_per_line() {
    let out = Mrq::new().stdout(&["--format", "ndjson", "docs/guide.md"], 1);
    insta::assert_snapshot!("guide_ndjson", out);
    for line in out.lines() {
        let _: serde_json::Value = serde_json::from_str(line).unwrap();
    }
}

#[test]
fn sarif_is_a_sarif_2_1_0_log() {
    let out = Mrq::new().stdout(&["--format", "sarif", "."], 1);
    insta::assert_snapshot!("current_directory_sarif", out);
    let log: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(log["version"], "2.1.0");
    let results = log["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 10, "{out}");
    for result in results {
        let uri = result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
            .as_str()
            .unwrap();
        assert!(!uri.starts_with('/') && !uri.starts_with("./"), "{uri}");
    }
}

#[test]
fn github_format_prints_workflow_commands() {
    let out = Mrq::new().stdout(&["--format", "github", "docs", "blog"], 1);
    insta::assert_snapshot!("docs_blog_github", out);
}

#[test]
fn github_format_is_the_default_in_github_actions() {
    let out = Mrq::new()
        .cmd()
        .env("GITHUB_ACTIONS", "true")
        .args(["validate", "warn"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(normalize(&out.stdout).starts_with("::warning file=warn/toc.md,"));
}

#[test]
fn stdin_is_validated_as_stdin() {
    let out = Mrq::new().output(
        &["--format", "json", "-"],
        Some("# Notes\n\n[ok](#notes) [bad](#nowhere)\n"),
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1, "{json}");
    assert_eq!(json[0]["file"], "<stdin>");
    assert_eq!(json[0]["code"], "missing-anchor");
    assert_eq!(json[0]["range"]["start"]["line"], 3);
}

#[test]
fn a_missing_path_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["validate", "nope.md"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("nope.md"));
}

#[test]
fn an_unreadable_schema_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["validate", "--config", "configs/bad-schema.yaml", "clean"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("missing.schema.json"));
}

#[test]
fn an_invalid_exclude_pattern_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["validate", "--config", "configs/bad-exclude.yaml", "clean"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("(unclosed"));
}

#[cfg(feature = "external-links")]
#[test]
fn external_links_are_checked_unless_excluded() {
    // Every URL is excluded, so nothing is requested.
    Mrq::new()
        .cmd()
        .args([
            "validate",
            "--external",
            "--config",
            "configs/exclude-all.yaml",
            "clean",
        ])
        .assert()
        .code(0)
        .stdout("");
}

#[cfg(not(feature = "external-links"))]
#[test]
fn external_without_the_feature_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["validate", "--external", "clean"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("external-links"));
}
