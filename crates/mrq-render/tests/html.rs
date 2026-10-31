//! The HTML backend beyond the spec suites: heading ids, flavor extensions (alerts,
//! callouts, admonitions, footnotes, math, task lists, tables, definition lists,
//! wikilinks, …), highlighted code, the stylesheet, standalone pages and the code-block
//! hook for diagrams.

use std::fs;

use mrq_core::Document;
use mrq_core::ast::CodeBlock;
use mrq_core::document::DocumentOptions;
use mrq_render::Theme;
use mrq_render::html::{HtmlError, HtmlOptions, HtmlRenderer};

fn fragment_options() -> HtmlOptions {
    HtmlOptions {
        fragment: true,
        ..HtmlOptions::default()
    }
}

/// The HTML fragment of `src` parsed with `flavor`.
fn render(flavor: &str, src: &str) -> String {
    let doc = DocumentOptions::new().flavor(flavor).parse(src).unwrap();
    HtmlRenderer::new(&Theme::default(), fragment_options())
        .unwrap()
        .render(&doc)
}

#[test]
fn headings_get_slugger_ids() {
    let src = "# Hello World\n\n## Hello World\n\n### Ünïcode & `code`\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn mkdocs_heading_attributes_and_ids() {
    let src = "# Intro {#start .lead data-x=1}\n\n## Intro\n\n## Intro\n";
    insta::assert_snapshot!(render("mkdocs", src));
}

#[test]
fn gfm_alerts() {
    let src = "> [!NOTE]\n> Useful *information*.\n\n> [!WARNING]\n> Careful.\n\n> [!CAUTION]\n> Danger.\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn obsidian_callouts_with_titles_and_folds() {
    let src =
        "> [!tip] Custom *title*\n> Body.\n\n> [!faq]- Closed\n> Hidden.\n\n> [!bug]+\n> Open.\n";
    insta::assert_snapshot!(render("obsidian", src));
}

#[test]
fn mkdocs_admonitions() {
    let src = "!!! note\n    Body.\n\n!!! danger \"Watch out\"\n    Body.\n\n!!! tip \"\"\n    No title.\n\n??? info inline \"Collapsed\"\n    Hidden.\n\n???+ example\n    !!! warning\n        Nested.\n";
    insta::assert_snapshot!(render("mkdocs", src));
}

#[test]
fn footnotes() {
    let src = "One[^a], two[^b] and one again[^a].\n\n[^b]: Second.\n[^a]: First with *emphasis*.\n\n    And a second paragraph.\n\n[^unused]: Never referenced.\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn math_as_mathml() {
    let src = "Inline $a^2 + b^2 = c^2$ math.\n\n$$\n\\frac{1}{2}\n$$\n\nBroken $\\frac{1$ math.\n";
    insta::assert_snapshot!(render("obsidian", src));
}

#[test]
fn mkdocs_math_delimiters() {
    let src = "Inline \\(x_1\\) and display:\n\n\\[\n\\sqrt{2}\n\\]\n";
    insta::assert_snapshot!(render("mkdocs", src));
}

#[test]
fn task_lists() {
    let src = "- [ ] open\n- [x] done\n- plain\n\n1. [X] loose\n\n2. [ ] items\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn tables() {
    let src = "| Left | Center | Right | None |\n| :--- | :----: | ----: | ---- |\n| `a` | **b** | c \\| d |\n| 1 | 2 | 3 | 4 |\n\n| Header only |\n| --- |\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn gfm_inline_extensions() {
    let src =
        "~~gone~~, www.example.com, <https://example.com/a b>, ![alt *text*](/img.png \"T\").\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn obsidian_inline_extensions() {
    let src = "See [[Other Note]], [[Other Note#Some Heading|alias]], [[#Local]] and [[file.pdf]].\n\n![[diagram.png|200x100]] ![[Embedded Note]]\n\n#tag/nested ==marked== %%hidden%%\n\n# Local\n";
    insta::assert_snapshot!(render("obsidian", src));
}

#[test]
fn definition_lists_and_abbreviations() {
    let src = "The HTML spec.\n\nTerm\n:   Definition of the term.\n\nOther term\n:   First.\n:   Second.\n\n*[HTML]: Hyper Text Markup Language\n";
    insta::assert_snapshot!(render("mkdocs", src));
}

#[test]
fn directives_frontmatter_and_comments_are_not_rendered() {
    let src =
        "---\ntitle: T\n---\n<!-- mrq:toc -->\n<!-- mrq:end -->\n\n<!-- a comment -->\n\ntext\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn highlighted_code_uses_classes() {
    let src = "```rust\nfn main() { println!(\"<hi>\"); }\n```\n\n```unknown-lang\n<plain> & text\n```\n\n    indented\n";
    insta::assert_snapshot!(render("gfm", src));
}

