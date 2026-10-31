//! The PDF backend (feature `pdf`): snapshots of the generated Typst markup, the LaTeX to
//! Typst math conversion, escaping round trips, and PDFs compiled offline with the
//! bundled fonts only, checked for validity and by their extracted text.

#![cfg(feature = "pdf")]

use std::path::Path;

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_render::Theme;
use mrq_render::diagram::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer, Registry};
use mrq_render::pdf::{
    PdfError, PdfOptions, PdfRenderer, bundled_font_families, codes, tex_to_typst,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pdf");

/// A diagram renderer that answers from memory.
struct Fixed(Result<DiagramOutput, DiagramError>);

impl DiagramRenderer for Fixed {
    fn id(&self) -> &'static str {
        "fixed"
    }

    fn languages(&self) -> &[String] {
        static LANGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        LANGS.get_or_init(|| vec!["fake".to_owned()])
    }

    fn formats(&self, _lang: &str) -> &[DiagramFormat] {
        &[DiagramFormat::Svg]
    }

    fn render(&self, _: &str, _: &str, _: DiagramFormat) -> Result<DiagramOutput, DiagramError> {
        self.0.clone()
    }
}

const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\"><rect width=\"40\" height=\"20\" fill=\"#c01c28\"/></svg>";

fn paper() -> Theme {
    Theme::builtin("paper").unwrap()
}

fn renderer(theme: &Theme) -> PdfRenderer {
    PdfRenderer::new(theme, PdfOptions::default())
        .unwrap()
        .with_diagrams(Registry::new().with_renderer(Fixed(Ok(DiagramOutput::Svg(SVG.to_owned())))))
}

fn fixture() -> Document {
    DocumentOptions::new()
        .flavor("gfm")
        .load(Path::new(FIXTURES).join("report.markdown"))
        .unwrap()
}

/// The Typst source without the theme preamble (everything up to the body marker).
fn body(src: &str) -> String {
    let doc = Document::parse(src);
    let source = renderer(&paper()).typst(&doc).source;
    let (_, body) = source.split_once("// --- body\n").expect("a body marker");
    body.to_owned()
}

