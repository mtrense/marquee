//! Acceptance tests for the Markdown printer `mrq_core::fmt`: every `CommonMark` and GFM
//! spec example renders to the same HTML before and after formatting, plus the printer's
//! own conventions (config markers, ATX headings, blank lines, escaping).

use std::fs;
use std::path::Path;

use mrq_core::Document;
use mrq_core::config::{CodeFence, Emphasis, FmtConfig, ListMarker, Strong};
use mrq_core::document::DocumentOptions;
use mrq_core::fmt::format;
use mrq_core::parse::ParseOptions;

const FENCE: &str = "````````````````````````````````";

/// One example of a spec file.
struct Example {
    /// 1-based example number within its file.
    number: usize,
    /// The Markdown input, with the spec's `→` replaced by tabs.
    markdown: String,
}

/// The examples of a spec file (same format and extraction as `tests/parse.rs`).
fn examples(file: &str) -> Vec<Example> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/spec")
        .join(file);
    let text = fs::read_to_string(&path).unwrap();
    let mut examples = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if !line.starts_with(FENCE) || !line[FENCE.len()..].trim_start().starts_with("example") {
            continue;
        }
        let mut markdown = String::new();
        for line in lines.by_ref() {
            if line == "." {
                break;
            }
            markdown.push_str(line);
            markdown.push('\n');
        }
        for line in lines.by_ref() {
            if line.starts_with(FENCE) {
                break;
            }
        }
        examples.push(Example {
            number: examples.len() + 1,
            markdown: markdown.replace('→', "\t"),
        });
    }
    examples
}

/// comrak's HTML for `src` with the extensions of `options`, raw HTML included, as an
/// oracle independent of mrq's AST.
fn html(src: &str, options: ParseOptions) -> String {
    let mut comrak = comrak::Options::default();
    let ext = &mut comrak.extension;
    ext.strikethrough = options.strikethrough;
    ext.table = options.table;
    ext.autolink = options.autolink;
    ext.tasklist = options.tasklist;
    ext.footnotes = options.footnotes;
    ext.alerts = options.alerts;
    ext.math_dollars = options.math;
    comrak.render.r#unsafe = true;
    comrak::markdown_to_html(src, &comrak)
}

fn parse(src: &str, flavor: &str) -> Document {
    DocumentOptions::new().flavor(flavor).parse(src).unwrap()
}

fn fmt(src: &str) -> String {
    format(&Document::parse(src), &FmtConfig::default())
}

fn fmt_with(src: &str, config: &FmtConfig) -> String {
    format(&Document::parse(src), config)
}

/// Formats every example of `file` with `flavor` (and `config`) and compares the HTML of the result
/// with the HTML of the input. `known` lists the examples that are expected not to round
/// trip; they must still fail, so the list never outlives a fix.
fn check_spec(file: &str, flavor: &str, known: &[usize]) {
    check_spec_with(file, flavor, known, &FmtConfig::default(), false);
}

