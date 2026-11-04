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

#[test]
fn nushell_snapshot() {
    insta::assert_snapshot!("nushell", completions("nushell"));
}

/// Whether `program` is on `PATH`.
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|dir| dir.join(program).is_file() || dir.join(format!("{program}.exe")).is_file())
    })
}

/// Asks nu's completion engine (`nu --ide-complete`) for the completions at the end of
/// `line`, with the generated script sourced first.
fn nu_complete(dir: &std::path::Path, line: &str) -> Vec<String> {
    let query = dir.join("query.nu");
    let text = format!("source mrq.nu\n{line}");
    std::fs::write(&query, &text).unwrap();
    let out = std::process::Command::new("nu")
        .current_dir(dir)
        .args([
            "--no-config-file",
            "--ide-complete",
            &text.len().to_string(),
            "query.nu",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    json["completions"]
        .as_array()
        .unwrap_or_else(|| panic!("no completions for `{line}`: {json}"))
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn nushell_script_sources_and_completes_values() {
    if !on_path("nu") {
        eprintln!("skipped: `nu` is not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mrq.nu"), completions("nushell")).unwrap();

    // Every subcommand is a known extern.
    let out = std::process::Command::new("nu")
        .current_dir(dir.path())
        .args([
            "--no-config-file",
            "-c",
            "source mrq.nu; scope commands | where name =~ '^mrq' | get name | to json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let known: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
    for sub in mrq_cli::cli::command().get_subcommands() {
        let name = format!("mrq {}", sub.get_name());
        assert!(known.contains(&name), "{name} missing from {known:?}");
    }

    let colors = nu_complete(dir.path(), "mrq --color a");
    assert_eq!(colors, ["always", "auto"]);
    let formats = nu_complete(dir.path(), "mrq render -o ");
    for format in ["html", "ansi", "md", "json", "man"] {
        assert!(
            formats.iter().any(|f| f == format),
            "{format} missing from {formats:?}"
        );
    }
    let shells = nu_complete(dir.path(), "mrq completions n");
    assert!(shells.iter().any(|s| s == "nushell"), "{shells:?}");
}

#[test]
fn carapace_snapshot() {
    insta::assert_snapshot!("carapace", completions("carapace"));
}

/// The carapace spec validates against carapace-spec's JSON Schema, vendored from
/// <https://github.com/carapace-sh/carapace-spec/blob/f87190765a152f65a11163c0fb81428d21e49d45/schema.json>
/// (commit f871907 of 2026-11-03).
#[test]
fn carapace_spec_matches_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/carapace/schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let spec: serde_json::Value = serde_saphyr::from_str(&completions("carapace")).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&spec)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    assert_eq!(spec["name"], "mrq");
}

/// Runs `carapace mrq export '' <args>` in `dir`, with the spec installed into
/// `dir/config`, and returns the completion values.
fn carapace_complete(dir: &std::path::Path, args: &[&str]) -> Vec<String> {
    let config = dir.join("config");
    let out = std::process::Command::new("carapace")
        .current_dir(dir.join("work"))
        .args(["mrq", "export", ""])
        .args(args)
        // `XDG_CONFIG_HOME` wins over the platform config dir, macOS included; `HOME`
        // keeps the user's own carapace config out of the test.
        .env("XDG_CONFIG_HOME", &config)
        .env("HOME", &config)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    let mut values: Vec<String> = json["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no values for {args:?}: {json}"))
        .iter()
        .map(|v| v["value"].as_str().unwrap().to_owned())
        .collect();
    values.sort();
    values
}

#[test]
fn carapace_spec_completes_values() {
    if !on_path("carapace") {
        eprintln!("skipped: `carapace` is not on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let specs = dir.path().join("config/carapace/specs");
    std::fs::create_dir_all(&specs).unwrap();
    std::fs::write(specs.join("mrq.yaml"), completions("carapace")).unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join("docs")).unwrap();
    for file in ["a.md", "b.markdown", "c.txt"] {
        std::fs::write(work.join(file), "").unwrap();
    }

    let formats = carapace_complete(dir.path(), &["render", "-o", ""]);
    assert_eq!(formats, ["ansi", "html", "json", "man", "md", "pdf"]);
    let colors = carapace_complete(dir.path(), &["--color", ""]);
    assert_eq!(colors, ["always", "auto", "never"]);
    let subcommands = carapace_complete(dir.path(), &[""]);
    for sub in mrq_cli::cli::command().get_subcommands() {
        assert!(
            subcommands.iter().any(|s| s == sub.get_name()),
            "{} missing from {subcommands:?}",
            sub.get_name()
        );
    }
    // Markdown inputs complete Markdown files and directories; other paths any file.
    let inputs = carapace_complete(dir.path(), &["lint", ""]);
    assert_eq!(inputs, ["a.md", "b.markdown", "docs/"]);
    let after_query = carapace_complete(dir.path(), &["query", "heading", ""]);
    assert_eq!(after_query, ["a.md", "b.markdown", "docs/"]);
    let out = carapace_complete(dir.path(), &["render", "--out", ""]);
    assert_eq!(out, ["a.md", "b.markdown", "c.txt", "docs/"]);
}
