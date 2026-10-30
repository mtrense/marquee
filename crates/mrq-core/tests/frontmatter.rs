//! Frontmatter detection, parsing, span lookup and integration with the parsed tree.

use mrq_core::ast::{FrontMatterFormat, NodeKind, NodeType};
use mrq_core::diag::Severity;
use mrq_core::frontmatter::{FRONTMATTER_ERROR_CODE, Frontmatter, parse_document};
use mrq_core::parse::{ComrakParser, ParseOptions};
use mrq_core::source::ByteRange;
use serde_json::json;

fn extract(src: &str) -> Frontmatter {
    Frontmatter::extract(src).expect("frontmatter detected")
}

fn slice(src: &str, range: ByteRange) -> &str {
    &src[range.start..range.end]
}

/// Key and value source text of the entry at `path`.
fn entry<'a>(src: &'a str, fm: &Frontmatter, path: &[&str]) -> (Option<&'a str>, &'a str) {
    let entry = fm
        .entry(path)
        .unwrap_or_else(|| panic!("no entry at {path:?}"));
    (
        entry.key.map(|key| slice(src, key)),
        slice(src, entry.value),
    )
}

#[test]
fn yaml_is_detected_and_parsed() {
    let src = "---\ntitle: Hello\ntags: [a, b]\n---\n# Body\n";
    let fm = extract(src);
    assert_eq!(fm.format(), FrontMatterFormat::Yaml);
    assert_eq!(
        slice(src, fm.span()),
        "---\ntitle: Hello\ntags: [a, b]\n---"
    );
    assert_eq!(slice(src, fm.content()), "title: Hello\ntags: [a, b]\n");
    assert_eq!(&src[fm.body_start()..], "# Body\n");
    assert_eq!(
        fm.value(),
        Some(&json!({"title": "Hello", "tags": ["a", "b"]}))
    );
    assert!(fm.error().is_none());
}

#[test]
fn toml_is_detected_and_parsed() {
    let src = "+++\ntitle = \"Hello\"\ndate = 2024-01-02\n[mrq]\nflavor = \"gfm\"\n+++\nBody\n";
    let fm = extract(src);
    assert_eq!(fm.format(), FrontMatterFormat::Toml);
    assert_eq!(&src[fm.body_start()..], "Body\n");
    assert_eq!(
        fm.value(),
        Some(&json!({"title": "Hello", "date": "2024-01-02", "mrq": {"flavor": "gfm"}}))
    );
    assert_eq!(fm.get_str(&["mrq", "flavor"]), Some("gfm"));
}

#[test]
fn no_frontmatter_without_both_delimiters_at_the_start() {
    for src in [
        "",
        "# Title\n",
        "---\n",
        "---",
        "---\ntitle: x\n",
        "\n---\na: 1\n---\n",
        " ---\na: 1\n---\n",
        "----\na: 1\n----\n",
        "---\na: 1\n+++\n",
        "+++\na = 1\n---\n",
    ] {
        assert!(Frontmatter::extract(src).is_none(), "{src:?}");
    }
}

#[test]
fn delimiters_allow_trailing_whitespace_and_closing_at_eof() {
    let src = "--- \na: 1\n---\t";
    let fm = extract(src);
    assert_eq!(fm.value(), Some(&json!({"a": 1})));
    assert_eq!(fm.body_start(), src.len());
    assert_eq!(slice(src, fm.span()), src);
}

#[test]
fn empty_frontmatter_is_an_empty_mapping() {
    for src in [
        "---\n---\n",
        "---\n\n  \n---\n",
        "---\n# only a comment\n---\n",
        "+++\n+++\n",
    ] {
        let fm = extract(src);
        assert_eq!(fm.value(), Some(&json!({})), "{src:?}");
        assert!(fm.error().is_none(), "{src:?}");
        assert_eq!(fm.get(&["a"]), None);
    }
    let src = "---\n---\nBody";
    let fm = extract(src);
    assert_eq!(slice(src, fm.content()), "");
    assert_eq!(&src[fm.body_start()..], "Body");
}

#[test]
fn malformed_yaml_yields_a_diagnostic() {
    let src = "---\ntitle: [unclosed\n---\nBody\n";
    let fm = extract(src);
    assert_eq!(fm.value(), None);
    assert_eq!(fm.entry(&["title"]), None);
    let error = fm.error().expect("error");
    assert!(error.range.start >= fm.content().start && error.range.end <= fm.content().end);
    let diag = fm.diagnostic("doc.md").expect("diagnostic");
    assert_eq!(diag.code, FRONTMATTER_ERROR_CODE);
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.file, std::path::PathBuf::from("doc.md"));
    assert_eq!(diag.range, error.range);
    assert!(diag.message.contains("YAML"), "{}", diag.message);
    // The body is still found.
    assert_eq!(&src[fm.body_start()..], "Body\n");
}