fn check_spec_with(file: &str, flavor: &str, known: &[usize], config: &FmtConfig, crlf: bool) {
    let examples = examples(file);
    assert!(examples.len() > 600, "{file}: only {}", examples.len());
    let mut failures = Vec::new();
    let mut fixed = Vec::new();
    for example in &examples {
        let markdown = if crlf {
            example.markdown.replace('\n', "\r\n")
        } else {
            example.markdown.clone()
        };
        let doc = parse(&markdown, flavor);
        let options = doc.flavor().parse_options();
        let formatted = format(&doc, config);
        // The printer writes `\n` line endings, also into code; comrak keeps the CR of
        // a CRLF inside code spans, where it should be part of one line ending.
        let before = html(&markdown, options).replace('\r', "");
        let after = html(&formatted, options);
        let same = before == after;
        match (same, known.contains(&example.number)) {
            (false, false) => failures.push(format!(
                "{file} example {}:\n--- input\n{}--- formatted\n{}--- html before\n{before}--- html after\n{after}",
                example.number, example.markdown, formatted
            )),
            (true, true) => fixed.push(example.number),
            _ => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        fixed.is_empty(),
        "{file}: examples {fixed:?} round-trip now; drop them from the known list"
    );
}

/// Spec examples whose meaning `fmt` does not keep, each for a reason given in
/// `fmt`'s module docs ("Known limits").
const COMMONMARK_KNOWN: &[usize] = &[];
const GFM_KNOWN: &[usize] = &[614];

#[test]
fn commonmark_spec_examples_keep_their_html() {
    check_spec("commonmark-spec.txt", "commonmark", COMMONMARK_KNOWN);
}

#[test]
fn gfm_spec_examples_keep_their_html() {
    check_spec("gfm-spec.txt", "gfm", GFM_KNOWN);
}

/// The other choice of every configurable marker.
fn other_markers() -> FmtConfig {
    FmtConfig {
        emphasis: Emphasis::Underscore,
        strong: Strong::Underscore,
        list_marker: ListMarker::Star,
        code_fence: CodeFence::Tilde,
        ..FmtConfig::default()
    }
}

#[test]
fn spec_examples_keep_their_html_with_other_markers() {
    let config = other_markers();
    check_spec_with(
        "commonmark-spec.txt",
        "commonmark",
        COMMONMARK_KNOWN,
        &config,
        false,
    );
    check_spec_with("gfm-spec.txt", "gfm", GFM_KNOWN, &config, false);
}

#[test]
fn spec_examples_keep_their_html_with_crlf() {
    let config = FmtConfig::default();
    check_spec_with(
        "commonmark-spec.txt",
        "commonmark",
        COMMONMARK_KNOWN,
        &config,
        true,
    );
    check_spec_with("gfm-spec.txt", "gfm", GFM_KNOWN, &config, true);
}

#[test]
fn empty_documents_stay_empty() {
    assert_eq!(fmt(""), "");
    assert_eq!(fmt("\n\n  \n"), "");
}

#[test]
fn blocks_are_separated_by_one_blank_line_and_end_with_one_newline() {
    assert_eq!(
        fmt("# A\nPara one\nline two\n\n\n\n---\n> quote\n\n\n"),
        "# A\n\nPara one\nline two\n\n---\n\n> quote\n"
    );
}

#[test]
fn trailing_whitespace_is_dropped() {
    assert_eq!(
        fmt("a   \nb\t\n\n> c  \n>\n> d\n"),
        "a\\\nb\n\n> c\n>\n> d\n"
    );
}

#[test]
fn headings_become_atx_unless_they_span_lines() {
    assert_eq!(fmt("Title\n=====\n\nSub\n---\n"), "# Title\n\n## Sub\n");
    assert_eq!(fmt("### Closed ###\n"), "### Closed\n");
    assert_eq!(fmt("Two\nlines\n===\n"), "Two\nlines\n===\n");
    assert_eq!(fmt("# Ends with \\#\n"), "# Ends with \\#\n");
}

#[test]
fn default_markers_come_from_the_config() {
    assert_eq!(
        fmt("_em_ __strong__\n\n* a\n+ b\n"),
        "*em* **strong**\n\n- a\n\n* b\n"
    );
    let config = FmtConfig {
        emphasis: Emphasis::Underscore,
        strong: Strong::Underscore,
        list_marker: ListMarker::Plus,
        code_fence: CodeFence::Tilde,
        ..FmtConfig::default()
    };
    assert_eq!(
        fmt_with("*em* **strong**\n\n- a\n\n```rust\nx\n```\n", &config),
        "_em_ __strong__\n\n+ a\n\n~~~rust\nx\n~~~\n"
    );
}

#[test]
fn intraword_emphasis_keeps_stars() {
    let config = FmtConfig {
        emphasis: Emphasis::Underscore,
        strong: Strong::Underscore,
        ..FmtConfig::default()
    };
    assert_eq!(fmt_with("a*b*c **d** e\n", &config), "a*b*c __d__ e\n");
    assert_eq!(fmt_with("**d**e\n", &config), "**d**e\n");
}

#[test]
fn nested_emphasis_alternates_markers() {
    assert_eq!(fmt("***a***\n"), "***a***\n");
    assert_eq!(fmt("**_a_**\n"), "**_a_**\n");
}

#[test]
fn indented_code_becomes_fenced_and_fences_outgrow_the_content() {
    assert_eq!(fmt("    code\n"), "```\ncode\n```\n");
    assert_eq!(
        fmt("~~~md\n```\nx\n```\n~~~\n"),
        "````md\n```\nx\n```\n````\n"
    );
    assert_eq!(fmt("``` a\nx\n```\n"), "```a\nx\n```\n");
    assert_eq!(fmt("~~~ a`b\nx\n~~~\n"), "~~~a`b\nx\n~~~\n");
}

#[test]
fn lists_indent_content_by_their_marker() {
    assert_eq!(
        fmt("* a\n\n  b\n* c\n   * d\n"),
        "- a\n\n  b\n\n- c\n\n  - d\n"
    );
    assert_eq!(fmt("1) a\n1) b\n   1) c\n"), "1) a\n2) b\n   1) c\n");
    assert_eq!(fmt("7. a\n7. b\n"), "7. a\n8. b\n");
    assert_eq!(fmt("- [ ] open\n- [X] done\n"), "- [ ] open\n- [x] done\n");
}

#[test]
fn text_that_looks_like_syntax_is_escaped() {
    assert_eq!(fmt("\\# not a heading\n"), "\\# not a heading\n");
    assert_eq!(fmt("\\- not a list\n"), "\\- not a list\n");
    assert_eq!(fmt("1\\. not a list\n"), "1\\. not a list\n");
    // Only what could open emphasis, or close the emphasis it is in.
    assert_eq!(
        fmt("\\*a\\* \\_b\\_ snake_case 2 * 3 x_\n"),
        "\\*a* \\_b_ snake_case 2 * 3 x_\n"
    );
    assert_eq!(fmt("*a\\* b*\n"), "*a\\* b*\n");
    assert_eq!(fmt("&amp;copy; \\[x\\]\n"), "\\&copy; \\[x\\]\n");
    assert_eq!(fmt("a\\\nb\n"), "a\\\nb\n");
}

#[test]
fn links_keep_their_form() {
    assert_eq!(
        fmt("[a](<b c> 'd') <https://x.y> www.z.com\n"),
        "[a](<b c> \"d\") <https://x.y> www.z.com\n"
    );
    assert_eq!(
        fmt("[a][Ref] [ref][] [ref]\n\n[ref]: /url  'title'\n"),
        "[a][Ref] [ref][] [ref]\n\n[ref]: /url \"title\"\n"
    );
}

#[test]
fn code_spans_choose_their_backticks() {
    assert_eq!(fmt("`` a`b ``\n"), "``a`b``\n");
    assert_eq!(fmt("`` `a ``\n"), "`` `a ``\n");
}

#[test]
fn tables_and_footnotes_print() {
    assert_eq!(
        fmt("a|b\n:-|-:\nc|`\\|`\n"),
        "| a   |    b |\n| :-- | ---: |\n| c   | `\\|` |\n"
    );
    assert_eq!(
        fmt("x[^1]\n\n[^1]: note\n\n    more\n"),
        "x[^1]\n\n[^1]: note\n\n    more\n"
    );
}

/// The fixtures in `tests/fixtures/fmt/`: each `<name>.md` formats to `<name>.out.md`,
/// which formats to itself, and both render to the same HTML.
#[test]
fn fixtures_format_to_their_expected_output() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fmt");
    let mut inputs: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "md")
                && !path
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".out")
        })
        .collect();
    inputs.sort();
    assert!(inputs.len() > 1, "no fixtures in {}", dir.display());
    for input in inputs {
        let src = fs::read_to_string(&input).unwrap();
        let expected = fs::read_to_string(input.with_extension("out.md")).unwrap();
        let name = input.display();
        assert_eq!(fmt(&src), expected, "{name}");
        assert_eq!(fmt(&expected), expected, "{name}: not idempotent");
        let options = Document::parse(&src).flavor().parse_options();
        assert_eq!(html(&src, options), html(&expected, options), "{name}");
    }
}

