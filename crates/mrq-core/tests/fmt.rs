//! Acceptance tests for the Markdown printer `mrq_core::fmt`: every `CommonMark` and GFM
//! spec example renders to the same HTML before and after formatting (in every wrap
//! mode), plus the printer's own conventions (config markers, ATX headings, blank lines,
//! escaping, wrapping).

use std::fs;
use std::path::Path;

use mrq_core::Document;
use mrq_core::ast::NodeKind;
use mrq_core::config::{
    CodeFence, Emphasis, FmtConfig, FrontmatterFormat, ListMarker, OrderedList, Strong, Wrap,
};
use mrq_core::diag::Severity;
use mrq_core::document::DocumentOptions;
use mrq_core::fmt::{Formatted, codes, format, format_with};
use mrq_core::parse::ParseOptions;
use proptest::prelude::*;

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
        let same = same_html(&before, &after, config.wrap);
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

/// Whether two renderings of a document are the same. Re-wrapping moves line breaks:
/// a soft break becomes a space or a space a soft break, which the HTML shows as `\n`
/// or ` ` (and a run of spaces may become one line break). Browsers render any run of
/// whitespace outside `<pre>` as one space, so unless `wrap` is `preserve`, such runs
/// are compared as one space; inside code blocks (`<pre><code>`, never wrapped)
/// whitespace must match exactly. A paragraph inside raw HTML `<pre>` (`CommonMark`
/// example 148) is wrapped like any other, so raw `<pre>` is not exempt.
fn same_html(before: &str, after: &str, wrap: Wrap) -> bool {
    if wrap == Wrap::Preserve {
        before == after
    } else {
        collapse_html_whitespace(before) == collapse_html_whitespace(after)
    }
}

