//! `mrq query` with each front-end and output mode, over a file, a directory and stdin.
//!
//! Every test runs `mrq` in `tests/fixtures/query` with `MRQ_CONFIG_HOME` pointing at an
//! empty temp directory, so no user config leaks in.

use std::path::PathBuf;
use std::process::Output;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/query")
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

    fn output(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = self.cmd();
        cmd.arg("query").args(args);
        if let Some(stdin) = stdin {
            cmd.write_stdin(stdin);
        }
        cmd.output().unwrap()
    }

    /// Stdout of a successful `mrq query`, with Windows path separators turned into `/`.
    fn stdout(&self, args: &[&str], stdin: Option<&str>) -> String {
        let out = self.output(args, stdin);
        assert!(out.status.success(), "mrq query {args:?} failed: {out:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        if cfg!(windows) {
            text.replace("\\\\", "/").replace('\\', "/")
        } else {
            text
        }
    }
}

#[test]
fn md_prints_sections_in_full() {
    let out = Mrq::new().stdout(&["# Install", "README.md"], None);
    insta::assert_snapshot!("md_section", out);
}

#[test]
fn md_separates_matches_with_a_blank_line() {
    let out = Mrq::new().stdout(&["- [ ]", "README.md"], None);
    assert_eq!(out, "- [ ] write docs\n\n- [ ] add tests\n");
}

#[test]
fn md_code_blocks_in_a_section() {
    let out = Mrq::new().stdout(&["# Install | ```sh", "README.md"], None);
    insta::assert_snapshot!("md_code_in_section", out);
}

#[test]
fn text_prints_plain_text_one_match_per_line() {
    let out = Mrq::new().stdout(&["-o", "text", "[]()", "README.md"], None);
    assert_eq!(out, "home link\nthe guide\n");
}

#[test]
fn json_is_one_entry_per_file() {
    let out = Mrq::new().stdout(&["--output", "json", "- [ ]", "README.md"], None);
    insta::assert_snapshot!("json_tasks", out);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json[0]["file"], "README.md");
    assert_eq!(json[0]["matches"].as_array().unwrap().len(), 2);
    assert_eq!(json[0]["matches"][0]["type"], "listItem");
}

#[test]
fn json_sections_are_section_objects() {
    let out = Mrq::new().stdout(&["-o", "json", "# Notes", "README.md"], None);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json[0]["matches"][0]["type"], "section");
    assert_eq!(json[0]["matches"][0]["depth"], 2);
}

#[test]
fn css_front_end() {
    let out = Mrq::new().stdout(
        &[
            "--css",
            r#"section[heading="Install"] code[lang=rust]"#,
            "README.md",
        ],
        None,
    );
    assert_eq!(out, "```rust\nfn main() {}\n```\n");
}

#[test]
fn jq_values_print_raw_strings_and_json() {
    let mrq = Mrq::new();
    let urls = mrq.stdout(
        &[
            "--jq",
            r#".. | select(.type? == "link") | .url"#,
            "README.md",
        ],
        None,
    );
    assert_eq!(urls, "https://example.com\ndocs/guide.md\n");
    let numbers = mrq.stdout(&["--jq", "1, [2]", "-o", "text", "README.md"], None);
    assert_eq!(numbers, "1\n[2]\n");
}

#[test]
fn jq_nodes_print_their_source() {
    let mrq = Mrq::new();
    let md = mrq.stdout(
        &["--jq", r#".. | select(.type? == "link")"#, "README.md"],
        None,
    );
    assert_eq!(
        md,
        "[home link](https://example.com)\n\n[the guide](docs/guide.md)\n"
    );
    let text = mrq.stdout(
        &[
            "--jq",
            "sections | select(.depth == 2)",
            "-o",
            "text",
            "docs/guide.md",
        ],
        None,
    );
    assert_eq!(text, "Install\nbrew install mrq\n");
}

#[test]
fn jq_json_keeps_any_value() {
    let out = Mrq::new().stdout(&["--jq", r#"1, "a""#, "-o", "json", "-"], Some("x\n"));
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{ "file": null, "matches": [1, "a"] }])
    );
}

#[test]
fn directory_has_file_headers_for_files_with_matches() {
    let mrq = Mrq::new();
    insta::assert_snapshot!(
        "directory_md",
        mrq.stdout(&["[]()", "docs", "README.md"], None)
    );
    insta::assert_snapshot!(
        "directory_text",
        mrq.stdout(&["-o", "text", "# Install", "docs", "README.md"], None)
    );
}

#[test]
fn directory_json_lists_every_file() {
    let out = Mrq::new().stdout(&["-o", "json", "[]()", "docs"], None);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let files: Vec<_> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["file"].as_str().unwrap().replace('\\', "/"),
                entry["matches"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(
        files,
        [
            ("docs/guide.md".to_owned(), 1),
            ("docs/plain.md".to_owned(), 0)
        ]
    );
}

#[test]
fn stdin() {
    let mrq = Mrq::new();
    let doc = "# A\n\none\n\n# B\n\ntwo\n";
    assert_eq!(mrq.stdout(&["# B", "-"], Some(doc)), "# B\n\ntwo\n");
    assert_eq!(
        mrq.stdout(&["# B", "README.md", "-"], Some(doc)),
        "==> <stdin> <==\n# B\n\ntwo\n"
    );
}

#[test]
fn no_match_exits_0_unless_exit_status_is_set() {
    let mrq = Mrq::new();
    let out = mrq.output(&["# Nowhere", "README.md"], None);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"");
    let out = mrq.output(&["--exit-status", "# Nowhere", "README.md"], None);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"");
    let out = mrq.output(
        &["--exit-status", "-o", "json", "# Nowhere", "README.md"],
        None,
    );
    assert_eq!(out.status.code(), Some(1));
    let out = mrq.output(&["-e", "# Install", "docs/plain.md", "README.md"], None);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn syntax_errors_show_a_caret_and_exit_2() {
    let out = Mrq::new().output(&["# A | ?", "README.md"], None);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout, b"");
    insta::assert_snapshot!("syntax_error", String::from_utf8(out.stderr).unwrap());
    for args in [
        &["--css", "heading[", "README.md"][..],
        &["--jq", ".[", "README.md"],
    ] {
        let out = Mrq::new().output(args, None);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains('^'), "{stderr}");
    }
}

#[test]
fn syntax_errors_do_not_read_inputs() {
    Mrq::new()
        .cmd()
        .args(["query", "# A | ?", "nope.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nope.md").not());
}

#[test]
fn jq_runtime_errors_are_reported_per_file_and_exit_2() {
    let out = Mrq::new().output(&["--jq", r#"error("boom")"#, "docs"], None);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap().replace('\\', "/");
    assert_eq!(
        stderr,
        "mrq: docs/guide.md: jq error: boom\nmrq: docs/plain.md: jq error: boom\n"
    );
}

#[test]
fn missing_paths_exit_2() {
    Mrq::new()
        .cmd()
        .args(["query", "# A", "nope.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nope.md"));
}