#[test]
fn table_columns_are_padded_to_their_widest_cell() {
    // At least three wide, so that every alignment marker fits.
    assert_eq!(fmt("|a|\n|-|\n"), "| a   |\n| --- |\n");
    assert_eq!(
        fmt("a|b|c\n:-|:-:|-:\nlong cell|x|y\n"),
        "| a         |  b  |   c |\n| :-------- | :-: | --: |\n| long cell |  x  |   y |\n"
    );
    // Centered text leans left when the padding is odd.
    assert_eq!(
        fmt("a|b\n:-:|-\nxxxx|y\n"),
        "|  a   | b   |\n| :--: | --- |\n| xxxx | y   |\n"
    );
}

#[test]
fn table_rows_get_every_column_and_no_more() {
    assert_eq!(
        fmt("a|b|c\n-|-|-\nx\nx|y|z|w\n"),
        "| a   | b   | c   |\n| --- | --- | --- |\n| x   |     |     |\n| x   | y   | z   |\n"
    );
}

#[test]
fn tables_align_inside_containers() {
    assert_eq!(
        fmt("> a|b\n> -|-\n> ccc|d\n"),
        "> | a   | b   |\n> | --- | --- |\n> | ccc | d   |\n"
    );
    assert_eq!(
        fmt("- a|b\n  -|-\n  ccc|d\n"),
        "- | a   | b   |\n  | --- | --- |\n  | ccc | d   |\n"
    );
}