/// `html` with every run of spaces, tabs and line breaks outside code blocks
/// (`<pre…><code…>…</pre>`) as one space.
fn collapse_html_whitespace(html: &str) -> String {
    let code_block = |at: usize| {
        let tag_end = html[at..].find('>').map_or(html.len(), |end| at + end + 1);
        html[tag_end..].starts_with("<code")
    };
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while !rest.is_empty() {
        let offset = html.len() - rest.len();
        let code = rest
            .match_indices("<pre")
            .map(|(at, _)| at)
            .find(|&at| code_block(offset + at));
        let (flow, pre) = match code {
            Some(start) => {
                let end = rest[start..]
                    .find("</pre>")
                    .map_or(rest.len(), |end| start + end + "</pre>".len());
                (&rest[..start], &rest[start..end])
            }
            None => (rest, ""),
        };
        let mut space = false;
        for c in flow.chars() {
            if matches!(c, ' ' | '\t' | '\n') {
                space = true;
            } else {
                if space {
                    out.push(' ');
                    space = false;
                }
                out.push(c);
            }
        }
        if space {
            out.push(' ');
        }
        out.push_str(pre);
        rest = &rest[flow.len() + pre.len()..];
    }
    out
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

/// The other choice of every configurable marker (and numbering).
fn other_markers() -> FmtConfig {
    FmtConfig {
        emphasis: Emphasis::Underscore,
        strong: Strong::Underscore,
        list_marker: ListMarker::Star,
        ordered_list: OrderedList::One,
        code_fence: CodeFence::Tilde,
        ..FmtConfig::default()
    }
}

fn preserved_numbers() -> FmtConfig {
    FmtConfig {
        ordered_list: OrderedList::Preserve,
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
fn spec_examples_keep_their_html_with_preserved_numbers() {
    let config = preserved_numbers();
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

/// Every wrap mode other than `preserve`; `width` at one column breaks at every place
/// it may.
fn wrap_modes() -> Vec<FmtConfig> {
    let wrap = |wrap, width| FmtConfig {
        wrap,
        width,
        ..FmtConfig::default()
    };
    vec![
        wrap(Wrap::Never, 80),
        wrap(Wrap::Width, 1),
        wrap(Wrap::Width, 30),
        wrap(Wrap::Sentence, 80),
    ]
}

#[test]
fn spec_examples_keep_their_html_in_every_wrap_mode() {
    for config in wrap_modes() {
        check_spec_with(
            "commonmark-spec.txt",
            "commonmark",
            COMMONMARK_KNOWN,
            &config,
            false,
        );
        check_spec_with("gfm-spec.txt", "gfm", GFM_KNOWN, &config, false);
    }
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
fn ordered_lists_follow_the_numbering_mode() {
    let src = "3. a\n9. b\n04. c\n";
    let numbered = |ordered_list| {
        let config = FmtConfig {
            ordered_list,
            ..FmtConfig::default()
        };
        fmt_with(src, &config)
    };
    assert_eq!(numbered(OrderedList::Ordered), "3. a\n4. b\n5. c\n");
    assert_eq!(numbered(OrderedList::One), "3. a\n3. b\n3. c\n");
    assert_eq!(numbered(OrderedList::Preserve), "3. a\n9. b\n04. c\n");
    // Past nine digits the start number repeats.
    assert_eq!(
        fmt("999999998. a\n1. b\n1. c\n"),
        "999999998. a\n999999999. b\n999999998. c\n"
    );
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

/// The Markdown files of `dir` (not the `.out.md` ones), sorted.
fn markdown_files(dir: &Path, outputs: bool) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "md")
                && (outputs
                    || !path
                        .file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .ends_with(".out"))
        })
        .collect();
    files.sort();
    assert!(files.len() > 1, "no fixtures in {}", dir.display());
    files
}

/// The JSON projection of `doc` as a structure to compare documents by: without ids,
/// positions and the choices the printer makes (heading style, list markers and
/// delimiters, code fences, container prefixes inside verbatim literals), with adjacent
/// text nodes joined. Unless `wrap` is
/// `preserve`, soft breaks count as spaces and whitespace runs in text as one space.
fn structure(doc: &Document, wrap: Wrap) -> serde_json::Value {
    fn normalize(node: &mut serde_json::Value, wrap: Wrap) {
        let serde_json::Value::Object(map) = node else {
            return;
        };
        for key in ["id", "position", "setext", "marker", "delimiter", "fence"] {
            map.remove(key);
        }
        // An HTML block that the admonitions pass re-parsed lacks its line ending.
        if map["type"] == "html"
            && let Some(serde_json::Value::String(value)) = map.get_mut("value")
        {
            value.truncate(value.trim_end_matches('\n').len());
        }
        // Obsidian comments and mkdocs math hold their source verbatim, container
        // prefixes included, which differ where a line continued a paragraph lazily.
        let verbatim = map["type"] == "math" || map.get("syntax").is_some_and(|s| s == "percent");
        if verbatim && let Some(serde_json::Value::String(value)) = map.get_mut("value") {
            let lines: Vec<&str> = value.split('\n').collect();
            *value = std::iter::once(lines[0])
                .chain(
                    lines[1..]
                        .iter()
                        .map(|line| line.trim_start_matches([' ', '\t', '>'])),
                )
                .collect::<Vec<_>>()
                .join("\n");
        }
        if let Some(serde_json::Value::Object(attrs)) = map.get_mut("attrs") {
            attrs.remove("position");
        }
        let Some(serde_json::Value::Array(children)) = map.get_mut("children") else {
            return;
        };
        let mut joined: Vec<serde_json::Value> = Vec::new();
        for mut child in children.drain(..) {
            if wrap != Wrap::Preserve && child["type"] == "softBreak" {
                child = serde_json::json!({ "type": "text", "value": " " });
            }
            normalize(&mut child, wrap);
            match joined.last_mut() {
                Some(last) if last["type"] == "text" && child["type"] == "text" => {
                    let value = format!(
                        "{}{}",
                        last["value"].as_str().unwrap(),
                        child["value"].as_str().unwrap()
                    );
                    last["value"] = value.into();
                }
                _ => joined.push(child),
            }
        }
        if wrap != Wrap::Preserve {
            for child in &mut joined {
                if child["type"] == "text" {
                    let value = child["value"].as_str().unwrap();
                    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
                    let pad = |yes: bool| if yes { " " } else { "" };
                    let value = format!(
                        "{}{collapsed}{}",
                        pad(value.starts_with(char::is_whitespace)),
                        pad(value.ends_with(char::is_whitespace) && !collapsed.is_empty())
                    );
                    child["value"] = value.into();
                }
            }
        }
        *children = joined;
    }
    let mut json = serde_json::to_value(doc.to_json_ast()).unwrap();
    normalize(&mut json, wrap);
    json
}

/// Asserts that `after` has the [`structure`] of `before`, showing where it differs.
fn assert_same_structure(before: &Document, after: &Document, wrap: Wrap, name: &str) {
    let pretty = |doc| serde_json::to_string_pretty(&structure(doc, wrap)).unwrap();
    let (before, after) = (pretty(before), pretty(after));
    if before != after {
        let (a, b): (Vec<_>, Vec<_>) = (before.lines().collect(), after.lines().collect());
        let at = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()));
        let from = at.saturating_sub(8);
        panic!(
            "{name}: the structure differs at line {at}\n--- before\n{}\n--- after\n{}",
            a[from..(at + 4).min(a.len())].join("\n"),
            b[from..(at + 4).min(b.len())].join("\n")
        );
    }
}