/// The text of a PDF, whitespace runs collapsed to one space.
fn text_of(pdf: &[u8]) -> String {
    let text = pdf_extract::extract_text_from_mem(pdf).expect("extractable text");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_valid_pdf(pdf: &[u8]) {
    assert!(pdf.starts_with(b"%PDF-"), "no PDF header");
    let tail = &pdf[pdf.len().saturating_sub(32)..];
    assert!(tail.windows(5).any(|w| w == b"%%EOF"), "no %%EOF trailer");
}

/// `text` without whitespace: text extraction splits and joins words at style changes.
fn compact(text: &str) -> String {
    text.split_whitespace().collect()
}

/// Asserts that `parts` appear in `text` in this order, ignoring whitespace.
fn assert_in_order(text: &str, parts: &[&str]) {
    let compacted = compact(text);
    let mut rest = compacted.as_str();
    for part in parts {
        let part = compact(part);
        match rest.find(&part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => panic!("`{part}` missing (in order) from the text:\n{text}"),
        }
    }
}

// --- The fixture (acceptance) -----------------------------------------------------------

#[test]
fn fixture_markup() {
    let doc = fixture();
    let typst = renderer(&paper()).typst(&doc);
    assert!(typst.diagnostics.is_empty(), "{:?}", typst.diagnostics);
    insta::assert_snapshot!(typst.source);
}

#[test]
fn fixture_renders_to_a_valid_pdf_whose_text_matches() {
    let doc = fixture();
    let output = renderer(&paper()).render_with_diagnostics(&doc).unwrap();
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_valid_pdf(&output.pdf);
    let text = text_of(&output.pdf);
    assert_in_order(
        &text,
        &[
            "The PDF backend",
            "A paragraph with emphasis, strong text, strikethrough, inline code, a link and an autolink https://example.org.",
            "Typst's markup characters stay literal: # $ * _ ` < > @ [ ] / \\ = - + ~",
            "= at a line start.",
            "After a hard break, a footnote",
            "Lists",
            "First item",
            "Nested item",
            "An open task",
            "Three",
            "Four",
            "A blockquote with emphasis.",
            "Warning",
            "Critical content demanding attention.",
            "fn main() {",
            "println!(\"Hello, {}!\", \"world\");",
            "plain code",
            "indented line",
            "Tables",
            "Left",
            "Center",
            "Right",
            "alpha",
            "beta",
            "22",
            // The footnote is at the bottom of the first page.
            "The footnote text with a link.",
            "Math",
            "Inline",
            "and display:",
            "Images",
        ],
    );
}

#[test]
fn every_builtin_theme_compiles_without_warnings() {
    let doc = fixture();
    for name in mrq_render::theme::BUILTIN_THEMES {
        let theme = Theme::builtin(name).unwrap();
        let output = renderer(&theme).render_with_diagnostics(&doc).unwrap();
        // github names fonts that are not bundled (a `codes::FONT` warning each).
        let typst: Vec<_> = output
            .diagnostics
            .iter()
            .filter(|d| d.code != codes::FONT)
            .collect();
        assert!(typst.is_empty(), "{name}: {typst:?}");
        assert_valid_pdf(&output.pdf);
    }
}

// --- Offline: bundled fonts only ----------------------------------------------------------

#[test]
fn the_paper_fonts_are_bundled() {
    let families = bundled_font_families();
    let theme = paper();
    for family in [
        theme.fonts.body.as_str(),
        theme.fonts.heading.as_deref().unwrap(),
        theme.fonts.mono.as_str(),
    ] {
        assert!(
            families.iter().any(|f| f == family),
            "{family} in {families:?}"
        );
    }
    assert!(families.iter().any(|f| f == "New Computer Modern Math"));
}

#[test]
fn a_font_family_that_is_not_bundled_is_a_warning() {
    let mut theme = paper();
    theme.fonts.body = "No Such Font Family".to_owned();
    let doc = Document::parse("Text.\n");
    let output = renderer(&theme).render_with_diagnostics(&doc).unwrap();
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert_eq!(output.diagnostics[0].code, codes::FONT);
    assert!(
        output.diagnostics[0]
            .message
            .contains("No Such Font Family")
    );
    assert_in_order(&text_of(&output.pdf), &["Text."]);
}

#[test]
fn generic_font_names_fall_back_silently() {
    let mut theme = paper();
    theme.fonts.body = "Inter, system-ui".to_owned();
    theme.fonts.mono = "ui-monospace".to_owned();
    let doc = Document::parse("Text `code`.\n");
    let output = renderer(&theme).render_with_diagnostics(&doc).unwrap();
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

// --- Escaping ---------------------------------------------------------------------------

#[test]
fn markup_characters_are_escaped() {
    assert_eq!(
        body("A #b $c$ **d** _e_ `f` < g @h [i] /j\\ k = l - m + n ~ o \"p\" 'q' 1. r\n"),
        "A \\#b $c$ #strong[d] #emph[e] #raw(\"f\") \\< g \\@h \\[i\\] \\/j\\\\ k \\= l \\- m \\+ n \\~ o \\\"p\\\" \\'q\\' 1\\. r\n\n"
    );
}

#[test]
fn escaped_text_survives_the_round_trip() {
    let text = "#x $y *z* _w_ <a> @b [c] //d \\e = f -- g ++ h ~ i ... j: http://k.l/m \"n\" 'o'";
    let src = format!(
        "{}\n",
        text.replace('\\', "\\\\")
            .replace('*', "\\*")
            .replace('_', "\\_")
            .replace('[', "\\[")
            .replace('<', "\\<")
    );
    let doc = Document::parse(&src);
    let pdf = renderer(&paper()).render(&doc).unwrap();
    assert_eq!(compact(&text_of(&pdf)), compact(text));
}

#[test]
fn line_starts_cannot_become_typst_blocks() {
    let src = "a\n= b\n/ e: f\n2\\. g\n-c\n+d\n";
    let doc = Document::parse(src);
    let pdf = renderer(&paper()).render(&doc).unwrap();
    assert_eq!(compact(&text_of(&pdf)), compact("a = b / e: f 2. g -c +d"));
}

#[test]
fn strings_escape_quotes_backslashes_and_controls() {
    assert_eq!(body("`a\"b\\c`\n"), "#raw(\"a\\\"b\\\\c\")\n\n");
}

// --- Structure --------------------------------------------------------------------------

#[test]
fn headings_get_labels_from_the_slugger_and_internal_links_use_them() {
    let out = body(
        "# Intro\n\n## Hello World\n\nSee [it](#hello-world), [gone](#nowhere) and [web](https://x.org/a?b).\n",
    );
    insta::assert_snapshot!(out);
}

#[test]
fn lists_tasks_and_tables() {
    let out = body(
        "- a\n- b\n\n  more\n\n1) x\n2) y\n\n- [ ] open\n- [x] done\n\n| h1 | h2 |\n|:--|--:|\n| 1 | 2 |\n",
    );
    insta::assert_snapshot!(out);
}

#[test]
fn footnotes_are_typst_footnotes_numbered_by_first_reference() {
    let out =
        body("A[^b] B[^a] C[^b] D[^missing].\n\n[^a]: Note a.\n[^b]: Note b.\n[^c]: Unused.\n");
    insta::assert_snapshot!(out);
}

#[test]
fn alerts_callouts_and_admonitions() {
    let gfm = body("> [!TIP]\n> Body.\n");
    insta::assert_snapshot!("gfm_alert", gfm);
    let obsidian = DocumentOptions::new()
        .flavor("obsidian")
        .parse("> [!bug]- Title *md*\n> Body.\n\n[[Other#Part|alias]] #tag ==mark== %%hidden%%\n")
        .unwrap();
    let source = renderer(&paper()).typst(&obsidian).source;
    insta::assert_snapshot!("obsidian", source.split_once("// --- body\n").unwrap().1);
    let mkdocs = DocumentOptions::new()
        .flavor("mkdocs")
        .parse("!!! danger \"\"\n    No title.\n\nTerm\n:   Details.\n")
        .unwrap();
    let source = renderer(&paper()).typst(&mkdocs).source;
    insta::assert_snapshot!("mkdocs", source.split_once("// --- body\n").unwrap().1);
}

#[test]
fn highlighted_code_carries_the_theme_colors() {
    let out = body("```rust\nlet x = 1;\n```\n");
    assert!(out.starts_with("#mrq-code(fill: rgb(\"#"), "{out}");
    assert!(out.contains("(\"let\", rgb(\""), "{out}");
    assert!(out.contains("(\" x \", none"), "{out}");
}

#[test]
fn code_on_a_code_theme_background_like_the_page_uses_the_code_background() {
    // paper's InspiredGitHub code theme is white on a white page.
    let out = body("```rust\nlet x = 1;\n```\n");
    assert!(out.starts_with("#mrq-code(fill: rgb(\"#f7f7f7\")"), "{out}");
}

#[test]
fn plain_code_keeps_leading_whitespace_in_the_pdf() {
    let doc = Document::parse("```\nplain\n    indented\n```\n");
    let typst = renderer(&paper()).typst(&doc);
    assert!(
        typst
            .source
            .contains("((\"    indented\", none, false, false, false),)"),
        "{}",
        typst.source
    );
    let pdf = renderer(&paper()).render(&doc).unwrap();
    assert_in_order(&text_of(&pdf), &["plain", "indented"]);
}

// --- Images and diagrams ------------------------------------------------------------------

#[test]
fn local_images_are_mounted_and_missing_ones_warn() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        Path::new(FIXTURES).join("logo.svg"),
        dir.path().join("logo.svg"),
    )
    .unwrap();
    let path = dir.path().join("doc.md");
    std::fs::write(
        &path,
        "![Logo](logo.svg) ![Gone](gone.png) ![Web](https://x.org/a.png)\n",
    )
    .unwrap();
    let doc = DocumentOptions::new().load(&path).unwrap();
    let output = renderer(&paper()).render_with_diagnostics(&doc).unwrap();
    let codes: Vec<&str> = output.diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(
        codes,
        [codes::IMAGE, codes::IMAGE],
        "{:?}",
        output.diagnostics
    );
    assert_valid_pdf(&output.pdf);
    assert_in_order(&text_of(&output.pdf), &["[Gone]", "[Web]"]);
    let source = renderer(&paper()).typst(&doc).source;
    assert!(
        source.contains("#mrq-image(\"/assets/1.svg\", alt: \"Logo\")"),
        "{source}"
    );
}