#[test]
fn table_cells_escape_pipes_and_keep_inline_content() {
    assert_eq!(
        fmt("a|b\n-|-\n`x\\|y` \\| z|*e* [l](/u)\n"),
        "| a           | b           |\n| ----------- | ----------- |\n| `x\\|y` \\| z | *e* [l](/u) |\n"
    );
}

#[test]
fn frontmatter_is_kept_verbatim() {
    assert_eq!(
        fmt("---\ntitle:   x\n---\n# A\n"),
        "---\ntitle:   x\n---\n\n# A\n"
    );
}

#[test]
fn spec_examples_format_idempotently() {
    let mut unstable = Vec::new();
    for (file, flavor) in [
        ("commonmark-spec.txt", "commonmark"),
        ("gfm-spec.txt", "gfm"),
    ] {
        for example in examples(file) {
            let once = format(&parse(&example.markdown, flavor), &FmtConfig::default());
            let twice = format(&parse(&once, flavor), &FmtConfig::default());
            if once != twice {
                unstable.push(format!(
                    "{file} example {}:\n{once}---\n{twice}",
                    example.number
                ));
            }
        }
    }
    assert!(unstable.is_empty(), "{}", unstable.join("\n"));
}

#[test]
fn reference_definitions_stay_where_they_were() {
    // In the item they were written in; a label whose definition is not recovered (a
    // multi-line one) is defined at the end.
    assert_eq!(
        fmt("- item\n\n  [b]:\n  /y\n- [b]\n\n[c\nd]\n\n[c\nd]: /z\n\n[unused]: </u v>  \"t\"\n"),
        "- item\n\n  [b]: /y\n\n- [b]\n\n[c\nd]\n\n[unused]: </u v> \"t\"\n\n[c d]: /z\n"
    );
}

#[test]
fn text_near_autolinks_and_html_keeps_its_meaning() {
    assert_eq!(
        fmt("foo<b>bar</b> 1 < 2 > 0 www.a.com/x&hl; <x\n"),
        "foo<b>bar</b> 1 < 2 > 0 www.a.com/x&hl; <x\n"
    );
    assert_eq!(fmt("\\<b> a\\&amp;\n"), "\\<b> a\\&amp;\n");
}

#[test]
fn flavor_syntax_prints_with_generic_rules() {
    let obsidian = "---\ntags: [a]\n---\n\n# T\n\n> [!tip]- Title\n> body\n\n[[Note#H|alias]] ![[img.png]] #tag %%c%% ==hi==\n";
    assert_eq!(
        format(&parse(obsidian, "obsidian"), &FmtConfig::default()),
        obsidian
    );
    let mkdocs = "!!! note \"T\"\n    body\n\n*[HTML]: Hyper\n\n## H {#id .c}\n";
    assert_eq!(
        format(&parse(mkdocs, "mkdocs"), &FmtConfig::default()),
        mkdocs
    );
    let gfm = "> [!NOTE]\n> Be careful\n\n$x^2$ and $$y$$\n";
    assert_eq!(fmt(gfm), gfm);
}

#[test]
fn directive_regions_are_printed_verbatim() {
    let src = "<!-- mrq:toc depth=2 -->\n\n- [A](#a)\n\n<!-- mrq:end -->\n\n# A\n";
    assert_eq!(fmt(src), src);
}
