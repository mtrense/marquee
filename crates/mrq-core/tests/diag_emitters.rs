//! Snapshot tests for every diagnostics emitter, plus SARIF schema validation.

use std::path::PathBuf;

use mrq_core::diag::{
    Diagnostic, Fix, Label, OutputFormat, Severity, ToolInfo, emit, emit_github, emit_json,
    emit_ndjson, emit_plain, emit_sarif,
};
use mrq_core::source::{ByteRange, SourceFile, TextEdit};

const GUIDE: &str = "# Guide\n\nSee [setup](missing.md) for details.\nA very long line with ünïcödé that goes on 🦀 and on.\n";
const README: &str = "# Readme\r\n\r\nIntro paragraph.\r\n## Usage\r\n";

fn sources() -> Vec<SourceFile> {
    vec![
        SourceFile::new(Some(PathBuf::from("docs/guide.md")), GUIDE),
        SourceFile::new(Some(PathBuf::from("README.md")), README),
    ]
}

fn find(text: &str, needle: &str) -> ByteRange {
    let start = text.find(needle).expect("needle in text");
    ByteRange::new(start, start + needle.len())
}

fn diagnostics() -> Vec<Diagnostic> {
    let long_line = find(
        GUIDE,
        "A very long line with ünïcödé that goes on 🦀 and on.",
    );
    let crab = find(GUIDE, "🦀");
    vec![
        Diagnostic::new(
            "MRQ001",
            Severity::Error,
            "docs/guide.md",
            find(GUIDE, "missing.md"),
            "link target `missing.md` does not exist",
        )
        .with_label(Label::new(find(GUIDE, "[setup]"), "linked from here"))
        .with_help("create the file or fix the link")
        .with_url("https://example.com/rules/MRQ001"),
        Diagnostic::new(
            "MD013",
            Severity::Warning,
            "docs/guide.md",
            long_line,
            "line length exceeds 40 characters",
        )
        .with_fix(Fix::new(
            "replace the crab",
            vec![TextEdit {
                range: crab,
                replacement: "crab".into(),
            }],
        )),
        Diagnostic::new(
            "MD001",
            Severity::Info,
            "README.md",
            find(README, "## Usage"),
            "heading levels should only increment by one",
        )
        .with_label(Label::new(find(README, "# Readme"), "previous heading")),
        Diagnostic::new(
            "MRQ999",
            Severity::Hint,
            "docs/gone file.md",
            ByteRange::new(3, 7),
            "50% done: a, b\nsecond line",
        ),
    ]
}

fn render(f: impl FnOnce(&mut Vec<u8>) -> std::io::Result<()>) -> String {
    let mut out = Vec::new();
    f(&mut out).expect("emitting succeeds");
    String::from_utf8(out).expect("utf-8 output")
}

fn tool() -> ToolInfo {
    ToolInfo {
        name: "mrq".into(),
        version: "0.0.0-test".into(),
        information_uri: Some("https://github.com/mtrense/marquee".into()),
    }
}

#[test]
fn json_snapshot() {
    let out = render(|w| emit_json(&diagnostics(), &sources(), w));
    insta::assert_snapshot!(out);
}

#[test]
fn ndjson_snapshot() {
    let out = render(|w| emit_ndjson(&diagnostics(), &sources(), w));
    assert_eq!(out.lines().count(), diagnostics().len());
    insta::assert_snapshot!(out);
}

#[test]
fn github_snapshot() {
    let out = render(|w| emit_github(&diagnostics(), &sources(), w));
    insta::assert_snapshot!(out);
}

#[test]
fn plain_snapshot() {
    let out = render(|w| emit_plain(&diagnostics(), &sources(), w));
    insta::assert_snapshot!(out);
}

#[test]
fn sarif_snapshot() {
    let out = render(|w| emit_sarif(&diagnostics(), &sources(), &tool(), w));
    insta::assert_snapshot!(out);
}

#[test]
fn sarif_validates_against_2_1_0_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("schemas/sarif-2.1.0.json")).expect("schema is JSON");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    assert!(
        !validator.is_valid(&serde_json::json!({"version": "2.0.0", "runs": []})),
        "the validator must reject a non-2.1.0 log"
    );
    for diags in [diagnostics(), Vec::new()] {
        let out = render(|w| emit_sarif(&diags, &sources(), &tool(), w));
        let instance: serde_json::Value = serde_json::from_str(&out).expect("SARIF is JSON");
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|e| format!("{e} at {}", e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "SARIF schema violations: {errors:#?}");
    }
}

#[test]
fn json_output_round_trips_as_json() {
    let out = render(|w| emit_json(&diagnostics(), &sources(), w));
    let value: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(value.as_array().map(Vec::len), Some(4));
    let empty = render(|w| emit_json(&[], &sources(), w));
    assert_eq!(empty.trim(), "[]");
}

#[test]
fn emit_dispatches_on_format() {
    let diags = diagnostics();
    let srcs = sources();
    let options = mrq_core::diag::EmitOptions {
        tool: tool(),
        ..Default::default()
    };
    let via = |format| render(|w| emit(format, &diags, &srcs, &options, w));
    assert_eq!(
        via(OutputFormat::Json),
        render(|w| emit_json(&diags, &srcs, w))
    );
    assert_eq!(
        via(OutputFormat::Ndjson),
        render(|w| emit_ndjson(&diags, &srcs, w))
    );
    assert_eq!(
        via(OutputFormat::Github),
        render(|w| emit_github(&diags, &srcs, w))
    );
    assert_eq!(
        via(OutputFormat::Sarif),
        render(|w| emit_sarif(&diags, &srcs, &tool(), w))
    );
    assert!(via(OutputFormat::Human).contains("MRQ001"));
}

#[cfg(feature = "human")]
mod human {
    use super::*;
    use mrq_core::diag::{HumanStyle, emit_human};

    #[test]
    fn human_ascii_snapshot() {
        let style = HumanStyle {
            color: false,
            unicode: false,
            width: 80,
        };
        let out = render(|w| emit_human(&diagnostics(), &sources(), &style, w));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn human_unicode_snapshot() {
        let style = HumanStyle {
            color: false,
            unicode: true,
            width: 80,
        };
        let out = render(|w| emit_human(&diagnostics()[..1], &sources(), &style, w));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn human_color_emits_ansi_escapes_only_when_enabled() {
        let render_with = |color, unicode| {
            let style = HumanStyle {
                color,
                unicode,
                width: 80,
            };
            render(|w| emit_human(&diagnostics(), &sources(), &style, w))
        };
        assert!(render_with(true, true).contains('\u{1b}'));
        assert!(render_with(true, false).contains('\u{1b}'));
        assert!(!render_with(false, true).contains('\u{1b}'));
        assert!(!render_with(false, false).contains('\u{1b}'));
    }
}