#[test]
fn diagrams_render_as_svg_and_failures_fall_back_to_code() {
    let doc = Document::parse("```fake\ngraph\n```\n");
    let typst = renderer(&paper()).typst(&doc);
    assert!(
        typst.source.contains("#mrq-diagram(\"/assets/1.svg\")"),
        "{}",
        typst.source
    );

    let failing = PdfRenderer::new(&paper(), PdfOptions::default())
        .unwrap()
        .with_diagrams(
            Registry::new().with_renderer(Fixed(Err(DiagramError::Failed("boom".into())))),
        );
    let output = failing.render_with_diagnostics(&doc).unwrap();
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert_eq!(
        output.diagnostics[0].code,
        mrq_render::diagram::codes::DIAGRAM
    );
    assert_in_order(&text_of(&output.pdf), &["graph"]);
}

// --- Math ---------------------------------------------------------------------------------

#[test]
fn simple_latex_converts_to_typst_math() {
    let cases = [
        ("x^2 + y_1", "x^(2) + y_(1)"),
        ("\\frac{a}{b}", "frac(a, b)"),
        ("\\sqrt{x}", "sqrt(x)"),
        ("\\sqrt[3]{x}", "root(3, x)"),
        ("\\alpha \\le \\beta", "alpha <= beta"),
        ("\\sum_{i=1}^{n} x_i", "sum_(i = 1)^(n) x_(i)"),
        ("ab", "a b"),
        ("\\mathbb{R}", "bb(R)"),
        ("\\text{if } x", "\"if \" x"),
        ("\\left( x \\right)", "( x )"),
        ("f(x) = 2.5", "f ( x ) = 2.5"),
        ("\\{a\\}", "\\{ a \\}"),
    ];
    for (tex, typst) in cases {
        assert_eq!(tex_to_typst(tex).as_deref(), Some(typst), "{tex}");
    }
}

