//! Acceptance tests for the selector DSL (`Query::selector`): every README example
//! parses and returns the expected nodes, the cheat sheet compiles to the expected
//! [`QueryPlan`]s, and parse errors point at the offending byte.

use mrq_core::Document;
use mrq_core::ast::{NodeType, TaskState};
use mrq_core::diag::Severity;
use mrq_core::query::{
    Attribute, Predicate, Query, QueryError, QueryPlan, Selector, SyntaxError, TextMatcher,
};

/// No results.
const NONE: [&str; 0] = [];

const DOC: &str = "\
---
title: Demo
---

# Install

Get it from [crates.io](https://crates.io) or ![logo](logo.png).

```sh
cargo install mrq-cli
```

```bash
echo bash
```

```rust
fn main() {}
```

## From source

- [ ] write docs
- [x] ship it
- plain item

1. first
2. second

> a quote

<div>html</div>

# API reference

| a | b |
| - | - |
| 1 | 2 |

# Usage

```rust
other()
```

| x |
| - |
| y |
";

/// The source slices of the selector's matches in [`DOC`].
fn select(selector: &str) -> Vec<String> {
    select_in(DOC, selector)
}

fn select_in(src: &str, selector: &str) -> Vec<String> {
    let doc = Document::parse(src);
    let query = Query::selector(selector).unwrap_or_else(|err| panic!("{selector}: {err}"));
    doc.select(&query)
        .unwrap()
        .iter()
        .map(|m| m.source(doc.text()).to_owned())
        .collect()
}

/// The heading texts of the sections the selector matches in [`DOC`].
fn headings(selector: &str) -> Vec<String> {
    let doc = Document::parse(DOC);
    doc.select(&Query::selector(selector).unwrap())
        .unwrap()
        .iter()
        .map(|m| m.section().expect("a section").heading().text())
        .collect()
}

fn plan(selector: &str) -> QueryPlan {
    Query::selector(selector)
        .unwrap_or_else(|err| panic!("{selector}: {err}"))
        .plan()
        .expect("selector queries are compiled")
        .clone()
}

fn syntax_error(selector: &str) -> SyntaxError {
    match Query::selector(selector) {
        Err(QueryError::Syntax(err)) => err,
        other => panic!("{selector}: expected a syntax error, got {other:?}"),
    }
}

fn nodes(ty: NodeType) -> Selector {
    Selector::new(ty)
}

// --- README examples -----------------------------------------------------------------

#[test]
fn readme_install_rust_code_blocks() {
    // mrq query '# Install | ```rust' README.md
    assert_eq!(
        select("# Install | ```rust"),
        ["```rust\nfn main() {}\n```"]
    );
}

#[test]
fn readme_install_shell_snippets() {
    // mrq query '# Install | ```sh' README.md (also the library example and `select=`)
    assert_eq!(
        select("# Install | ```sh"),
        ["```sh\ncargo install mrq-cli\n```"]
    );
}

#[test]
fn readme_all_links() {
    // mrq query '[]()' docs/
    assert_eq!(select("[]()"), ["[crates.io](https://crates.io)"]);
}

#[test]
fn readme_open_tasks() {
    // mrq query '- [ ]' TODO.md
    assert_eq!(select("- [ ]"), ["- [ ] write docs"]);
}

#[test]
fn readme_tables_under_api_headings() {
    // mrq query '# /^API/ | :-:' api.md
    assert_eq!(
        select("# /^API/ | :-:"),
        ["| a | b |\n| - | - |\n| 1 | 2 |"]
    );
}

// --- Cheat sheet ---------------------------------------------------------------------

#[test]
fn section_selector_matches_heading_text() {
    assert_eq!(headings("# install"), ["Install"]);
    assert_eq!(headings("# Install"), ["Install"]);
    assert_eq!(headings("#source"), ["From source"]);
    assert_eq!(
        headings("#"),
        ["Install", "From source", "API reference", "Usage"]
    );
}

#[test]
fn list_item_selectors() {
    assert_eq!(
        select("-"),
        ["- [ ] write docs", "- [x] ship it", "- plain item"]
    );
    assert_eq!(select("- plain"), ["- plain item"]);
    assert_eq!(select("1."), ["1. first", "2. second"]);
    assert_eq!(select("1. sec"), ["2. second"]);
}

#[test]
fn task_selectors() {
    assert_eq!(select("- [x]"), ["- [x] ship it"]);
    assert_eq!(select("- [X]"), ["- [x] ship it"]);
    assert_eq!(select("- [?]"), ["- [ ] write docs", "- [x] ship it"]);
    assert_eq!(select("- [ ] docs"), ["- [ ] write docs"]);
    assert_eq!(select("- [ ] ship"), NONE);
    assert_eq!(
        select("1. [ ]"),
        NONE,
        "`1.` tasks are in ordered lists only"
    );
}

#[test]
fn link_and_image_selectors() {
    assert_eq!(select("[crates]()"), ["[crates.io](https://crates.io)"]);
    assert_eq!(select("[](https)"), ["[crates.io](https://crates.io)"]);
    assert_eq!(select("[text](url)"), NONE);
    assert_eq!(select("![]()"), ["![logo](logo.png)"]);
    assert_eq!(select("![logo](/\\.png$/)"), ["![logo](logo.png)"]);
}

#[test]
fn block_selectors() {
    assert_eq!(select(">"), ["> a quote"]);
    assert_eq!(select("> quote"), ["> a quote"]);
    assert_eq!(select("> nope"), NONE);
    assert_eq!(
        select("P: crates"),
        ["Get it from [crates.io](https://crates.io) or ![logo](logo.png)."]
    );
    assert_eq!(select(":-:").len(), 2);
    assert_eq!(select(":-: y"), ["| x |\n| - |\n| y |"]);
}

#[test]
fn code_block_selectors() {
    assert_eq!(select("```").len(), 4);
    assert_eq!(select("```rust").len(), 2);
    assert_eq!(select("```RUST").len(), 2, "bare languages ignore case");
    assert_eq!(
        select("```sh"),
        ["```sh\ncargo install mrq-cli\n```"],
        "not `bash`"
    );
    assert_eq!(select("```rust main"), ["```rust\nfn main() {}\n```"]);
    assert_eq!(
        select("``` echo"),
        ["```bash\necho bash\n```"],
        "a space ends the language"
    );
    assert_eq!(select("```/sh$/").len(), 2);
    assert_eq!(select("```* other"), ["```rust\nother()\n```"]);
}

#[test]
fn html_selector() {
    assert_eq!(select("</>"), ["<div>html</div>"]);
    assert_eq!(select("</> div"), ["<div>html</div>"]);
    assert_eq!(select("</> span"), NONE);
    let src = "a <b>bold</b>\n\n<!-- note -->\n";
    assert_eq!(select_in(src, "</>"), ["<b>", "</b>", "<!-- note -->"]);
    assert_eq!(select_in(src, "</> note"), ["<!-- note -->"]);
}

#[test]
fn frontmatter_selector() {
    assert_eq!(select("+++"), ["---\ntitle: Demo\n---"]);
    assert_eq!(select("+++ demo"), ["---\ntitle: Demo\n---"]);
    assert_eq!(select("+++yaml").len(), 1);
    assert_eq!(select("+++toml"), NONE);
    assert_eq!(select("+++ other"), NONE);
    assert_eq!(select_in("+++\na = 1\n+++\n\nx\n", "+++toml").len(), 1);
}

#[test]
fn pipes_chain_left_to_right() {
    assert_eq!(select("# Install | - | []()"), NONE);
    assert_eq!(select("# Install | # source | - [x]"), ["- [x] ship it"]);
    assert_eq!(select("# usage|```"), ["```rust\nother()\n```"]);
    assert_eq!(select("> | P:"), ["a quote"]);
}

// --- Text matchers -------------------------------------------------------------------

#[test]
fn text_matchers() {
    assert_eq!(headings("# \"install\""), NONE);
    assert_eq!(headings("# \"Install\""), ["Install"]);
    assert_eq!(headings("# \"Inst\""), NONE, "quoted is exact");
    assert_eq!(headings("# ^api"), ["API reference"]);
    assert_eq!(headings("# source$"), ["From source"]);
    assert_eq!(headings("# ^usage$"), ["Usage"]);
    assert_eq!(headings("# ^us$"), NONE);
    assert_eq!(
        headings("# /e$/"),
        ["From source", "API reference", "Usage"]
    );
    assert_eq!(headings("# *"), headings("#"));
    assert_eq!(
        headings("# api reference"),
        ["API reference"],
        "bare text spans words"
    );
}

#[test]
fn matcher_plans() {
    let section = |m: TextMatcher| QueryPlan::section(m);
    assert_eq!(
        plan("# Install"),
        section(TextMatcher::substring("Install").ignore_case())
    );
    assert_eq!(
        plan("#   two  words  "),
        section(TextMatcher::substring("two  words").ignore_case())
    );
    assert_eq!(
        plan("# \"Install\""),
        section(TextMatcher::exact("Install"))
    );
    assert_eq!(
        plan("# \"a \\\" | b\""),
        section(TextMatcher::exact("a \" | b"))
    );
    assert_eq!(
        plan("# ^In"),
        section(TextMatcher::prefix("In").ignore_case())
    );
    assert_eq!(
        plan("# all$"),
        section(TextMatcher::suffix("all").ignore_case())
    );
    assert_eq!(
        plan("# ^Install$"),
        section(TextMatcher::exact("Install").ignore_case())
    );
    assert_eq!(plan("# ^\"In\""), section(TextMatcher::prefix("In")));
    assert_eq!(plan("# \"all\"$"), section(TextMatcher::suffix("all")));
    assert_eq!(
        plan("# ^\"Install\"$"),
        section(TextMatcher::exact("Install"))
    );
    assert_eq!(
        plan("# /^A\\/B/"),
        section(TextMatcher::regex("^A/B").unwrap())
    );
    assert_eq!(plan("# *"), section(TextMatcher::Any));
    assert_eq!(plan("#"), section(TextMatcher::Any));
}

#[test]
fn selector_plans() {
    let text = |s: &str| Predicate::Text(TextMatcher::substring(s).ignore_case());
    assert_eq!(
        plan("- a"),
        QueryPlan::select(
            nodes(NodeType::ListItem)
                .with(Predicate::Ordered(false))
                .with(text("a"))
        )
    );
    assert_eq!(
        plan("1."),
        QueryPlan::select(nodes(NodeType::ListItem).with(Predicate::Ordered(true)))
    );
    assert_eq!(
        plan("- [ ]"),
        QueryPlan::select(
            nodes(NodeType::ListItem)
                .with(Predicate::Ordered(false))
                .with(Predicate::Task(Some(TaskState::Open)))
        )
    );
    assert_eq!(
        plan("[a](b)"),
        QueryPlan::select(nodes(NodeType::Link).with(text("a")).with(Predicate::Attr(
            Attribute::Url,
            TextMatcher::substring("b").ignore_case()
        )))
    );
    assert_eq!(plan("![]()"), QueryPlan::select(nodes(NodeType::Image)));
    assert_eq!(
        plan("```rust x"),
        QueryPlan::select(
            nodes(NodeType::CodeBlock)
                .with(Predicate::Attr(
                    Attribute::Lang,
                    TextMatcher::exact("rust").ignore_case()
                ))
                .with(text("x"))
        )
    );
    assert_eq!(plan(">"), QueryPlan::select(nodes(NodeType::BlockQuote)));
    assert_eq!(plan(":-:"), QueryPlan::select(nodes(NodeType::Table)));
    assert_eq!(
        plan("P: x"),
        QueryPlan::select(nodes(NodeType::Paragraph).with(text("x")))
    );
    assert_eq!(
        plan("+++toml x"),
        QueryPlan::select(
            nodes(NodeType::FrontMatter)
                .with(Predicate::Attr(
                    Attribute::Format,
                    TextMatcher::exact("toml").ignore_case()
                ))
                .with(Predicate::Attr(
                    Attribute::Raw,
                    TextMatcher::substring("x").ignore_case()
                ))
        )
    );
    assert_eq!(
        plan("# a | ```"),
        QueryPlan::section(TextMatcher::substring("a").ignore_case())
            .pipe(QueryPlan::select(nodes(NodeType::CodeBlock)))
    );
    assert_eq!(
        plan(">|>|>"),
        QueryPlan::select(nodes(NodeType::BlockQuote))
            .pipe(QueryPlan::select(nodes(NodeType::BlockQuote)))
            .pipe(QueryPlan::select(nodes(NodeType::BlockQuote)))
    );
}

#[test]
fn query_keeps_its_source_and_plan() {
    let query = Query::selector("# Install | ```sh").unwrap();
    assert_eq!(query.source(), "# Install | ```sh");
    assert_eq!(query.plan(), Some(&plan("# Install | ```sh")));
}

// --- Errors --------------------------------------------------------------------------

#[test]
fn errors_point_at_the_offending_byte() {
    let cases = [
        ("", 0, "expected a selector"),
        ("   ", 3, "expected a selector"),
        ("?", 0, "expected a selector"),
        ("# Install |", 11, "expected a selector"),
        ("# Install | | ```", 12, "expected a selector"),
        ("[text](url", 10, "expected `)`"),
        ("[text", 5, "expected `]`"),
        ("[text] (url)", 6, "expected `(`"),
        ("# \"open", 7, "expected `\"`"),
        ("# /open", 7, "expected `/`"),
        ("# * x", 4, "expected `|` or end of query"),
        ("# \"a\" b", 6, "expected `|` or end of query"),
        ("# /(unclosed/", 2, "invalid regex"),
    ];
    for (selector, offset, message) in cases {
        let err = syntax_error(selector);
        assert_eq!(err.offset(), offset, "{selector:?}: {err}");
        assert!(
            err.message().contains(message),
            "{selector:?}: {}",
            err.message()
        );
        assert_eq!(err.query(), selector);
    }
}

#[test]
fn errors_render_with_a_caret() {
    let err = syntax_error("# A | ?");
    let rendered = err.render();
    let mut lines = rendered.lines();
    assert_eq!(lines.next(), Some("# A | ?"));
    let caret = lines.next().unwrap();
    assert!(
        caret.starts_with("      ^ expected a selector"),
        "{caret:?}"
    );
    assert_eq!(lines.next(), None);
    assert!(err.to_string().contains("expected a selector"), "{err}");
    assert!(err.to_string().contains('6'), "{err}");
}

#[test]
fn caret_counts_characters_not_bytes() {
    let err = syntax_error("# ünïcode | ?");
    let rendered = err.render();
    let caret = rendered.lines().nth(1).unwrap();
    assert_eq!(caret.find('^'), Some(12), "{rendered}");
    assert_eq!(err.offset(), 14);
}

#[test]
fn errors_map_to_diagnostics() {
    let err = syntax_error("# A | ?");
    assert_eq!(err.range().start, 6);
    assert_eq!(err.range().end, 7);
    assert_eq!(syntax_error("# A |").range().start, 5);
    assert_eq!(syntax_error("# A |").range().end, 5);
    // A query written at byte 100 of a file (e.g. an include directive's `select=`).
    let diag = err.to_diagnostic("book.md", 100);
    assert_eq!(diag.code, "query");
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.file.to_str(), Some("book.md"));
    assert_eq!((diag.range.start, diag.range.end), (106, 107));
    assert!(diag.message.contains("expected a selector"));
}

#[test]
fn regex_errors_are_syntax_errors() {
    let err = syntax_error("# A | ```/[/");
    assert_eq!(err.offset(), 9);
    assert!(
        err.message().starts_with("invalid regex"),
        "{}",
        err.message()
    );
    assert!(!err.message().contains('\n'), "{:?}", err.message());
}
