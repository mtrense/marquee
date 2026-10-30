//! Behaviour of the `mrq` command-line shell: help output, stubbed
//! subcommands, global flags and shell completions.

use assert_cmd::Command;
use predicates::prelude::*;

/// Every subcommand the README documents, plus `completions`.
const SUBCOMMANDS: &[&str] = &[
    "toc",
    "render",
    "validate",
    "lint",
    "fmt",
    "query",
    "merge",
    "interpolate",
    "split",
    "stats",
    "diff",
    "show",
    "lsp",
    "config",
    "completions",
];

fn mrq() -> Command {
    let mut cmd = Command::cargo_bin("mrq").unwrap();
    cmd.env_remove("NO_COLOR").env_remove("CLICOLOR_FORCE");
    cmd
}

fn stdout_of(args: &[&str]) -> String {
    let out = mrq().args(args).output().unwrap();
    assert!(out.status.success(), "mrq {args:?} failed: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn help_lists_every_command() {
    let help = stdout_of(&["--help"]);
    for name in SUBCOMMANDS {
        assert!(
            help.lines()
                .any(|l| l.trim_start().starts_with(&format!("{name} "))),
            "`mrq --help` does not list `{name}`:\n{help}"
        );
    }
}

#[test]
fn help_snapshot() {
    insta::assert_snapshot!("help", stdout_of(&["--help"]));
}

#[test]
fn subcommand_help_snapshots() {
    for name in SUBCOMMANDS {
        insta::assert_snapshot!(format!("help_{name}"), stdout_of(&[name, "--help"]));
    }
    for name in ["show", "schema"] {
        insta::assert_snapshot!(
            format!("help_config_{name}"),
            stdout_of(&["config", name, "--help"])
        );
    }
}

#[test]
fn no_subcommand_is_a_usage_error() {
    mrq()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn stubbed_subcommands_report_not_implemented_and_exit_2() {
    let invocations: &[&[&str]] = &[
        &["render", "-o", "html", "README.md"],
        &["validate"],
        &["lint", "--fix", "docs/"],
        &["fmt", "--check", "."],
        &["query", "# Install", "README.md"],
        &["merge", "a.md", "b.md"],
        &["interpolate", "--in-place", "README.md"],
        &["split", "-d", "2", "guide.md"],
        &["stats"],
        &["diff", "old.md", "new.md"],
        &["show"],
        &["lsp"],
    ];
    for args in invocations {
        mrq()
            .args(*args)
            .assert()
            .code(2)
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::contains("not implemented"));
    }
}

#[test]
fn global_flags_are_accepted_anywhere() {
    mrq()
        .args([
            "--config", "x.yaml", "--flavor", "obsidian", "--color", "never", "--format", "json",
            "-vv", "diff", "old.md", "new.md",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not implemented"));
    mrq()
        .args(["lint", "-q", "--format", "sarif", "--flavor", "mkdocs"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not implemented"));
}

#[test]
fn invalid_global_flag_values_are_rejected() {
    for args in [
        &["--color", "sometimes", "stats"][..],
        &["--flavor", "markdown-it", "stats"][..],
        &["--format", "xml", "stats"][..],
    ] {
        mrq()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("invalid value"));
    }
}

#[test]
fn quiet_and_verbose_conflict() {
    mrq()
        .args(["-q", "-v", "stats"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn query_syntaxes_are_mutually_exclusive() {
    mrq()
        .args(["query", "--css", "--jq", "x", "a.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn completions_are_generated_for_every_shell() {
    for (shell, needle) in [
        ("bash", "_mrq()"),
        ("zsh", "#compdef mrq"),
        ("fish", "complete -c mrq"),
        ("elvish", "edit:completion:arg-completer[mrq]"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        let out = stdout_of(&["completions", shell]);
        assert!(out.contains(needle), "{shell} completions lack {needle:?}");
        assert!(
            out.contains("interpolate"),
            "{shell} completions lack subcommands"
        );
    }
}

#[test]
fn completions_reject_unknown_shell() {
    mrq()
        .args(["completions", "tcsh"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value"));
}
