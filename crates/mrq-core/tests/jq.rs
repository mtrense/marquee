//! Acceptance tests for the jq front-end (`Query::jq`): the README example, the mrq
//! helper definitions, JSON outputs and their mapping back to the source, and compile and
//! runtime errors as diagnostics.

use mrq_core::Document;
use mrq_core::diag::Severity;
use mrq_core::query::{JsonOutput, Match, Query, QueryError, Syntax, SyntaxError};
use serde_json::{Value, json};

const DOC: &str = "\
---
title: Demo
tags: [a, b]
---

# Install

Get it from [crates.io](https://crates.io) or [docs](./docs.md).

```sh
cargo install mrq-cli
```

## From source

```Rust
build()
```

# Usage

Run `mrq` *now*.
";

fn compile(jq: &str) -> Query {
    Query::jq(jq).unwrap_or_else(|err| panic!("{jq}: {err}"))
}

/// The JSON values the program outputs on [`DOC`].
fn values(jq: &str) -> Vec<Value> {
    let doc = Document::parse(DOC);
    doc.select_json(&compile(jq))
        .unwrap_or_else(|err| panic!("{jq}: {err}"))
        .into_iter()
        .map(JsonOutput::into_value)
        .collect()
}

fn syntax_error(jq: &str) -> SyntaxError {
    match Query::jq(jq) {
        Err(QueryError::Syntax(err)) => err,
        other => panic!("{jq}: expected a syntax error, got {other:?}"),
    }
}

fn runtime_error(jq: &str) -> QueryError {
    let doc = Document::parse(DOC);
    match doc.select_json(&compile(jq)) {
        Err(err) => err,
        Ok(found) => panic!("{jq}: expected an error, got {found:?}"),
    }
}

// --- README example ------------------------------------------------------------------

#[test]
fn readme_link_urls() {
    assert_eq!(
        values(r#".. | select(.type? == "link") | .url"#),
        [json!("https://crates.io"), json!("./docs.md")]
    );
}

#[test]
fn indexing_numbers_and_strings_fails_as_in_jq() {
    // Why the README example uses `.type?`: `..` also yields numbers and strings.
    let err = runtime_error(r#".. | select(.type == "link")"#);
    assert_eq!(err.to_string(), r#"jq error: cannot index 1 with "type""#);
}

#[test]
fn jq_queries_record_their_syntax_and_source() {
    let query = compile(".");
    assert_eq!(query.syntax(), Syntax::Jq);
    assert_eq!(query.source(), ".");
    assert!(query.plan().is_none());
}

#[test]
fn queries_are_send_and_sync() {
    fn check<T: Send + Sync>(_: &T) {}
    check(&compile("."));
}

// --- Input and standard library ------------------------------------------------------

#[test]
fn the_input_is_the_json_projection() {
    let value = values(".").remove(0);
    let projected: Value =
        serde_json::from_str(&Document::parse(DOC).to_json_ast().to_json_string()).unwrap();
    assert_eq!(value, projected);
    assert_eq!(
        values(".frontmatter.title, .mrq_ast, (.children | length)"),
        [json!("Demo"), json!(1), json!(8)]
    );
}

#[test]
fn the_standard_library_is_available() {
    assert_eq!(
        values(r#"[.. | objects | .type | strings] | map(select(test("^h"))) | unique"#),
        [json!(["heading"])]
    );
    assert_eq!(
        values(".frontmatter.tags | map(ascii_upcase) | join(\",\")"),
        [json!("A,B")]
    );
}

#[test]
fn numbers_and_constructed_values_convert_to_json() {
    assert_eq!(
        values(r#"1, 1.5, 1e1000, -0, "x", null, true, [1, {"a": 2}], (1 / 3 | . * 3)"#),
        [
            json!(1),
            json!(1.5),
            json!(f64::MAX),
            json!(0),
            json!("x"),
            Value::Null,
            json!(true),
            json!([1, {"a": 2}]),
            json!(1.0),
        ]
    );
    assert_eq!(values("nan, infinite"), [Value::Null, json!(f64::MAX)]);
    assert_eq!(values("{(1 | tostring): 2}"), [json!({"1": 2})]);
}

// --- mrq helpers ---------------------------------------------------------------------

#[test]
fn nodes_walks_the_tree_without_frontmatter_values() {
    let types = values("[nodes | .type] | unique");
    assert_eq!(
        types,
        [json!([
            "code",
            "emphasis",
            "frontmatter",
            "heading",
            "inlineCode",
            "link",
            "paragraph",
            "root",
            "text"
        ])]
    );
}

#[test]
fn links_yields_link_nodes() {
    assert_eq!(
        values("links | .url"),
        [json!("https://crates.io"), json!("./docs.md")]
    );
    assert_eq!(values("links | text"), [json!("crates.io"), json!("docs")]);
}

#[test]
fn code_selects_code_blocks_by_language_ignoring_case() {
    assert_eq!(values("code | .lang"), [json!("sh"), json!("Rust")]);
    assert_eq!(values(r#"code("rust") | .value"#), [json!("build()\n")]);
    assert_eq!(values(r#"code("RUST") | .value"#), [json!("build()\n")]);
    assert_eq!(values(r#"[code("py")]"#), [json!([])]);
}

#[test]
fn text_is_the_plain_text_of_nodes_and_sections() {
    assert_eq!(values(".children[-1] | text"), [json!("Run mrq now.")]);
    assert_eq!(values(r#""as is" | text"#), [json!("as is")]);
    assert_eq!(
        values(r#"sections | select(.heading | text == "From source") | text"#),
        [json!("From source\nbuild()")]
    );
}

#[test]
fn sections_yields_heading_and_content_with_a_span() {
    let found = values(
        "sections | {depth, heading: (.heading | text), content: [.children[].type], start: .position.start.offset, end: .position.end.offset}",
    );
    let install = DOC.find("# Install").unwrap();
    let usage = DOC.find("# Usage").unwrap();
    let source = DOC.find("## From").unwrap();
    assert_eq!(
        found,
        [
            json!({"depth": 1, "heading": "Install", "content": ["paragraph", "code", "heading", "code"], "start": install, "end": usage - 2}),
            json!({"depth": 2, "heading": "From source", "content": ["code"], "start": source, "end": usage - 2}),
            json!({"depth": 1, "heading": "Usage", "content": ["paragraph"], "start": usage, "end": DOC.len() - 1}),
        ]
    );
    assert_eq!(values("[sections] | .[0].type"), [json!("section")]);
}

#[test]
fn sections_are_scoped_to_the_input() {
    assert_eq!(
        values(r#"sections | select(.heading | text == "Install") | [sections | .heading | text]"#),
        [json!(["From source"])]
    );
    assert_eq!(values("[.children[2] | sections]"), [json!([])]);
    assert_eq!(
        values(r#"[sections | select(.depth == 1) | nodes | select(.type == "code") | .lang]"#),
        [json!(["sh", "Rust"])]
    );
}

// --- Outputs back to the source ------------------------------------------------------

#[test]
fn node_outputs_map_back_to_matches() {
    let doc = Document::parse(DOC);
    let outputs = doc
        .select_json(&compile(r#"code("sh"), links, sections, 42"#))
        .unwrap();
    let sources: Vec<Option<&str>> = outputs
        .iter()
        .map(|output| output.span().map(|span| &DOC[span.start..span.end]))
        .collect();
    assert_eq!(
        sources,
        [
            Some("```sh\ncargo install mrq-cli\n```"),
            Some("[crates.io](https://crates.io)"),
            Some("[docs](./docs.md)"),
            Some(&DOC[DOC.find("# Install").unwrap()..DOC.find("# Usage").unwrap() - 2]),
            Some("## From source\n\n```Rust\nbuild()\n```"),
            Some("# Usage\n\nRun `mrq` *now*."),
            None,
        ]
    );
    let matches: Vec<Option<Match<'_>>> = outputs.iter().map(|o| o.to_match(&doc)).collect();
    assert!(matches[0].is_some_and(|m| m.node().is_some()));
    assert_eq!(matches[0].unwrap().text(), "cargo install mrq-cli");
    assert!(matches[3].is_some_and(|m| m.is_section()));
    assert_eq!(
        matches[3].unwrap().section().unwrap().heading().text(),
        "Install"
    );
    assert!(matches[6].is_none());
}

#[test]
fn edited_or_foreign_objects_do_not_map_to_nodes() {
    let doc = Document::parse(DOC);
    let outputs = doc
        .select_json(&compile(
            "(links | .position.start.offset += 1), (links | del(.id)), {id: 0}",
        ))
        .unwrap();
    assert!(outputs.iter().all(|o| o.to_match(&doc).is_none()));
    // A span is still there without the id.
    assert_eq!(
        outputs[2].span().map(|s| &DOC[s.start..s.end]),
        Some("[crates.io](https://crates.io)")
    );
}

#[test]
fn select_maps_jq_outputs_to_matches() {
    let doc = Document::parse(DOC);
    let found = doc.select(&compile(r#"code("sh"), sections"#)).unwrap();
    let texts: Vec<String> = found.iter().map(Match::text).collect();
    assert_eq!(texts[0], "cargo install mrq-cli");
    assert_eq!(found.len(), 4);
    assert!(found[1].is_section());

    let err = doc.select(&compile(".children[0], 1")).unwrap_err();
    assert!(
        matches!(&err, QueryError::NotANode { value } if value == "1"),
        "{err:?}"
    );
}

#[test]
fn plan_queries_project_their_matches() {
    let doc = Document::parse(DOC);
    let outputs = doc
        .select_json(&Query::selector("# Install | ```sh").unwrap())
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].value()["type"], "code");
    assert_eq!(outputs[0].value()["lang"], "sh");
    let sections = doc.select_json(&Query::css("section").unwrap()).unwrap();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[1].value()["type"], "section");
    assert_eq!(sections[1].value()["depth"], 2);
    assert_eq!(sections[1].value()["heading"]["type"], "heading");
    assert!(sections[1].to_match(&doc).is_some_and(|m| m.is_section()));
}

// --- Errors --------------------------------------------------------------------------

#[test]
fn compile_errors_point_at_the_problem() {
    let cases: &[(&str, usize, &str)] = &[
        ("", 0, "expected term"),
        (".[", 1, "unclosed `[`, expected closing bracket"),
        ("\"abc", 0, "unclosed `\"`, expected closing quote"),
        ("if . then 1", 11, "expected else or end"),
        (". | foo", 4, "undefined filter `foo/0`"),
        ("code(1; 2)", 0, "undefined filter `code/2`"),
        ("$x", 0, "undefined variable `$x`"),
        (". & .", 2, "unexpected token `&`"),
        (". )", 2, "unexpected token `)`"),
        ("[1,]", 3, "expected term, found `]`"),
    ];
    for &(jq, offset, message) in cases {
        let err = syntax_error(jq);
        assert_eq!((err.offset(), err.message()), (offset, message), "{jq}");
        assert_eq!(err.query(), jq);
    }
}

#[test]
fn compile_errors_render_a_caret_and_convert_to_diagnostics() {
    let err = syntax_error("links | txt");
    assert_eq!(
        err.render(),
        "links | txt\n        ^ undefined filter `txt/0`"
    );
    let diagnostic = err.to_diagnostic("doc.md", 100);
    assert_eq!(diagnostic.code, "query");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.range.start, 108);
    assert_eq!(
        diagnostic.message,
        "invalid query: undefined filter `txt/0`"
    );
}

#[test]
fn runtime_errors_are_query_errors_with_diagnostics() {
    let err = runtime_error(".children[0] | .type + 1");
    let QueryError::Runtime(runtime) = &err else {
        panic!("expected a runtime error, got {err:?}");
    };
    assert_eq!(runtime.query(), ".children[0] | .type + 1");
    assert!(
        runtime.message().contains("cannot"),
        "{}",
        runtime.message()
    );
    assert_eq!(err.to_string(), format!("jq error: {}", runtime.message()));
    let diagnostic = runtime.to_diagnostic("doc.md", 10);
    assert_eq!(diagnostic.code, "query");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!((diagnostic.range.start, diagnostic.range.end), (10, 34));
    assert_eq!(
        diagnostic.message,
        format!("jq error: {}", runtime.message())
    );

    let QueryError::Runtime(custom) = runtime_error(r#"error("boom")"#) else {
        panic!("expected a runtime error");
    };
    assert_eq!(custom.message(), "boom");
    let QueryError::Runtime(halt) = runtime_error("halt") else {
        panic!("expected a runtime error");
    };
    assert_eq!(halt.message(), "halt with exit code 0");
    let QueryError::Runtime(text) = runtime_error("1 | text") else {
        panic!("expected a runtime error");
    };
    assert_eq!(
        text.message(),
        "text: expected a node, a section or a string, got 1"
    );
}