#[test]
fn malformed_yaml_error_points_at_the_offending_token() {
    let src = "---\na: 1\nb: c: d\n---\n";
    let error = extract(src).error().cloned().expect("error");
    assert_eq!(&src[error.range.start..=error.range.start], ":");
    assert_eq!(src[..error.range.start].lines().count(), 3);
}

#[test]
fn duplicate_yaml_keys_are_malformed() {
    let fm = extract("---\na: 1\na: 2\n---\n");
    assert!(fm.error().is_some());
}

#[test]
fn malformed_toml_yields_a_diagnostic() {
    let src = "+++\ntitle = \n+++\n";
    let fm = extract(src);
    assert_eq!(fm.value(), None);
    let error = fm.error().expect("error");
    assert_eq!(error.range.start, src.find(" \n").unwrap() + 1);
    assert!(error.message.contains("TOML"), "{}", error.message);
    assert!(fm.diagnostic("a.md").is_some());
}

#[test]
fn non_mapping_frontmatter_is_malformed() {
    for src in ["---\n- a\n- b\n---\n", "---\njust text\n---\n"] {
        let fm = extract(src);
        assert_eq!(fm.value(), None, "{src:?}");
        let error = fm.error().expect("error");
        assert!(error.message.contains("mapping"), "{}", error.message);
        assert_eq!(slice(src, error.range), slice(src, fm.content()).trim_end());
    }
}

#[test]
fn diagnostic_is_none_for_valid_frontmatter() {
    assert!(extract("---\na: 1\n---\n").diagnostic("a.md").is_none());
}

#[test]
fn bom_before_the_opening_delimiter() {
    let src = "\u{feff}---\na: 1\n---\nBody\n";
    let fm = extract(src);
    assert_eq!(fm.span().start, 3);
    assert_eq!(slice(src, fm.span()), "---\na: 1\n---");
    assert_eq!(fm.value(), Some(&json!({"a": 1})));
    assert_eq!(entry(src, &fm, &["a"]), (Some("a"), "1"));

    let src = "\u{feff}+++\na = 1\n+++\n";
    assert_eq!(extract(src).get(&["a"]), Some(&json!(1)));
}

#[test]
fn crlf_line_endings() {
    let src = "---\r\ntitle: Hello\r\nnested:\r\n  key: value\r\n---\r\nBody\r\n";
    let fm = extract(src);
    assert_eq!(
        slice(src, fm.span()),
        "---\r\ntitle: Hello\r\nnested:\r\n  key: value\r\n---"
    );
    assert_eq!(
        slice(src, fm.content()),
        "title: Hello\r\nnested:\r\n  key: value\r\n"
    );
    assert_eq!(&src[fm.body_start()..], "Body\r\n");
    assert_eq!(fm.get_str(&["nested", "key"]), Some("value"));
    assert_eq!(entry(src, &fm, &["nested", "key"]), (Some("key"), "value"));
    assert_eq!(entry(src, &fm, &["nested"]), (Some("nested"), "key: value"));

    let src = "+++\r\n[a]\r\nb = 1\r\n+++\r\n";
    let fm = extract(src);
    assert_eq!(fm.get(&["a", "b"]), Some(&json!(1)));
    assert_eq!(entry(src, &fm, &["a", "b"]), (Some("b"), "1"));
}

#[test]
fn malformed_crlf_error_points_into_the_right_line() {
    let src = "---\r\na: 1\r\nb: [\r\n---\r\n";
    let error = extract(src).error().cloned().expect("error");
    let line = src[..error.range.start].matches('\n').count() + 1;
    assert!(line >= 3, "line {line}");
}

#[test]
fn nested_yaml_key_span_lookup() {
    let src = "---\n\
title: \"Quoted\"\n\
author:\n  name: Ada\n  links:\n    - https://a.example\n    - { rel: home, href: /h }\n\
mrq:\n  flavor: obsidian\n\
tags: [x, y]\n\
summary: |\n  line one\n  line two\n\n\
empty:\n\
---\nBody\n";
    let fm = extract(src);
    assert_eq!(entry(src, &fm, &["title"]), (Some("title"), "\"Quoted\""));
    assert_eq!(entry(src, &fm, &["author", "name"]), (Some("name"), "Ada"));
    assert_eq!(
        entry(src, &fm, &["author", "links", "0"]),
        (None, "https://a.example")
    );
    assert_eq!(
        entry(src, &fm, &["author", "links", "1"]),
        (None, "{ rel: home, href: /h }")
    );
    assert_eq!(
        entry(src, &fm, &["author", "links", "1", "href"]),
        (Some("href"), "/h")
    );
    assert_eq!(
        entry(src, &fm, &["author", "links"]),
        (
            Some("links"),
            "- https://a.example\n    - { rel: home, href: /h }"
        )
    );
    assert_eq!(
        entry(src, &fm, &["mrq", "flavor"]),
        (Some("flavor"), "obsidian")
    );
    assert_eq!(entry(src, &fm, &["tags"]), (Some("tags"), "[x, y]"));
    assert_eq!(entry(src, &fm, &["tags", "1"]), (None, "y"));
    assert_eq!(
        entry(src, &fm, &["summary"]),
        (Some("summary"), "line one\n  line two")
    );
    let (key, value) = entry(src, &fm, &["empty"]);
    assert_eq!((key, value), (Some("empty"), ""));
    assert_eq!(fm.entry(&["author", "missing"]), None);
    assert_eq!(fm.entry(&["tags", "2"]), None);
    // The root is the whole mapping.
    assert_eq!(fm.entry::<&str>(&[]).unwrap().key, None);
    assert!(slice(src, fm.entry::<&str>(&[]).unwrap().value).starts_with("title"));
}

