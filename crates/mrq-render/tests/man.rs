//! The man backend: roff snapshots of the fixtures in `tests/fixtures/man/`, the page
//! title conventions (H1, frontmatter, file name), escaping and the SYNOPSIS section.
//! Whether `man -l` shows the fixtures well is a manual check (`docs/testing.md`);
//! `mandoc_accepts_the_fixtures` runs mandoc's linter where it is installed.

use std::path::Path;

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_render::man::{ManOptions, ManRenderer, codes};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/man");

fn fixture(name: &str, flavor: &str) -> Document {
    DocumentOptions::new()
        .flavor(flavor)
        .load(Path::new(FIXTURES).join(name))
        .unwrap()
}

fn render(src: &str) -> String {
    ManRenderer::new(ManOptions::default()).render(&Document::parse(src))
}

/// The output without the apostrophe preamble of the `roff` crate.
fn body(src: &str) -> String {
    strip_preamble(&render(src))
}

fn strip_preamble(roff: &str) -> String {
    let preamble = ".ie \\n(.g .ds Aq \\(aq\n.el .ds Aq '\n";
    roff.replacen(preamble, "", 1)
}

// --- Snapshots --------------------------------------------------------------------------

#[test]
fn gfm_fixture() {
    let doc = fixture("mrq.1.md", "gfm");
    let output = ManRenderer::new(ManOptions::default()).render_with_diagnostics(&doc);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    insta::assert_snapshot!(output.roff);
}

#[test]
fn mkdocs_fixture() {
    let doc = fixture("mkdocs.7.md", "mkdocs");
    let output = ManRenderer::new(ManOptions::default()).render_with_diagnostics(&doc);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    insta::assert_snapshot!(output.roff);
}

#[test]
fn obsidian_syntax() {
    let src = "# notes(5)\n\nSee [[Other note#Part|the part]], [[Other]] and [[#Local]], \
               ![[pic.png]] #tag %%hidden%% ==marked==.\n\n> [!tip]- Fold *me*\n> Body.\n";
    let doc = DocumentOptions::new()
        .flavor("obsidian")
        .parse(src)
        .unwrap();
    let roff = ManRenderer::new(ManOptions::default()).render(&doc);
    insta::assert_snapshot!(strip_preamble(&roff));
}

// --- The title ------------------------------------------------------------------------

#[test]
fn the_h1_gives_name_section_and_description() {
    let out = body("# mrq-fmt(1) -- format Markdown files\n\nText.\n");
    assert_eq!(
        out,
        ".TH MRQ\\-FMT 1\n.SH NAME\nmrq\\-fmt \\- format Markdown files\n.PP\nText.\n"
    );
}

#[test]
fn h1_separators_and_plain_names() {
    assert!(
        body("# tool - does things\n").starts_with(".TH TOOL 1\n.SH NAME\ntool \\- does things\n")
    );
    assert!(body("# tool — does things\n").contains(".SH NAME\ntool \\- does things\n"));
    assert_eq!(body("# tool(8)\n"), ".TH TOOL 8\n.SH NAME\ntool\n");
}

#[test]
fn frontmatter_overrides_the_h1() {
    let src = "---\ntitle: other\nsection: 5\ndate: 2026-01-02\nsource: mrq 1.0\nmanual: User Commands\n---\n# tool(1) -- does things\n";
    assert_eq!(
        body(src),
        ".TH OTHER 5 2026-01-02 \"mrq 1.0\" \"User Commands\"\n.SH NAME\nother \\- does things\n"
    );
}

#[test]
fn options_fill_in_what_the_document_does_not_say() {
    let options = ManOptions {
        date: Some("October 2026".to_owned()),
        source: None,
        manual: Some("mrq Manual".to_owned()),
    };
    let roff = ManRenderer::new(options).render(&Document::parse("# tool(1)\n"));
    assert!(
        roff.contains(".TH TOOL 1 \"October 2026\" \"\" \"mrq Manual\"\n"),
        "{roff}"
    );
    let src = "---\ndate: 2020-01-01\n---\n# tool(1)\n";
    let options = ManOptions {
        date: Some("October 2026".to_owned()),
        ..ManOptions::default()
    };
    let roff = ManRenderer::new(options).render(&Document::parse(src));
    assert!(roff.contains(".TH TOOL 1 2020-01-01\n"), "{roff}");
}