#[test]
fn unconvertible_latex_is_none() {
    for tex in [
        "\\begin{matrix} a & b \\end{matrix}",
        "\\unknowncommand x",
        "a \\\\ b",
        "\\frac{a}",
    ] {
        assert_eq!(tex_to_typst(tex), None, "{tex}");
    }
}

#[test]
fn unconvertible_math_falls_back_to_its_tex_with_a_warning() {
    let doc = Document::parse("$\\unknowncommand x$\n\n$$\n\\begin{matrix} a \\end{matrix}\n$$\n");
    let output = renderer(&paper()).render_with_diagnostics(&doc).unwrap();
    let codes: Vec<&str> = output.diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes, [codes::MATH, codes::MATH]);
    assert_in_order(
        &text_of(&output.pdf),
        &["\\unknowncommand x", "\\begin{matrix} a \\end{matrix}"],
    );
}

// --- Theme and template -------------------------------------------------------------------

#[test]
fn the_preamble_maps_the_theme() {
    let doc = Document::parse("Text.\n");
    let source = renderer(&paper()).typst(&doc).source;
    let (preamble, _) = source.split_once("// --- body\n").unwrap();
    insta::assert_snapshot!(preamble);
}

#[test]
fn the_override_template_wraps_the_document() {
    let mut theme = paper();
    theme.overrides.pdf = Some(Path::new(FIXTURES).join("template/template.typ"));
    let doc = fixture();
    let output = renderer(&theme).render_with_diagnostics(&doc).unwrap();
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_in_order(
        &text_of(&output.pdf),
        &[
            "CONFIDENTIAL DRAFT",
            "Title: PDF fixture",
            "The PDF backend",
        ],
    );
}

#[test]
fn a_missing_template_is_an_error() {
    let mut theme = paper();
    theme.overrides.pdf = Some(Path::new(FIXTURES).join("nope.typ"));
    assert!(matches!(
        PdfRenderer::new(&theme, PdfOptions::default()),
        Err(PdfError::Override { .. })
    ));
}

#[test]
fn a_broken_template_is_a_compile_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.typ");
    std::fs::write(&path, "#let template(body) = { undefined-thing }\n").unwrap();
    let mut theme = paper();
    theme.overrides.pdf = Some(path);
    let err = renderer(&theme)
        .render(&Document::parse("Text.\n"))
        .unwrap_err();
    match err {
        PdfError::Compile { messages } => assert!(
            messages.iter().any(|m| m.contains("unknown variable")),
            "{messages:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn templates_cannot_load_packages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.typ");
    std::fs::write(
        &path,
        "#import \"@preview/example:0.1.0\": *\n#let template(body) = body\n",
    )
    .unwrap();
    let mut theme = paper();
    theme.overrides.pdf = Some(path);
    let err = renderer(&theme)
        .render(&Document::parse("Text.\n"))
        .unwrap_err();
    assert!(matches!(err, PdfError::Compile { .. }), "{err:?}");
}

#[test]
fn output_is_reproducible() {
    let doc = fixture();
    let a = renderer(&paper()).render(&doc).unwrap();
    let b = renderer(&paper()).render(&doc).unwrap();
    assert_eq!(a, b);
}