/// The fixtures in `tests/fixtures/fmt/`: each `<name>.md` formats to `<name>.out.md`,
/// which formats to itself, and both parse to the same [`structure`]. Fixtures at the
/// top level are GFM and also render to the same HTML; those in `obsidian/` and
/// `mkdocs/` are parsed with that flavor (comrak's HTML knows nothing of it). An
/// optional `<name>.fmt.yml` holds the `fmt` settings to format with.
#[test]
fn fixtures_format_to_their_expected_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fmt");
    for flavor in ["gfm", "obsidian", "mkdocs"] {
        let dir = if flavor == "gfm" {
            root.clone()
        } else {
            root.join(flavor)
        };
        for input in markdown_files(&dir, false) {
            let src = fs::read_to_string(&input).unwrap();
            let expected = fs::read_to_string(input.with_extension("out.md")).unwrap();
            let config: FmtConfig = match fs::read_to_string(input.with_extension("fmt.yml")) {
                Ok(yaml) => serde_saphyr::from_str(&yaml).unwrap(),
                Err(_) => FmtConfig::default(),
            };
            let name = input.display();
            let doc = parse(&src, flavor);
            assert_eq!(format(&doc, &config), expected, "{name}");
            let reparsed = parse(&expected, flavor);
            assert_eq!(
                format(&reparsed, &config),
                expected,
                "{name}: not idempotent"
            );
            assert_same_structure(&doc, &reparsed, config.wrap, &name.to_string());
            if flavor == "gfm" {
                let options = doc.flavor().parse_options();
                let (before, after) = (html(&src, options), html(&expected, options));
                assert!(
                    same_html(&before, &after, config.wrap),
                    "{name}: HTML differs\n{before}---\n{after}"
                );
            }
        }
    }
}

/// The flavor parser fixtures (`tests/fixtures/obsidian`, `tests/fixtures/mkdocs`) keep
/// their structure under fmt, in every wrap mode, and format idempotently.
#[test]
fn flavor_parser_fixtures_keep_their_structure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for flavor in ["obsidian", "mkdocs"] {
        for input in markdown_files(&root.join(flavor), true) {
            let src = fs::read_to_string(&input).unwrap();
            let doc = parse(&src, flavor);
            let configs = std::iter::once(FmtConfig::default()).chain(wrap_modes());
            for config in configs {
                let name = format!("{} ({:?} {})", input.display(), config.wrap, config.width);
                let once = format(&doc, &config);
                let reparsed = parse(&once, flavor);
                assert_same_structure(&doc, &reparsed, config.wrap, &format!("{name}\n{once}"));
                assert_eq!(format(&reparsed, &config), once, "{name}: not idempotent");
            }
        }
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
    assert_eq!(
        fmt("+++\ntitle =   'x' # why\n+++\n# A\n"),
        "+++\ntitle =   'x' # why\n+++\n\n# A\n"
    );
}

fn normalize() -> FmtConfig {
    FmtConfig {
        frontmatter: FrontmatterFormat::Normalize,
        ..FmtConfig::default()
    }
}

/// Formats `src` with `fmt.frontmatter: normalize` and no code formatters.
fn fmt_normalized(src: &str) -> Formatted {
    format_with(&Document::parse(src), &normalize(), &no_code_formatter)
}

fn no_code_formatter(_: &str, _: &str) -> Option<Result<String, String>> {
    None
}