#[test]
fn the_file_name_gives_name_and_section() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mrq-lint.8.md");
    std::fs::write(&path, "## Description\n\nLints.\n").unwrap();
    let doc = DocumentOptions::new().load(&path).unwrap();
    let output = ManRenderer::new(ManOptions::default()).render_with_diagnostics(&doc);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(
        strip_preamble(&output.roff),
        ".TH MRQ\\-LINT 8\n.SH NAME\nmrq\\-lint\n.SH DESCRIPTION\nLints.\n"
    );
}

#[test]
fn a_page_without_a_name_is_reported() {
    let doc = Document::parse("Some text.\n");
    let output = ManRenderer::new(ManOptions::default()).render_with_diagnostics(&doc);
    assert_eq!(
        strip_preamble(&output.roff),
        ".TH UNTITLED 1\n.PP\nSome text.\n"
    );
    assert_eq!(output.diagnostics.len(), 1);
    assert_eq!(output.diagnostics[0].code, codes::MAN_NAME);
}

#[test]
fn a_written_name_section_is_kept() {
    let out = body("# tool(1) -- generated\n\n## Name\n\ntool \\- written\n");
    assert_eq!(out, ".TH TOOL 1\n.SH NAME\ntool \\- written\n");
}

#[test]
fn a_later_h1_is_a_section() {
    let out = body("Intro.\n\n# Usage\n\n### Sub\n");
    assert!(out.contains(".PP\nIntro.\n.SH USAGE\n.SS Sub\n"), "{out}");
}

// --- Inlines and escaping ---------------------------------------------------------------

#[test]
fn escaping() {
    let out = body("# t(1)\n\na\\\\b c-d \"q\"\n.dot\n'quote\n");
    assert!(
        out.ends_with(".PP\na\\\\b c\\-d \"q\"\n\\&.dot\n\\*(Aqquote\n"),
        "{out}"
    );
    let out = body("# t(1)\n\n## A \"quoted\" -- \\\\ heading\n");
    assert!(
        out.ends_with(".SH \"A \\(dqQUOTED\\(dq \\-\\- \\\\ HEADING\"\n"),
        "{out}"
    );
}

#[test]
fn hard_breaks_start_a_new_line() {
    let out = body("# t(1)\n\none\\\n.two\n");
    assert!(out.ends_with(".PP\none\n.br\n\\&.two\n"), "{out}");
}

#[test]
fn synopsis_keeps_its_lines() {
    let out = body("# t(1)\n\n## SYNOPSIS\n\n`t` *a*\n`t` **b**\n\n## Other\n\nx\ny\n");
    assert!(
        out.contains(".SH SYNOPSIS\n\\fBt\\fR \\fIa\\fR\n.br\n\\fBt\\fR \\fBb\\fR\n"),
        "{out}"
    );
    assert!(out.ends_with(".SH OTHER\nx\ny\n"), "{out}");
}

#[test]
fn tables_ask_for_tbl() {
    let roff = render("# t(1)\n\n| a |\n| - |\n| b |\n");
    assert!(roff.starts_with("'\\\" t\n"), "{roff}");
    assert!(!render("# t(1)\n").starts_with("'\\\""));
}

// --- mandoc -------------------------------------------------------------------------------

/// mandoc's linter finds no warnings in the fixtures. Needs `mandoc` on the `PATH`.
#[test]
#[ignore = "needs mandoc"]
fn mandoc_accepts_the_fixtures() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    for (name, flavor) in [("mrq.1.md", "gfm"), ("mkdocs.7.md", "mkdocs")] {
        let options = ManOptions {
            date: Some("2026-10-31".to_owned()),
            ..ManOptions::default()
        };
        let roff = ManRenderer::new(options).render(&fixture(name, flavor));
        let mut child = Command::new("mandoc")
            .args(["-T", "lint", "-W", "warning"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("mandoc is installed");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(roff.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let report = String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && report.trim().is_empty(),
            "{name}:\n{report}"
        );
    }
}
