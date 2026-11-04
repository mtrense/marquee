//! Completion quality: every value-taking argument of the clap tree is either a path
//! with a path `ValueHint`, a closed set with documented possible values, or a known
//! free-form value; plus snapshots of the generated bash, zsh and fish scripts.

use std::any::TypeId;
use std::path::PathBuf;

use assert_cmd::Command;
use clap::ValueHint;

/// Free-form value arguments (`subcommand path`, `arg id`): neither paths nor closed
/// sets. A new value argument must be added here deliberately, or get a path hint or
/// possible values.
const FREE_FORM: &[(&str, &str)] = &[
    ("mrq toc", "depth"),
    ("mrq render", "style"),
    ("mrq render", "width"),
    ("mrq lint", "rule"),
    ("mrq query", "query"),
    ("mrq merge", "shift"),
    ("mrq merge", "title"),
    ("mrq split", "depth"),
    ("mrq stats", "wpm"),
];

/// Path arguments that take files and directories alike, so `ValueHint::AnyPath` is
/// right. clap infers `AnyPath` for every `PathBuf`, so any other path argument must say
/// `FilePath` or `DirPath` explicitly.
const ANY_PATH: &[&str] = &["paths"];

fn walk(cmd: &clap::Command, path: &str, problems: &mut Vec<String>) {
    for arg in cmd.get_arguments() {
        if !arg.get_num_args().is_some_and(|n| n.takes_values()) {
            continue;
        }
        let id = arg.get_id().as_str();
        if arg.is_global_set() && path != "mrq" {
            continue; // checked where it is declared
        }
        let is_path = arg.get_value_parser().type_id() == TypeId::of::<PathBuf>();
        let possible = arg.get_possible_values();
        if is_path {
            let ok = match arg.get_value_hint() {
                ValueHint::FilePath | ValueHint::DirPath => true,
                ValueHint::AnyPath => ANY_PATH.contains(&id),
                _ => false,
            };
            if !ok {
                problems.push(format!(
                    "{path} {id}: path without a FilePath/DirPath ValueHint ({:?})",
                    arg.get_value_hint()
                ));
            }
        } else if !possible.is_empty() {
            for value in possible.iter().filter(|v| !v.is_hide_set()) {
                if value.get_help().is_none() {
                    problems.push(format!(
                        "{path} {id}: possible value `{}` has no help",
                        value.get_name()
                    ));
                }
            }
        } else if !FREE_FORM.contains(&(path, id)) {
            problems.push(format!(
                "{path} {id}: neither a hinted path, nor a closed set, nor listed as free-form"
            ));
        }
    }
    for sub in cmd.get_subcommands() {
        walk(sub, &format!("{path} {}", sub.get_name()), problems);
    }
}

#[test]
fn every_value_argument_completes_values() {
    let mut cmd = mrq_cli::cli::command();
    cmd.build();
    let mut problems = Vec::new();
    walk(&cmd, "mrq", &mut problems);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn completions(shell: &str) -> String {
    let out = Command::cargo_bin("mrq")
        .unwrap()
        .args(["completions", shell])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn bash_snapshot() {
    insta::assert_snapshot!("bash", completions("bash"));
}

#[test]
fn zsh_snapshot() {
    insta::assert_snapshot!("zsh", completions("zsh"));
}

#[test]
fn fish_snapshot() {
    insta::assert_snapshot!("fish", completions("fish"));
}
