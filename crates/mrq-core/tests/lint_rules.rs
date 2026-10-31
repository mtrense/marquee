//! The built-in rules against the fixtures in `tests/fixtures/lint/`.
//!
//! Each `<rule>/<name>.md` is loaded from its path (so links and includes resolve
//! relative to it, against files in subdirectories, which are not fixtures) and linted
//! with only `<rule>` enabled; rule options come from `markdownlint-configure-file`
//! comments inside the fixture, which markdownlint reads too. For every fixture:
//!
//! - `pass*.md` must be clean, `fail*.md` must not;
//! - the findings and the text after applying every fix are an insta snapshot;
//! - the fixed text must have no fixable findings left;
//! - `<name>.markdownlint.txt` (what markdownlint-cli2 reported, recorded by
//!   `cargo xtask lint-parity`) must match mrq's findings: line, column where
//!   markdownlint gives one, rule and message; `<name>.markdownlint.fixed` (its `--fix`
//!   output, absent when nothing changed) must match the fixed text. Every `MD###`
//!   fixture needs the recording; `MRQ###` rules have no markdownlint counterpart, so
//!   their fixtures have none and are checked by the snapshot alone.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::config::{LintConfig, LintExtends};
use mrq_core::diag::Diagnostic;
use mrq_core::lint::{Linter, Registry};
use mrq_core::source::{PositionEncoding, TextEdit};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lint")
}

/// `(rule id, fixture path)` for every fixture, sorted.
fn fixtures() -> Vec<(String, PathBuf)> {
    let mut fixtures = Vec::new();
    for dir in std::fs::read_dir(fixture_root()).unwrap() {
        let dir = dir.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        let rule = dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_ascii_uppercase();
        for file in std::fs::read_dir(&dir).unwrap() {
            let file = file.unwrap().path();
            if file.extension().is_some_and(|ext| ext == "md") {
                fixtures.push((rule.clone(), file));
            }
        }
    }
    fixtures.sort();
    fixtures
}

fn linter(rule: &str) -> Linter {
    let rules = serde_json::from_value(serde_json::json!({ rule: true })).unwrap();
    let config = LintConfig {
        extends: LintExtends::None,
        rules,
    };
    Linter::new(&Registry::builtin(), &config).unwrap()
}

/// 1-based line and column (UTF-16 units, like markdownlint) of `offset`.
fn position(doc: &Document, offset: usize) -> (usize, usize) {
    let position = doc
        .source()
        .offset_to_position(offset, PositionEncoding::Utf16)
        .unwrap();
    (position.line + 1, position.col + 1)
}

/// `line:column RULE message` per finding, sorted by position.
fn findings(doc: &Document, diagnostics: &[Diagnostic]) -> Vec<(usize, usize, String)> {
    let mut findings: Vec<_> = diagnostics
        .iter()
        .map(|diag| {
            let (line, col) = position(doc, diag.range.start);
            (line, col, format!("{} {}", diag.code, diag.message))
        })
        .collect();
    findings.sort();
    findings
}

/// The text after applying every fix whose edits do not overlap an earlier one. Like
/// markdownlint, identical fixes (two findings inserting the same blank line) apply
/// once.
fn apply_fixes(doc: &Document, diagnostics: &[Diagnostic]) -> String {
    let mut fixes: Vec<&Vec<TextEdit>> = diagnostics
        .iter()
        .filter_map(|diag| diag.fix.as_ref().map(|fix| &fix.edits))
        .collect();
    fixes.sort_by_key(|edits| edits.iter().map(|edit| edit.range.start).min());
    fixes.dedup();
    let mut edits = Vec::new();
    let mut end = 0;
    for fix in fixes {
        let start = fix.iter().map(|edit| edit.range.start).min().unwrap_or(end);
        if start < end && !edits.is_empty() {
            continue;
        }
        end = fix.iter().map(|edit| edit.range.end).max().unwrap_or(start);
        edits.extend(fix.iter().cloned());
    }
    doc.source().apply_edits(&edits).unwrap().text().to_owned()
}