#[test]
fn normalized_yaml_frontmatter_is_canonical() {
    let out = fmt_normalized(
        "---\ntitle:   'Hello'\ntags: [a,   b]\nn: 0x10\nyes: true\nnested: {x: 1,   y: \"two\"}\n---\n# A\n",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert_eq!(
        out.text,
        "---\ntitle: Hello\ntags:\n- a\n- b\n\"n\": 16\n\"yes\": true\nnested:\n  x: 1\n  \"y\": two\n---\n\n# A\n"
    );
    // An empty mapping has no lines.
    assert_eq!(fmt_normalized("---\n{}\n---\nA\n").text, "---\n---\n\nA\n");
}

#[test]
fn normalized_toml_frontmatter_is_canonical() {
    let out = fmt_normalized(
        "+++\ntitle =   'x'\nn = 0x10\nd =  1979-05-27\nlist = [ 1,2 ]\npoint = {x=1}\n[t]\nk  = \"v\"\n[[a]]\nb = 1\n+++\n# A\n",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert_eq!(
        out.text,
        "+++\ntitle = \"x\"\nn = 16\nd = 1979-05-27\nlist = [1, 2]\npoint = { x = 1 }\n\n[t]\nk = \"v\"\n\n[[a]]\nb = 1\n+++\n\n# A\n"
    );
}

#[test]
fn normalized_frontmatter_keeps_its_data_and_is_stable() {
    for src in [
        "---\ntitle: Hello\nlong: |\n  line one\n  line two\nempty: ''\nq: \"multi\\nline\"\nanchor: &x [1, 2]\nalias: *x\nlist:\n  - a: 1\n    b: [true, null, 1.5]\n  - c\n---\n",
        "---\ndate: 2024-01-01\nwords: ['yes', \"no\", ~, '', ' x ', -.5, 0o17]\nlong: \"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\"\n---\n",
        "+++\ndt = 1979-05-27T07:32:00Z\nt = 07:32:00\nf = 1e3\ns = '''\nraw \\ text'''\n\"key with space\" = 1\na.b.c = 2\n[x.y]\nz = [[1], {w = 'v'}]\n+++\n",
    ] {
        let doc = Document::parse(src);
        let once = fmt_normalized(src);
        assert!(once.diagnostics.is_empty(), "{src}: {:?}", once.diagnostics);
        let reparsed = Document::parse(once.text.as_str());
        assert_eq!(
            reparsed.frontmatter().unwrap().value(),
            doc.frontmatter().unwrap().value(),
            "{src}\n{}",
            once.text
        );
        assert_eq!(fmt_normalized(&once.text).text, once.text, "{src}");
    }
}

#[test]
fn frontmatter_that_cannot_be_normalized_is_kept_with_a_warning() {
    for (src, reason) in [
        ("---\n# a comment\ntitle: x\n---\n", "comment"),
        ("---\ntitle: x # why\n---\n", "comment"),
        ("+++\n# a comment\ntitle = 'x'\n+++\n", "comment"),
        ("+++\nlist = [\n  1, # one\n]\n+++\n", "comment"),
        ("---\ndate: !!str 2024\n---\n", "tag"),
        ("---\n1: one\n---\n", "key"),
        ("---\nflag: yes\n---\n", "YAML 1.1"),
        ("---\nlist: [a, off]\n---\n", "YAML 1.1"),
        ("---\n? [a]\n: b\n---\n", "key"),
        ("---\ntitle: [unclosed\n---\n", "parse"),
        ("+++\ntitle = \n+++\n", "parse"),
    ] {
        let doc = Document::parse(src);
        let out = format_with(&doc, &normalize(), &no_code_formatter);
        assert_eq!(out.text, src, "{src}");
        let [warning] = out.diagnostics.as_slice() else {
            panic!("{src}: {:?}", out.diagnostics)
        };
        assert_eq!(warning.code, codes::FRONTMATTER_NORMALIZE);
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.range, doc.frontmatter().unwrap().span(), "{src}");
        assert!(
            warning.message.contains(reason),
            "{src}: {}",
            warning.message
        );
    }
}

#[test]
fn spec_examples_format_idempotently() {
    let mut unstable = Vec::new();
    for (file, flavor) in [
        ("commonmark-spec.txt", "commonmark"),
        ("gfm-spec.txt", "gfm"),
    ] {
        let configs = [FmtConfig::default(), other_markers(), preserved_numbers()];
        for config in configs.into_iter().chain(wrap_modes()) {
            for example in examples(file) {
                let once = format(&parse(&example.markdown, flavor), &config);
                let twice = format(&parse(&once, flavor), &config);
                if once != twice {
                    unstable.push(format!(
                        "{file} example {} ({config:?}):\n{once}---\n{twice}",
                        example.number
                    ));
                }
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

fn fmt_flavor(src: &str, flavor: &str) -> String {
    format(&parse(src, flavor), &FmtConfig::default())
}

#[test]
fn flavor_syntax_prints_in_its_canonical_form() {
    let obsidian = "---\ntags: [a]\n---\n\n# T\n\n> [!tip]- Title\n> body\n\n[[Note#H|alias]] ![[img.png]] #tag %%c%% ==hi==\n";
    assert_eq!(fmt_flavor(obsidian, "obsidian"), obsidian);
    assert_eq!(
        fmt_flavor(">[!tip]-  A\nb\n\n[[ a # b | c ]]\n", "obsidian"),
        "> [!tip]- A\n> b\n\n[[a#b|c]]\n"
    );
    let mkdocs = "!!! note \"T\"\n    body\n\n*[HTML]: Hyper\n*[CSS]: Style\n\n## H {#id .c}\n";
    assert_eq!(fmt_flavor(mkdocs, "mkdocs"), mkdocs);
    // Bodies are re-indented by four spaces, attributes and definitions normalized.
    assert_eq!(
        fmt_flavor(
            "!!!note   \"T\"\n\tbody\n\n## H {: .c  #id k='v w' b }\n",
            "mkdocs"
        ),
        "!!! note \"T\"\n    body\n\n## H {#id .c k=\"v w\" b}\n"
    );
    assert_eq!(
        fmt_flavor("T\n: a\n\nU\n\n:  b\n", "mkdocs"),
        "T\n:   a\n\nU\n\n:   b\n"
    );
    let gfm = "> [!NOTE]\n> Be careful\n\n$x^2$ and $$y$$\n";
    assert_eq!(fmt(gfm), gfm);
}

#[test]
fn text_that_looks_like_flavor_syntax_is_escaped() {
    // Tags where a tag may start, and every `%%`.
    assert_eq!(
        fmt_flavor(
            "\\#tag *\\#em* a\\#b \\#1984 \\# x \\%%a\\%% 100%%%\n",
            "obsidian"
        ),
        "\\#tag *\\#em* a#b #1984 # x \\%%a\\%% 100\\%\\%%\n"
    );
    // Heading attributes, admonition headers, and `\\[`, which opens math.
    assert_eq!(
        fmt_flavor("## A \\{#x}\n\nb\n\\!!! c\n\n[d] \\(e\n", "mkdocs"),
        "## A \\{#x}\n\nb\n\\!!! c\n\n&#91;d\\] (e\n"
    );
    // Other flavors read none of it.
    assert_eq!(
        fmt("\\#tag \\%%a%% ## A {#x}\nb\n!!! c\n"),
        "#tag %%a%% ## A {#x}\nb\n!!! c\n"
    );
}

#[test]
fn verbatim_flavor_syntax_keeps_its_source_in_containers() {
    // `%%` comments and backslash math are literal source, prefixes included.
    for (src, flavor) in [
        ("> a %%b\n> c%% d\n", "obsidian"),
        ("> %%\n> x\n>\n> y\n> %%\n", "obsidian"),
        ("- a %%b\n  c%%\n", "obsidian"),
        ("> \\[\n> a\n> \\] \\( b \\)\n", "mkdocs"),
    ] {
        assert_eq!(fmt_flavor(src, flavor), src);
    }
}

/// A flavor that prints GFM alerts as they were written.
#[derive(Debug)]
struct KeepAlerts;

impl mrq_core::flavor::PrinterHooks for KeepAlerts {
    fn verbatim(&self, kind: &mrq_core::ast::NodeKind) -> bool {
        matches!(kind, NodeKind::Callout(_))
    }
}

impl mrq_core::flavor::Flavor for KeepAlerts {
    fn name(&self) -> &'static str {
        "keep-alerts"
    }

    fn parse_options(&self) -> ParseOptions {
        ParseOptions::gfm()
    }

    fn slugger(&self) -> Box<dyn mrq_core::flavor::Slugger> {
        Box::new(mrq_core::flavor::GithubSlugger::new())
    }

    fn printer_hooks(&self) -> Option<&dyn mrq_core::flavor::PrinterHooks> {
        Some(self)
    }
}

#[test]
fn printer_hooks_can_print_nodes_from_their_source() {
    let mut registry = mrq_core::flavor::Registry::builtin();
    registry.register(std::sync::Arc::new(KeepAlerts));
    let options = DocumentOptions::new()
        .registry(registry)
        .flavor("keep-alerts");
    // The alert keeps its spacing; the list item's indentation is the printer's.
    let doc = options
        .parse("* > [!NOTE]\n  >Be  *careful*\n\n  _x_\n")
        .unwrap();
    assert_eq!(
        format(&doc, &FmtConfig::default()),
        "- > [!NOTE]\n  >Be  *careful*\n\n  *x*\n"
    );
}

#[test]
fn directive_regions_are_printed_verbatim() {
    let src = "<!-- mrq:toc depth=2 -->\n\n- [A](#a)\n\n<!-- mrq:end -->\n\n# A\n";
    assert_eq!(fmt(src), src);
}

/// Inputs the wrapping proptest found the printer core getting wrong, in `preserve`
/// mode too: each keeps its HTML and formats idempotently.
#[test]
fn printer_edge_cases_keep_their_html() {
    for (src, expected) in [
        // A `\\` after a bare URL would be part of it.
        ("a www.x.com  \nb\n", "a www.x.com  \nb\n"),
        // A code block of one empty line.
        ("```\n\n```\n", "```\n\n```\n"),
        // An unclosed HTML block ends a list item with blank lines.
        ("- <!--\n\nx\n", "- <!--\n\nx\n"),
        // Nested empty items in one line would be a thematic break.
        ("- - +\n", "* - -\n"),
        // Before a hard break `\\`, `_` is followed by punctuation.
        ("_  \nb\n", "\\_\\\nb\n"),
        // A paragraph after a quote in a tight item.
        ("- > a\n  >\n  b\n", "- > a\n  >\n  b\n"),
        // An HTML block start and a one-column delimiter row at the start of a line.
        ("a\n\\<div\n", "a\n\\<div\n"),
        ("a\n\\:--\n", "a\n\\:--\n"),
    ] {
        let once = fmt(src);
        assert_eq!(once, expected, "{src:?}");
        let options = ParseOptions::gfm();
        assert_eq!(html(src, options), html(&once, options), "{src:?}");
        assert_eq!(fmt(&once), once, "{src:?}: not idempotent");
    }
}

fn wrapped(src: &str, wrap: Wrap, width: u16) -> String {
    let config = FmtConfig {
        wrap,
        width,
        ..FmtConfig::default()
    };
    fmt_with(src, &config)
}

#[test]
fn preserve_keeps_the_line_breaks_as_written() {
    let src = "A line that is longer than the configured width.\nshort\n";
    assert_eq!(wrapped(src, Wrap::Preserve, 10), src);
}

#[test]
fn never_puts_each_paragraph_on_one_line() {
    assert_eq!(
        wrapped("one\ntwo  three\n\n> four\n> five\n", Wrap::Never, 3),
        "one two three\n\n> four five\n"
    );
}

#[test]
fn width_fills_lines_up_to_the_width() {
    assert_eq!(
        wrapped("aaa bbb ccc\nddd eee fff ggg\n", Wrap::Width, 11),
        "aaa bbb ccc\nddd eee fff\nggg\n"
    );
    // Optimal fit: no line is left much shorter than the others (first fit would
    // give `aaa bb`, `cc`).
    assert_eq!(
        wrapped("aaa bb cc ddddd\n", Wrap::Width, 6),
        "aaa\nbb cc\nddddd\n"
    );
    // Display width: each CJK character takes two columns.
    assert_eq!(
        wrapped("日本 語の 文章\n", Wrap::Width, 9),
        "日本 語の\n文章\n"
    );
    // A word longer than the width gets a line of its own.
    assert_eq!(
        wrapped("a bbbbbbbbbbbb c\n", Wrap::Width, 5),
        "a\nbbbbbbbbbbbb\nc\n"
    );
}

#[test]
fn sentence_puts_each_sentence_on_a_line() {
    assert_eq!(
        wrapped(
            "One. Two!\nThree? \"Four.\" Five *six.* Seven\n",
            Wrap::Sentence,
            80
        ),
        "One.\nTwo!\nThree?\n\"Four.\"\nFive *six.*\nSeven\n"
    );
    // Abbreviations, initials and a lowercase continuation end no sentence.
    assert_eq!(
        wrapped(
            "See e.g. Dr. Who, J. R. R. Tolkien and vs. Ends here. and on. Next\n",
            Wrap::Sentence,
            80
        ),
        "See e.g. Dr. Who, J. R. R. Tolkien and vs. Ends here. and on.\nNext\n"
    );
}

#[test]
fn hard_breaks_stay_in_every_mode() {
    let src = "aaa bbb\\\nccc  \nddd eee\n";
    for wrap in [Wrap::Preserve, Wrap::Never, Wrap::Width, Wrap::Sentence] {
        let out = wrapped(src, wrap, 3);
        assert!(
            out.starts_with("aaa") && out.contains("bbb\\\nccc\\\nddd"),
            "{wrap:?}: {out:?}"
        );
    }
}

#[test]
fn wrapping_never_breaks_inside_atoms() {
    let src = "a `x y z` b [l t](</u v> \"t i\") c <https://e.f/g> d <span class=\"k\">x</span> e $m n$ f\n";
    assert_eq!(
        wrapped(src, Wrap::Width, 1),
        "a\n`x y z`\nb\n[l\nt](</u v> \"t i\")\nc\n<https://e.f/g>\nd <span class=\"k\">x</span>\ne\n$m n$\nf\n"
    );
}

#[test]
fn wrapped_lines_never_start_a_block() {
    let src = "x - x + x * x # x > x 1. x 2) x === x --- x _ _ _ x <div> x ~~~ x\n";
    assert_eq!(
        wrapped(src, Wrap::Width, 1),
        "x\n\\-\nx\n\\+\nx\n\\*\nx\n\\#\nx\n\\>\nx\n1\\.\nx\n2\\)\nx\n\\===\nx\n\\---\nx\n_\n_\n_\nx <div>\nx\n\\~\\~\\~\nx\n"
    );
    assert_eq!(wrapped("aaaaa _ _ _\n", Wrap::Width, 5), "aaaaa\n\\_ _ _\n");
    // A table delimiter row and an HTML block start from plain text, a definition.
    let config = FmtConfig {
        wrap: Wrap::Width,
        width: 1,
        ..FmtConfig::default()
    };
    assert_eq!(
        format(&parse("a|b -|- \\<div\n", "gfm"), &config),
        "a|b\n\\-|-\n\\<div\n"
    );
    // Flavor syntax only where the flavor reads it: admonition headers in mkdocs.
    assert_eq!(
        format(&parse("a : b !!! c ??? d\n", "mkdocs"), &config),
        "a\n\\:\nb\n\\!!!\nc\n\\???\nd\n"
    );
    assert_eq!(
        format(&parse("a !!! c ??? d\n", "gfm"), &config),
        "a\n!!!\nc\n???\nd\n"
    );
}

#[test]
fn container_prefixes_count_toward_the_width() {
    assert_eq!(
        wrapped("> aaa bbb ccc\n", Wrap::Width, 9),
        "> aaa bbb\n> ccc\n"
    );
    assert_eq!(
        wrapped("10. aaa bbb ccc\n", Wrap::Width, 11),
        "10. aaa bbb\n    ccc\n"
    );
    // A task marker takes room on the first line only.
    assert_eq!(
        wrapped("- [ ] aaa bbb ccc ddd\n", Wrap::Width, 13),
        "- [ ] aaa bbb\n  ccc ddd\n"
    );
    assert_eq!(
        wrapped("> - aaa bbb\n", Wrap::Width, 7),
        "> - aaa\n>   bbb\n"
    );
}

#[test]
fn headings_and_tables_are_not_wrapped() {
    let src = "# A long heading\n\n| a b c |\n| ----- |\n";
    assert_eq!(wrapped(src, Wrap::Width, 3), src);
}

/// Paragraph-ish Markdown made of words that are syntax at the start of a line, inside
/// containers, with soft and hard breaks.
fn wrappable() -> impl Strategy<Value = String> {
    let word = prop_oneof![
        3 => "[a-z]{1,6}",
        1 => "[A-Z][a-z]{0,4}[.!?]",
        3 => prop::sample::select(vec![
            "-", "+", "*", "1.", "2)", "#", "##", ">", "=", "===", "---", "_", "___", "~~~",
            "```", "|", "-|-", ":--", ":", "<div", "<div>", "</div>", "<!--", "-->",
            "<span>", "`code`", "`a b`", "[link](/url \"t\")", "[ref]", "<https://x.y>",
            "www.x.com", "**bold**", "*em*", "_em_", "Dr.", "e.g.", "end.", "!", "!!!",
            "![img](/i)", "&amp;", "\\", "\\*", "$x$", "~~del~~", "[^1]", "...", "a@b.cd",
        ])
        .prop_map(str::to_owned),
    ];
    let gap = prop_oneof![6 => Just(" "), 2 => Just("\n"), 1 => Just("  \n"), 1 => Just("  ")];
    let line = prop::collection::vec((word, gap), 1..25).prop_map(|words| {
        words
            .into_iter()
            .flat_map(|(word, gap)| [word, gap.to_owned()])
            .collect::<String>()
    });
    // The first line's prefix and that of the others.
    let prefix = prop::sample::select(vec![
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("10. ", "    "),
        ("- [ ] ", "  "),
        ("> - ", ">   "),
    ]);
    (prefix, line).prop_map(|((first, rest), text)| {
        let mut out = String::new();
        for (i, line) in text.trim_end().lines().enumerate() {
            out.push_str(if i == 0 { first } else { rest });
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("\n[ref]: /r\n\n[^1]: note\n");
        out
    })
}

proptest! {
    /// Re-wrapping keeps the HTML and is idempotent, for every input the printer keeps
    /// as it is without wrapping (other inputs test the printer core, not wrapping). A
    /// paragraph right after a table (in a tight list item) is a known limit of the
    /// table printer: once the table has pipes, its lines are rows.
    #[test]
    fn rewrapped_paragraphs_keep_their_html(
        src in wrappable(),
        wrap in prop::sample::select(vec![Wrap::Never, Wrap::Width, Wrap::Sentence]),
        width in 1u16..40,
    ) {
        let doc = parse(&src, "gfm");
        let options = doc.flavor().parse_options();
        let before = html(&src, options);
        let preserved = format(&doc, &FmtConfig::default());
        if html(&preserved, options) != before
            || format(&parse(&preserved, "gfm"), &FmtConfig::default()) != preserved
            || before.split("</table>\n").skip(1).any(|rest| !rest.starts_with('<'))
        {
            return Ok(());
        }
        let config = FmtConfig { wrap, width, ..FmtConfig::default() };
        let once = format(&doc, &config);
        let after = html(&once, options);
        prop_assert!(
            same_html(&before, &after, wrap),
            "{src:?}\n--- formatted\n{once}--- before\n{before}--- after\n{after}"
        );
        let twice = format(&parse(&once, "gfm"), &config);
        prop_assert_eq!(&once, &twice, "not idempotent: {:?}", src);
    }
}

/// Paragraph-ish Markdown with the syntax of `flavor` (and text that looks like it)
/// inside the flavor's containers, as [`wrappable`] is for GFM.
fn flavored(flavor: &'static str) -> impl Strategy<Value = String> {
    let syntax: Vec<&str> = if flavor == "obsidian" {
        vec![
            "[[a]]",
            "[[a#b|c d]]",
            "![[x.png]]",
            "![[x|100]]",
            "#tag",
            "#1",
            "#a/b",
            "#",
            "\\#t",
            "%%c%%",
            "%%c",
            "d%%",
            "%%",
            "\\%%",
            "50%",
            "%",
            "[!note]",
            "[[",
            "]]",
            "==hi==",
            "$x$",
            "!",
        ]
    } else {
        vec![
            "!!!", "???", "???+", "!!! note", "\"t\"", ":", ":   x", "*[A]:", "*[", "]:",
            "\\(x\\)", "\\[y\\]", "\\(", "\\)", "\\[", "\\]", "(", ")", "[", "]", "{#x}", "{", "}",
            "$x$", "==hi==",
        ]
    };
    let word = prop_oneof![
        3 => "[a-z]{1,6}",
        3 => prop::sample::select(syntax).prop_map(str::to_owned),
        1 => prop::sample::select(vec![
            "-", "+", "*", "1.", "#", ">", "===", "---", "`code`", "[link](/u)", "*em*",
            "**b**", "\\", "&amp;", "<span>",
        ])
        .prop_map(str::to_owned),
    ];
    let gap = prop_oneof![6 => Just(" "), 2 => Just("\n"), 1 => Just("  \n")];
    let line = prop::collection::vec((word, gap), 1..20).prop_map(|words| {
        words
            .into_iter()
            .flat_map(|(word, gap)| [word, gap.to_owned()])
            .collect::<String>()
    });
    let prefixes = if flavor == "obsidian" {
        vec![
            ("", ""),
            ("> ", "> "),
            ("- ", "  "),
            ("> [!note] T\n> ", "> "),
            ("# ", ""),
        ]
    } else {
        vec![
            ("", ""),
            ("> ", "> "),
            ("- ", "  "),
            ("!!! note\n    ", "    "),
            ("Term\n:   ", "    "),
            ("## ", ""),
        ]
    };
    (prop::sample::select(prefixes), line).prop_map(|((first, rest), text)| {
        let mut out = String::new();
        for (i, line) in text.trim_end().lines().enumerate() {
            out.push_str(if i == 0 { first } else { rest });
            out.push_str(line);
            out.push('\n');
        }
        out
    })
}

proptest! {
    /// Flavor syntax keeps its structure under fmt, wrapped or not, and formats
    /// idempotently.
    #[test]
    fn flavor_syntax_keeps_its_structure(
        (flavor, src) in prop::sample::select(vec!["obsidian", "mkdocs"])
            .prop_flat_map(|flavor| (Just(flavor), flavored(flavor))),
        wrap in prop::sample::select(vec![Wrap::Preserve, Wrap::Never, Wrap::Width, Wrap::Sentence]),
        width in 1u16..40,
    ) {
        let doc = parse(&src, flavor);
        // Known limits: an admonition header in a definition term splits it into
        // blocks, which no term can hold; in a tight list item, an admonition header
        // right before a definition list, a table or an empty item becomes a term, a
        // table header or a setext heading; a line of a `%%` comment or math (verbatim
        // source) that continued a paragraph in a container lazily may start a block
        // once it is indented.
        let ast = doc.ast();
        prop_assume!(ast.descendants(ast.root()).all(|id| match ast.kind(id) {
            NodeKind::DefinitionTerm => ast.children(id).len() == 1,
            NodeKind::Admonition(_) => !ast.next_sibling(id).is_some_and(|next| {
                match ast.kind(next) {
                    NodeKind::DefinitionList | NodeKind::Table(_) => true,
                    NodeKind::List(_) => ast.children(ast.children(next)[0]).is_empty(),
                    _ => false,
                }
            }),
            NodeKind::Math(_) | NodeKind::Comment(_) => {
                let span = ast.span(id);
                let nested = ast.ancestors(id).filter(|&a| ast.kind(a).is_block()).count() > 2;
                !nested || !src[span.start..span.end].contains('\n')
            }
            _ => true,
        }));
        let config = FmtConfig { wrap, width, ..FmtConfig::default() };
        let once = format(&doc, &config);
        let reparsed = parse(&once, flavor);
        let (before, after) = (structure(&doc, wrap), structure(&reparsed, wrap));
        prop_assert!(
            before == after,
            "{flavor} {src:?}\n--- formatted\n{once}--- before\n{}\n--- after\n{}",
            serde_json::to_string(&before).unwrap(),
            serde_json::to_string(&after).unwrap()
        );
        let twice = format(&reparsed, &config);
        prop_assert_eq!(&once, &twice, "not idempotent: {} {:?}", flavor, src);
    }
}