#[test]
fn highlighting_can_be_turned_off() {
    let doc = Document::parse("```rust\nfn main() {}\n```\n");
    let options = HtmlOptions {
        highlight: false,
        ..fragment_options()
    };
    let html = HtmlRenderer::new(&Theme::default(), options)
        .unwrap()
        .render(&doc);
    assert_eq!(
        html,
        "<pre><code class=\"language-rust\">fn main() {}\n</code></pre>\n"
    );
}

#[test]
fn tagfilter_escapes_disallowed_raw_html() {
    let src = "<script>alert(1)</script>\n\n<b>ok</b> <iframe src=x></iframe>\n";
    let doc = Document::parse(src);
    let render = |tagfilter| {
        let options = HtmlOptions {
            tagfilter,
            ..fragment_options()
        };
        HtmlRenderer::new(&Theme::default(), options)
            .unwrap()
            .render(&doc)
    };
    insta::assert_snapshot!(render(true));
    assert!(render(false).contains("<script>alert(1)</script>"));
}

#[test]
fn stylesheet_is_generated_from_the_theme() {
    let theme = Theme::builtin("dark").unwrap();
    let renderer = HtmlRenderer::new(&theme, HtmlOptions::default()).unwrap();
    insta::assert_snapshot!(renderer.stylesheet());
}

#[test]
fn standalone_page() {
    let src = "---\ntitle: \"A <title>\"\n---\n# Heading\n\nText with $x$.\n";
    let doc = Document::parse(src);
    let html = HtmlRenderer::new(&Theme::default(), HtmlOptions::default())
        .unwrap()
        .render(&doc);
    // The stylesheet has its own snapshot.
    let style_start = html.find("<style>").unwrap() + "<style>".len();
    let style_end = html.find("</style>").unwrap();
    let page = format!("{}…{}", &html[..style_start], &html[style_end..]);
    insta::assert_snapshot!(page);
}

#[test]
fn standalone_title_falls_back_to_the_first_h1_then_a_default() {
    let title = |src: &str| {
        let html = HtmlRenderer::new(&Theme::default(), HtmlOptions::default())
            .unwrap()
            .render(&Document::parse(src));
        let start = html.find("<title>").unwrap() + "<title>".len();
        let end = html.find("</title>").unwrap();
        html[start..end].to_owned()
    };
    assert_eq!(title("## Sub\n\n# Main *title*\n"), "Main title");
    assert_eq!(title("text\n"), "Document");
}

#[test]
fn css_override_is_appended() {
    let dir = tempfile::tempdir().unwrap();
    let css = dir.path().join("extra.css");
    fs::write(&css, "body { margin: 0; }\n").unwrap();
    let mut theme = Theme::default();
    theme.overrides.html = Some(css);
    let renderer = HtmlRenderer::new(&theme, HtmlOptions::default()).unwrap();
    assert!(renderer.stylesheet().ends_with("body { margin: 0; }\n"));
    let html = renderer.render(&Document::parse("x\n"));
    assert!(html.contains("body { margin: 0; }"));
}

#[test]
fn missing_css_override_is_a_clear_error() {
    let mut theme = Theme::default();
    theme.overrides.html = Some("/does/not/exist.css".into());
    let err = HtmlRenderer::new(&theme, HtmlOptions::default())
        .err()
        .unwrap();
    assert!(matches!(err, HtmlError::Override { .. }), "{err}");
    assert!(
        err.to_string()
            .starts_with("cannot read the HTML override /does/not/exist.css"),
        "{err}"
    );
}

#[test]
fn unknown_code_theme_is_an_error() {
    let mut theme = Theme::default();
    theme.code.theme = "Nope".to_owned();
    let err = HtmlRenderer::new(&theme, HtmlOptions::default())
        .err()
        .unwrap();
    assert!(matches!(err, HtmlError::Highlight(_)), "{err}");
}

#[test]
fn code_block_hook_replaces_blocks_it_handles() {
    let doc = Document::parse("```mermaid\ngraph TD\n```\n\n```rust\nlet x = 1;\n```\n");
    let options = HtmlOptions {
        highlight: false,
        ..fragment_options()
    };
    let html = HtmlRenderer::new(&Theme::default(), options)
        .unwrap()
        .with_code_hook(|block: &CodeBlock| {
            (block.lang() == Some("mermaid"))
                .then(|| format!("<svg data-src=\"{}\"></svg>\n", block.literal.trim()))
        })
        .render(&doc);
    insta::assert_snapshot!(html);
}
