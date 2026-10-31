//! Fixture tests for `mrq_core::transform::shift_headings`: each
//! `tests/fixtures/transform/shift/<name>.md`, shifted as [`FIXTURES`] says, gives
//! `<name>.out.md` and the diagnostics listed in `<name>.diag.txt` (none if absent).

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use mrq_core::Document;
use mrq_core::ast::NodeKind;
use mrq_core::source::{ByteRange, PositionEncoding, apply_edits};
use mrq_core::transform::{Overflow, ShiftOptions, Shifted, shift_headings};

/// Fixture name, shift and overflow handling.
const FIXTURES: &[(&str, i32, Overflow)] = &[
    ("atx", 1, Overflow::Clamp),
    ("setext", 1, Overflow::Clamp),
    ("clamp-down", 2, Overflow::Clamp),
    ("clamp-up", -1, Overflow::Clamp),
    ("bold", 2, Overflow::Bold),
];

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transform/shift")
}

/// Shifts every heading of `src`.
fn shift(src: &str, options: ShiftOptions) -> (String, Shifted) {
    let doc = Document::parse(src);
    let shifted = shift_headings(&doc, ByteRange::new(0, src.len()), options);
    (apply_edits(src, &shifted.edits).unwrap(), shifted)
}

/// One `line:col code: message` line per diagnostic, 1-based.
fn diagnostic_lines(doc: &Document, shifted: &Shifted) -> String {
    let mut out = String::new();
    for diagnostic in &shifted.diagnostics {
        let position = doc
            .source()
            .offset_to_position(diagnostic.range.start, PositionEncoding::Utf8)
            .unwrap();
        writeln!(
            out,
            "{}:{} {}: {}",
            position.line + 1,
            position.col + 1,
            diagnostic.code,
            diagnostic.message
        )
        .unwrap();
    }
    out
}

#[test]
fn fixtures_shift_to_their_expected_output() {
    for &(name, by, overflow) in FIXTURES {
        let dir = fixture_dir();
        let src = fs::read_to_string(dir.join(format!("{name}.md"))).unwrap();
        let expected = fs::read_to_string(dir.join(format!("{name}.out.md"))).unwrap();
        let diagnostics =
            fs::read_to_string(dir.join(format!("{name}.diag.txt"))).unwrap_or_default();
        let options = ShiftOptions::by(by).overflow(overflow);
        let (out, shifted) = shift(&src, options);
        assert_eq!(out, expected, "{name}");
        assert_eq!(
            diagnostic_lines(&Document::parse(src.as_str()), &shifted),
            diagnostics,
            "{name}: diagnostics"
        );
    }
}

#[test]
fn every_fixture_is_listed() {
    for entry in fs::read_dir(fixture_dir()).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap();
        let name = file.split('.').next().unwrap();
        assert!(
            FIXTURES.iter().any(|&(listed, ..)| listed == name),
            "{file} is not in FIXTURES"
        );
    }
}

#[test]
fn shifting_atx_headings_back_restores_the_source() {
    let src = fs::read_to_string(fixture_dir().join("atx.md")).unwrap();
    let (down, _) = shift(&src, ShiftOptions::by(1));
    let (up, shifted) = shift(&down, ShiftOptions::by(-1));
    assert_eq!(up, src);
    assert_eq!(shifted.diagnostics, vec![]);
}

#[test]
fn bold_headings_parse_as_paragraphs_of_strong_text() {
    let src = fs::read_to_string(fixture_dir().join("bold.md")).unwrap();
    let (out, _) = shift(&src, ShiftOptions::by(2).overflow(Overflow::Bold));
    let doc = Document::parse(out.as_str());
    let ast = doc.ast();
    let count = |want: fn(&Document, mrq_core::ast::NodeId) -> bool| {
        ast.descendants(ast.root())
            .filter(|&id| want(&doc, id))
            .count()
    };
    let headings = count(|doc, id| matches!(doc.ast().kind(id), NodeKind::Heading(_)));
    let bold = count(|doc, id| {
        let ast = doc.ast();
        matches!(ast.kind(id), NodeKind::Paragraph)
            && matches!(ast.children(id), [only] if matches!(ast.kind(*only), NodeKind::Strong))
    });
    assert_eq!((headings, bold), (1, 5), "{out}");
}

#[test]
fn shifted_output_parses_to_the_expected_levels() {
    let src = fs::read_to_string(fixture_dir().join("setext.md")).unwrap();
    let levels = |doc: &Document| -> Vec<(u8, bool)> {
        let ast = doc.ast();
        ast.descendants(ast.root())
            .filter_map(|id| match ast.kind(id) {
                NodeKind::Heading(heading) => Some((heading.level, heading.setext)),
                _ => None,
            })
            .collect()
    };
    let before = levels(&Document::parse(src.as_str()));
    let (out, _) = shift(&src, ShiftOptions::by(1));
    let after = levels(&Document::parse(out));
    assert_eq!(
        after,
        before
            .iter()
            .map(|&(level, _)| (level + 1, false))
            .collect::<Vec<_>>()
    );
}
