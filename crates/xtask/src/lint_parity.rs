//! Lint parity with markdownlint: records what `markdownlint-cli2` reports for the lint
//! fixtures, which `crates/mrq-core/tests/lint_rules.rs` then compares with mrq.
//!
//! Every `<root>/<rule>/<name>.md` is linted with only `<rule>` enabled (options come from
//! `markdownlint-configure-file` comments in the fixture). The findings go to
//! `<name>.markdownlint.txt`, one `line[:column] RULE message` per line, and the output
//! of `--fix` to `<name>.markdownlint.fixed` when it differs from the fixture.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The markdownlint-cli2 release the recorded files come from.
pub const DEFAULT_COMMAND: &str = "npx --yes markdownlint-cli2@0.24.0";

/// What one fixture produced.
#[derive(Debug, PartialEq, Eq)]
pub struct Recorded {
    /// Normalized findings, one per line.
    pub findings: String,
    /// The fixed text, if `--fix` changed anything.
    pub fixed: Option<String>,
}

/// Runs markdownlint-cli2 (`command`, split on whitespace) on every fixture under `root`.
/// Writes the recorded files, or with `check` only compares them; returns the fixtures
/// whose recording is new or changed.
pub fn run(root: &Path, command: &str, check: bool) -> io::Result<Vec<PathBuf>> {
    let mut changed = Vec::new();
    for fixture in fixtures(root)? {
        let rule = fixture
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::to_ascii_uppercase)
            .unwrap_or_default();
        let recorded = record(&fixture, &rule, command)?;
        let findings_path = fixture.with_extension("markdownlint.txt");
        let fixed_path = fixture.with_extension("markdownlint.fixed");
        let old = Recorded {
            findings: std::fs::read_to_string(&findings_path).unwrap_or_default(),
            fixed: std::fs::read_to_string(&fixed_path).ok(),
        };
        if old == recorded && findings_path.exists() {
            continue;
        }
        changed.push(fixture.clone());
        if check {
            continue;
        }
        std::fs::write(&findings_path, &recorded.findings)?;
        match &recorded.fixed {
            Some(fixed) => std::fs::write(&fixed_path, fixed)?,
            None if fixed_path.exists() => std::fs::remove_file(&fixed_path)?,
            None => {}
        }
    }
    Ok(changed)
}

/// Every `*.md` one directory below `root`, sorted.
fn fixtures(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut fixtures = Vec::new();
    for dir in std::fs::read_dir(root)? {
        let dir = dir?.path();
        if !dir.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(&dir)? {
            let file = file?.path();
            if file.extension().is_some_and(|ext| ext == "md") {
                fixtures.push(file);
            }
        }
    }
    fixtures.sort();
    Ok(fixtures)
}

/// Lints and fixes a copy of `fixture` in a temporary directory with only `rule` on.
fn record(fixture: &Path, rule: &str, command: &str) -> io::Result<Recorded> {
    let dir = tempfile::tempdir()?;
    let name = fixture
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("fixture name is not UTF-8"))?;
    let text = std::fs::read_to_string(fixture)?;
    let copy = dir.path().join(name);
    std::fs::write(&copy, &text)?;
    std::fs::write(
        dir.path().join(".markdownlint-cli2.jsonc"),
        format!("{{ \"config\": {{ \"default\": false, \"{rule}\": true }} }}\n"),
    )?;
    let output = cli(command, dir.path(), &[name])?;
    let findings = normalize(&output, name);
    cli(command, dir.path(), &["--fix", name])?;
    let fixed = std::fs::read_to_string(&copy)?;
    Ok(Recorded {
        findings,
        fixed: (fixed != text).then_some(fixed),
    })
}

/// Runs the CLI in `dir` and returns stdout and stderr together; findings exit 1.
fn cli(command: &str, dir: &Path, args: &[&str]) -> io::Result<String> {
    let mut words = command.split_whitespace();
    let program = words
        .next()
        .ok_or_else(|| io::Error::other("empty markdownlint-cli2 command"))?;
    let output = Command::new(program)
        .args(words)
        .args(args)
        .current_dir(dir)
        .output()?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(io::Error::other(format!(
            "`{command}` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

/// `name:12:3 error MD004/ul-style Message` → `12:3 MD004 Message`, in output order.
fn normalize(output: &str, name: &str) -> String {
    let prefix = format!("{name}:");
    let mut findings = String::new();
    for line in output.lines() {
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        let Some((position, rest)) = rest.split_once(' ') else {
            continue;
        };
        let rest = rest
            .strip_prefix("error ")
            .or_else(|| rest.strip_prefix("warning "))
            .unwrap_or(rest);
        let Some((names, message)) = rest.split_once(' ') else {
            continue;
        };
        let id = names.split('/').next().unwrap_or(names);
        let _ = writeln!(findings, "{position} {id} {message}");
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn normalize_keeps_position_id_and_message() {
        let output = "Linting: 1 file\n\
                      a.md:9:3 error MD004/ul-style Unordered list style [Expected: dash; Actual: plus]\n\
                      a.md:5 MD003/heading-style Heading style\n\
                      b.md:1 error MD001/heading-increment x\n";
        assert_eq!(
            normalize(output, "a.md"),
            "9:3 MD004 Unordered list style [Expected: dash; Actual: plus]\n5 MD003 Heading style\n"
        );
    }
}