/// Parses `line[:column] RULE message` lines recorded from markdownlint-cli2.
fn recorded(text: &str) -> Vec<(usize, Option<usize>, String)> {
    let mut findings: Vec<_> = text
        .lines()
        .map(|line| {
            let (position, rest) = line.split_once(' ').unwrap();
            let (line, col) = match position.split_once(':') {
                Some((line, col)) => (line.parse().unwrap(), Some(col.parse().unwrap())),
                None => (position.parse().unwrap(), None),
            };
            (line, col, rest.to_owned())
        })
        .collect();
    findings.sort();
    findings
}

#[test]
fn rule_fixtures() {
    let mut failures = String::new();
    let mut snapshots = Vec::new();
    for (rule, path) in fixtures() {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let doc = Document::load(&path).unwrap();
        let linter = linter(&rule);
        let diagnostics = linter.check(&doc);
        let found = findings(&doc, &diagnostics);
        let fixed = apply_fixes(&doc, &diagnostics);
        let label = format!("{}/{name}", rule.to_ascii_lowercase());

        if name.starts_with("pass") && !found.is_empty() {
            let _ = writeln!(failures, "{label}: expected no findings, got {found:?}");
        }
        if name.starts_with("fail") && found.is_empty() {
            let _ = writeln!(failures, "{label}: expected findings, got none");
        }
        let refixed = linter.check(&Document::parse(fixed.clone()));
        if refixed.iter().any(|diag| diag.fix.is_some()) {
            let _ = writeln!(
                failures,
                "{label}: fixed text still has fixable findings: {refixed:?}"
            );
        }

        let recording = std::fs::read_to_string(path.with_extension("markdownlint.txt"));
        let markdownlint_rule = !rule.starts_with("MRQ");
        if markdownlint_rule != recording.is_ok() {
            let _ = writeln!(
                failures,
                "{label}: {}",
                if markdownlint_rule {
                    "no markdownlint recording, run `cargo xtask lint-parity`"
                } else {
                    "mrq rules have no markdownlint recording"
                }
            );
        }
        if let Ok(expected) = recording {
            let expected = recorded(&expected);
            // Columns count only where markdownlint reports one (not for whole lines), so
            // findings pair up by line and message first.
            let mut found = found.clone();
            found.sort_by(|a, b| (a.0, &a.2, a.1).cmp(&(b.0, &b.2, b.1)));
            let mut expected = expected;
            expected.sort_by(|a, b| (a.0, &a.2, a.1).cmp(&(b.0, &b.2, b.1)));
            let actual: Vec<_> = found
                .iter()
                .zip(
                    expected
                        .iter()
                        .map(|(_, col, _)| *col)
                        .chain(std::iter::repeat(Some(0))),
                )
                .map(|((line, col, message), recorded_col)| {
                    (*line, recorded_col.map(|_| *col), message.clone())
                })
                .collect();
            if actual != expected {
                let _ = writeln!(
                    failures,
                    "{label}: differs from markdownlint\n  mrq:          {actual:?}\n  markdownlint: {expected:?}"
                );
            }
            let expected_fixed = std::fs::read_to_string(path.with_extension("markdownlint.fixed"))
                .unwrap_or_else(|_| text.clone());
            if fixed != expected_fixed {
                let _ = writeln!(
                    failures,
                    "{label}: fix differs from markdownlint\n  mrq:          {fixed:?}\n  markdownlint: {expected_fixed:?}"
                );
            }
        }

        let mut snapshot = String::new();
        for (line, col, message) in &found {
            let _ = writeln!(snapshot, "{line}:{col} {message}");
        }
        if fixed != text {
            snapshot.push_str("--- fixed ---\n");
            snapshot.push_str(&fixed);
        }
        snapshots.push((label.replace('/', "_"), snapshot));
    }
    assert!(failures.is_empty(), "{failures}");
    for (name, snapshot) in snapshots {
        insta::assert_snapshot!(name, snapshot);
    }
}

#[test]
fn every_builtin_rule_has_fixtures() {
    let fixtures = fixtures();
    for meta in Registry::builtin().rules() {
        for kind in ["pass", "fail"] {
            assert!(
                fixtures.iter().any(|(rule, path)| rule == meta.id
                    && path
                        .file_stem()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .starts_with(kind)),
                "{} has no {kind} fixture in tests/fixtures/lint/{}/",
                meta.id,
                meta.id.to_ascii_lowercase()
            );
        }
    }
}
