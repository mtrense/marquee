//! `mrq lint` over the current directory, directories, files and stdin, in every output
//! format, with `--rule`, `--fix`, `--fix-only` and `--diff`, and its exit codes.
//!
//! Read-only tests run `mrq` in `tests/fixtures/lint`; tests that write run it in a temp
//! directory with copies of the fixtures. `MRQ_CONFIG_HOME` points at an empty temp
//! directory, so no user config leaks in, and both directories have a `.mrq.yaml` with a
//! `lint` section, so no `.markdownlint.*` further up does either.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lint")
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
        cmd.arg("lint").args(args);
        if let Some(stdin) = stdin {
            cmd.write_stdin(stdin);
        }
        cmd.output().unwrap()
    }

    /// Stdout of `mrq lint`, asserting the exit code, with Windows path separators turned
    /// into `/`.
    fn stdout(&self, args: &[&str], code: i32) -> String {
        let out = self.output(args, None);
        assert_eq!(out.status.code(), Some(code), "mrq lint {args:?}: {out:?}");
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
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let text = if cfg!(windows) {
        text.replace("\\\\", "/").replace('\\', "/")
    } else {
        text
    };
    text.replace(env!("CARGO_PKG_VERSION"), "[version]")
}

/// `(file, code)` of every finding in JSON output.
fn codes(json: &str) -> Vec<(String, String)> {
    let json: serde_json::Value = serde_json::from_str(json).unwrap();
    json.as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["file"].as_str().unwrap().to_owned(),
                d["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

const FIXED: &str = "# Fixable\n\nText with trailing spaces.\n\n* one\n* two\n";

#[test]
fn clean_files_exit_0_and_print_nothing() {
    let out = Mrq::new().output(&["clean.md"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}

#[test]
fn findings_exit_1_in_the_human_format() {
    let out = Mrq::new().stdout(&["--format", "human", "docs"], 1);
    insta::assert_snapshot!("docs_human", out);
}

#[test]
fn no_path_lints_the_current_directory() {
    let mrq = Mrq::new();
    let out = mrq.stdout(&["--format", "json"], 1);
    assert_eq!(out, mrq.stdout(&["--format", "json", "."], 1));
    insta::assert_snapshot!("current_directory_json", out);
}

#[test]
fn every_format_is_supported() {
    let mrq = Mrq::new();
    let github = mrq.stdout(&["--format", "github", "docs/mixed.md"], 1);
    insta::assert_snapshot!("mixed_github", github);
    let ndjson = mrq.stdout(&["--format", "ndjson", "docs"], 1);
    for line in ndjson.lines() {
        let _: serde_json::Value = serde_json::from_str(line).unwrap();
    }
    let sarif = mrq.stdout(&["--format", "sarif", "docs"], 1);
    let log: serde_json::Value = serde_json::from_str(&sarif).unwrap();
    assert_eq!(log["version"], "2.1.0");
    assert_eq!(
        log["runs"][0]["results"].as_array().unwrap().len(),
        ndjson.lines().count()
    );
}

#[test]
fn rule_restricts_by_id_alias_or_tag() {
    let mrq = Mrq::new();
    let only = |rules: &[&str]| {
        let mut args = vec!["--format", "json"];
        for rule in rules {
            args.extend(["--rule", rule]);
        }
        args.push("docs");
        let mut found: Vec<String> = codes(&mrq.stdout(&args, 1))
            .into_iter()
            .map(|(_, code)| code)
            .collect();
        found.sort();
        found.dedup();
        found
    };
    assert_eq!(only(&["MD001"]), ["MD001"]);
    assert_eq!(only(&["no-trailing-spaces", "md001"]), ["MD001", "MD009"]);
    let headings = only(&["headings"]);
    assert!(headings.contains(&"MD022".to_owned()), "{headings:?}");
    assert!(!headings.contains(&"MD009".to_owned()), "{headings:?}");
}

#[test]
fn rule_enables_rules_the_config_turns_off() {
    let mrq = Mrq::new();
    let config = ["--config", "configs/no-headings.yaml", "--format", "json"];
    let all = codes(&mrq.stdout(&[&config[..], &["docs/mixed.md"]].concat(), 0));
    assert_eq!(all, []);
    let md001 = codes(&mrq.stdout(
        &[&config[..], &["--rule", "MD001", "docs/mixed.md"]].concat(),
        1,
    ));
    assert_eq!(md001, [("docs/mixed.md".to_owned(), "MD001".to_owned())]);
}

#[test]
fn an_unknown_rule_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["lint", "--rule", "MD999", "clean.md"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("MD999"));
}

#[test]
fn a_missing_path_is_a_usage_error() {
    Mrq::new()
        .cmd()
        .args(["lint", "nope.md"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("nope.md"));
}

#[test]
fn markdownlint_config_is_the_fallback() {
    let mrq = Mrq::writable();
    fs::remove_file(mrq.dir.join(".mrq.yaml")).unwrap();
    mrq.write(
        ".markdownlint.json",
        r#"{ "MD001": false, "MD022": false }"#,
    );
    let out = mrq.stdout(&["--format", "json", "docs/mixed.md"], 0);
    assert_eq!(codes(&out), []);
}

#[test]
fn fix_rewrites_files_and_reports_what_is_left() {
    let mrq = Mrq::writable();
    let before = mrq.read("clean.md");
    let out = mrq.stdout(&["--fix", "--format", "json", "."], 1);
    assert_eq!(
        codes(&out),
        [("docs/mixed.md".to_owned(), "MD001".to_owned())]
    );
    assert_eq!(mrq.read("docs/fixable.md"), FIXED);
    assert_eq!(
        mrq.read("docs/mixed.md"),
        "# Mixed\n\n### Skipped level\n\nText.\n"
    );
    assert_eq!(mrq.read("clean.md"), before);
    // Atomic writes leave no temp files behind.
    let mut names: Vec<String> = fs::read_dir(mrq.dir.join("docs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["fixable.md", "mixed.md"]);

    // Fixed files lint clean, apart from what has no fix.
    let again = mrq.stdout(&["--fix", "--format", "json", "docs/fixable.md"], 0);
    assert_eq!(codes(&again), []);
}

#[test]
fn fix_honours_rule() {
    let mrq = Mrq::writable();
    mrq.stdout(&["--fix", "--rule", "MD009", "docs/fixable.md"], 0);
    assert_eq!(
        mrq.read("docs/fixable.md"),
        "# Fixable\nText with trailing spaces.\n* one\n* two\n"
    );
}

#[test]
fn fix_only_fixes_and_reports_nothing() {
    let mrq = Mrq::writable();
    let out = mrq.output(&["--fix-only", "docs"], None);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(mrq.read("docs/fixable.md"), FIXED);
}

#[test]
fn diff_shows_fixes_without_writing() {
    let mrq = Mrq::writable();
    let out = mrq.stdout(&["--diff", "docs", "clean.md"], 1);
    insta::assert_snapshot!("docs_diff", out);
    assert_eq!(
        mrq.read("docs/fixable.md"),
        fs::read_to_string(fixtures().join("docs/fixable.md")).unwrap()
    );
    let clean = mrq.output(&["--diff", "clean.md"], None);
    assert_eq!(clean.status.code(), Some(0), "{clean:?}");
    assert!(clean.stdout.is_empty(), "{clean:?}");
}

#[test]
fn diff_conflicts_with_fix() {
    Mrq::new()
        .cmd()
        .args(["lint", "--diff", "--fix", "clean.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn stdin_is_linted_as_stdin() {
    let out = Mrq::new().output(&["--format", "json", "-"], Some("# A\ntext\n"));
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let found = codes(&normalize(&out.stdout));
    assert_eq!(found, [("<stdin>".to_owned(), "MD022".to_owned())]);
}

#[test]
fn stdin_fix_prints_the_fixed_text_and_reports_on_stderr() {
    let mrq = Mrq::new();
    let out = mrq.output(
        &["--fix", "--format", "json", "-"],
        Some("# A\n### B\ntext"),
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(normalize(&out.stdout), "# A\n\n### B\n\ntext\n");
    assert_eq!(
        codes(&normalize(&out.stderr)),
        [("<stdin>".to_owned(), "MD001".to_owned())]
    );

    let out = mrq.output(&["--fix-only", "-"], Some("# A\n### B\ntext"));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(normalize(&out.stdout), "# A\n\n### B\n\ntext\n");
    assert!(out.stderr.is_empty(), "{out:?}");

    let out = mrq.output(&["--fix", "-"], Some("# Clean\n"));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(normalize(&out.stdout), "# Clean\n");
}

#[test]
fn stdin_diff_names_stdin() {
    let out = Mrq::new().output(&["--diff", "-"], Some("# A\ntext\n"));
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let diff = normalize(&out.stdout);
    assert!(diff.starts_with("--- a/<stdin>\n+++ b/<stdin>\n"), "{diff}");
}