#[test]
fn nested_toml_key_span_lookup() {
    let src = "+++\n\
title = \"Hello\"\n\
author.name = \"Ada\"\n\
tags = [\"x\", { y = 1 }]\n\
[mrq]\n\
flavor = \"gfm\"\n\
[[links]]\n\
href = \"/a\"\n\
[[links]]\n\
href = \"/b\"\n\
+++\n";
    let fm = extract(src);
    assert_eq!(entry(src, &fm, &["title"]), (Some("title"), "\"Hello\""));
    assert_eq!(
        entry(src, &fm, &["author", "name"]),
        (Some("name"), "\"Ada\"")
    );
    assert_eq!(entry(src, &fm, &["tags", "0"]), (None, "\"x\""));
    assert_eq!(entry(src, &fm, &["tags", "1", "y"]), (Some("y"), "1"));
    assert_eq!(entry(src, &fm, &["mrq"]), (Some("mrq"), "[mrq]"));
    assert_eq!(
        entry(src, &fm, &["mrq", "flavor"]),
        (Some("flavor"), "\"gfm\"")
    );
    assert_eq!(entry(src, &fm, &["links", "1"]), (None, "[[links]]"));
    assert_eq!(
        entry(src, &fm, &["links", "1", "href"]),
        (Some("href"), "\"/b\"")
    );
    assert_eq!(fm.get_str(&["links", "1", "href"]), Some("/b"));
    assert_eq!(fm.entry(&["links", "2"]), None);
}

#[test]
fn json_pointer_lookup() {
    let src = "---\na/b:\n  - x\n  - k~: v\n---\n";
    let fm = extract(src);
    assert_eq!(fm.pointer("/a~1b/1/k~0"), Some(&json!("v")));
    let entry = fm.entry_at_pointer("/a~1b/1/k~0").unwrap();
    assert_eq!(slice(src, entry.key.unwrap()), "k~");
    assert_eq!(slice(src, entry.value), "v");
    assert_eq!(fm.entry_at_pointer("").unwrap().key, None);
    assert_eq!(fm.entry_at_pointer("no-slash"), None);
}

#[test]
fn parse_document_adds_a_front_matter_node_and_offsets_body_spans() {
    let src = "\u{feff}---\r\ntitle: x\r\n---\r\n# Heading\r\n\r\nText\r\n";
    let parser = ComrakParser::new(ParseOptions::gfm());
    let (ast, fm) = parse_document(src, &parser);
    let fm = fm.expect("frontmatter");
    let root = ast.node(ast.root());
    assert_eq!(root.span(), ByteRange::new(0, src.len()));
    let children: Vec<_> = root.children().collect();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].node_type(), NodeType::FrontMatter);
    assert_eq!(children[0].span(), fm.span());
    let NodeKind::FrontMatter(payload) = children[0].kind() else {
        panic!("not frontmatter");
    };
    assert_eq!(payload.format, FrontMatterFormat::Yaml);
    assert_eq!(payload.literal, "title: x\r\n");
    assert_eq!(children[1].source(src), "# Heading");
    assert_eq!(children[2].source(src), "Text");
    let text = children[2].first_child().unwrap();
    assert_eq!(text.source(src), "Text");
}

#[test]
fn parse_document_without_frontmatter_is_a_plain_parse() {
    let src = "---\n\nText\n";
    let parser = ComrakParser::new(ParseOptions::gfm());
    let (ast, fm) = parse_document(src, &parser);
    assert!(fm.is_none());
    let first = ast.node(ast.children(ast.root())[0]);
    assert_eq!(first.node_type(), NodeType::ThematicBreak);
}

#[test]
fn parse_document_with_malformed_frontmatter_still_parses_the_body() {
    let src = "---\na: [\n---\nText\n";
    let parser = ComrakParser::new(ParseOptions::gfm());
    let (ast, fm) = parse_document(src, &parser);
    assert!(fm.unwrap().error().is_some());
    let children = ast.children(ast.root());
    assert_eq!(ast.kind(children[0]).node_type(), NodeType::FrontMatter);
    assert_eq!(ast.node(children[1]).source(src), "Text");
}
