//! Acceptance tests for the CSS front-end (`Query::css`): the README examples, type,
//! attribute and pseudo-class selectors, combinators, selector lists, the compiled
//! [`QueryPlan`]s, and parse errors that point at the offending byte.

use mrq_core::Document;
use mrq_core::ast::NodeType;
use mrq_core::diag::Severity;
use mrq_core::query::{
    Attribute, Predicate, Query, QueryError, QueryPlan, Selector, Syntax, SyntaxError, TextMatcher,
};

/// No results.
const NONE: [&str; 0] = [];

const DOC: &str = "\
---
title: Demo
---

# Install

Get it from [crates.io](https://crates.io), [docs](./docs.md) or ![logo](logo.png).

```sh
cargo install mrq-cli
```

```rust
fn main() {}
```

## From source

```rust title=\"build\"
build()
```

- [ ] write docs
- [x] ship it
- plain item

1. first
2. second

> a quote
>
> ```sh
> quoted
> ```

# API reference

| a | b |
| - | - |
| 1 | 2 |

# Usage

Run `mrq`.

```rust
other()
```
";

fn compile(css: &str) -> Query {
    Query::css(css).unwrap_or_else(|err| panic!("{css}: {err}"))
}

/// The source slices of the selector's matches in [`DOC`].
fn select(css: &str) -> Vec<String> {
    let doc = Document::parse(DOC);
    doc.select(&compile(css))
        .unwrap()
        .iter()
        .map(|m| m.source(doc.text()).to_owned())
        .collect()
}

/// The plain texts of the selector's matches in [`DOC`].
fn texts(css: &str) -> Vec<String> {
    let doc = Document::parse(DOC);
    doc.select(&compile(css))
        .unwrap()
        .iter()
        .map(mrq_core::query::Match::text)
        .collect()
}

/// The heading texts of the sections the selector matches in [`DOC`].
fn headings(css: &str) -> Vec<String> {
    let doc = Document::parse(DOC);
    doc.select(&compile(css))
        .unwrap()
        .iter()
        .map(|m| m.section().expect("a section").heading().text())
        .collect()
}

fn plan(css: &str) -> QueryPlan {
    compile(css)
        .plan()
        .expect("CSS queries are compiled")
        .clone()
}

fn syntax_error(css: &str) -> SyntaxError {
    match Query::css(css) {
        Err(QueryError::Syntax(err)) => err,
        other => panic!("{css}: expected a syntax error, got {other:?}"),
    }
}

fn nodes(ty: NodeType) -> Selector {
    Selector::new(ty)
}

fn attr(attribute: Attribute, m: TextMatcher) -> Predicate {
    Predicate::Attr(attribute, m)
}

// --- README examples -----------------------------------------------------------------

#[test]
fn readme_rust_code_in_install_section() {
    // mrq query --css 'section[heading="Install"] code[lang=rust]' README.md
    assert_eq!(
        texts(r#"section[heading="Install"] code[lang=rust]"#),
        ["fn main() {}", "build()"]
    );
}

#[test]
fn readme_http_links() {
    // mrq query --css 'link[url^="http"]' docs/
    assert_eq!(
        select(r#"link[url^="http"]"#),
        ["[crates.io](https://crates.io)"]
    );
}

#[test]
fn section_heading_attribute_matches_sections() {
    let doc = Document::parse(DOC);
    let found = doc
        .select(&compile(r#"section[heading="API reference"]"#))
        .unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].is_section());
    assert_eq!(
        found[0].source(doc.text()),
        "# API reference\n\n| a | b |\n| - | - |\n| 1 | 2 |"
    );
    assert_eq!(headings(r#"section[heading="Install"]"#), ["Install"]);
    assert_eq!(headings(r#"section[heading="install"]"#), NONE);
    assert_eq!(headings(r#"section[heading="Inst"]"#), NONE, "`=` is exact");
}

// --- Type selectors ------------------------------------------------------------------

#[test]
fn type_selectors() {
    assert_eq!(
        headings("section"),
        ["Install", "From source", "API reference", "Usage"]
    );
    assert_eq!(select("heading").len(), 4);
    assert_eq!(select("h1").len(), 3);
    assert_eq!(select("h2"), ["## From source"]);
    assert_eq!(select("H2"), ["## From source"], "type names ignore case");
    assert_eq!(select("h3"), NONE);
    assert_eq!(select("code").len(), 5, "`code` is a code block");
    assert_eq!(select("code_block").len(), 5);
    assert_eq!(select("inline-code"), ["`mrq`"]);
    assert_eq!(select("link").len(), 2);
    assert_eq!(select("image"), ["![logo](logo.png)"]);
    assert_eq!(select("list").len(), 2);
    assert_eq!(select("item").len(), 5);
    assert_eq!(select("li").len(), 5);
    assert_eq!(select("task"), ["- [ ] write docs", "- [x] ship it"]);
    assert_eq!(select("ul > li").len(), 3);
    assert_eq!(select("ol > item"), ["1. first", "2. second"]);
    assert_eq!(select("table").len(), 1);
    assert_eq!(select("blockquote").len(), 1);
    assert_eq!(select("block_quote").len(), 1, "node type names work too");
    assert_eq!(texts("blockquote > p"), ["a quote"]);
    assert_eq!(select("frontmatter"), ["---\ntitle: Demo\n---"]);
}

#[test]
fn universal_selector() {
    assert_eq!(texts("[lang=sh]"), ["cargo install mrq-cli", "quoted"]);
    assert_eq!(texts("*[lang=sh]"), texts("[lang=sh]"));
    assert_eq!(select("*").len(), select("* ").len());
}

// --- Attribute selectors -------------------------------------------------------------

#[test]
fn attribute_operators() {
    assert_eq!(select("link[url=\"./docs.md\"]"), ["[docs](./docs.md)"]);
    assert_eq!(select("link[url$=\".md\"]"), ["[docs](./docs.md)"]);
    assert_eq!(
        select("link[url*=crates]"),
        ["[crates.io](https://crates.io)"]
    );
    assert_eq!(texts("code[info~='title=\"build\"']"), ["build()"]);
    assert_eq!(texts("code[info~=title]"), NONE, "`~=` matches whole words");
    assert_eq!(texts("code[info*=title]"), ["build()"]);
    assert_eq!(select("image[url='logo.png']"), ["![logo](logo.png)"]);
    assert_eq!(select("frontmatter[format=yaml]").len(), 1);
}

#[test]
fn attribute_presence_and_case_flags() {
    assert_eq!(select("code[lang]").len(), 5);
    assert_eq!(select("link[title]"), NONE);
    assert_eq!(select("code[lang=RUST]"), NONE, "values are case-sensitive");
    assert_eq!(select("code[lang=RUST i]").len(), 3);
    assert_eq!(select("code[lang=\"RUST\"I]").len(), 3);
    assert_eq!(select("code[lang=RUST s]"), NONE);
    assert_eq!(select("code[ lang = rust ]").len(), 3);
    assert_eq!(
        select("code[LANG=rust]").len(),
        3,
        "attribute names ignore case"
    );
}

#[test]
fn heading_text_and_level_attributes() {
    assert_eq!(headings("section[heading^=API]"), ["API reference"]);
    assert_eq!(headings("section[heading*=source]"), ["From source"]);
    assert_eq!(headings("section[heading=\"install\" i]"), ["Install"]);
    assert_eq!(headings("section[level=2]"), ["From source"]);
    assert_eq!(
        headings("section[heading$=e][heading^=F]"),
        ["From source"],
        "several heading attributes all apply"
    );
    assert_eq!(select("heading[heading=Usage]"), ["# Usage"]);
    assert_eq!(select("h1[level=1]").len(), 3);
    assert_eq!(
        texts("p[text*=crates]"),
        ["Get it from crates.io, docs or logo."]
    );
}

#[test]
fn quoted_values_support_escapes() {
    let doc = Document::parse("[a](x\"y) [b](x'y)\n");
    let run = |css: &str| -> Vec<String> {
        doc.select(&compile(css))
            .unwrap()
            .iter()
            .map(mrq_core::query::Match::text)
            .collect()
    };
    assert_eq!(run(r#"link[url="x\"y"]"#), ["a"]);
    assert_eq!(run(r"link[url='x\'y']"), ["b"]);
    assert_eq!(run(r#"link[url='x"y']"#), ["a"]);
}

// --- Combinators and lists -----------------------------------------------------------

#[test]
fn descendant_and_child_combinators() {
    assert_eq!(
        texts(r#"section[heading="Install"] code"#),
        ["cargo install mrq-cli", "fn main() {}", "build()", "quoted"]
    );
    assert_eq!(
        texts(r#"section[heading="Install"] > code"#),
        ["cargo install mrq-cli", "fn main() {}"]
    );
    assert_eq!(
        texts(r#"section[heading="Install"]>code"#),
        texts(r#"section[heading="Install"] > code"#)
    );
    assert_eq!(texts("section > section > code"), ["build()"]);
    assert_eq!(texts("blockquote code"), ["quoted"]);
    assert_eq!(texts("section blockquote > code"), ["quoted"]);
    assert_eq!(select("list > task:nth-child(2)"), ["- [x] ship it"]);
}

#[test]
fn selector_lists_are_unions_in_document_order() {
    assert_eq!(
        select("table, blockquote").len(),
        2,
        "blockquote first, then the table"
    );
    assert!(select("table, blockquote")[0].starts_with('>'));
    assert_eq!(select("code, code[lang=rust]").len(), 5, "deduplicated");
    assert_eq!(
        headings("section[level=2],section[heading=Usage]"),
        ["From source", "Usage"]
    );
}

// --- Pseudo-classes ------------------------------------------------------------------

#[test]
fn has_pseudo_class() {
    assert_eq!(headings("section:has(table)"), ["API reference"]);
    assert_eq!(
        headings("section:has(blockquote)"),
        ["Install", "From source"]
    );
    assert_eq!(headings("section:has(> blockquote)"), ["From source"]);
    assert_eq!(
        headings("section:has(code[lang=sh])"),
        ["Install", "From source"]
    );
    assert_eq!(headings("section:has(> code[lang=sh])"), ["Install"]);
    assert_eq!(
        headings("section:has(blockquote > code)"),
        ["Install", "From source"]
    );
    assert_eq!(
        headings("section:has(table, inline-code)"),
        ["API reference", "Usage"]
    );
    assert_eq!(select("list:has(> task)").len(), 1);
}

#[test]
fn not_pseudo_class() {
    assert_eq!(
        headings("section:not([heading=Usage])"),
        ["Install", "From source", "API reference"]
    );
    assert_eq!(headings("section:not(:has(code))"), ["API reference"]);
    assert_eq!(
        texts("code:not([lang=rust])"),
        ["cargo install mrq-cli", "quoted"]
    );
    assert_eq!(texts("code:not(blockquote code)").len(), 4);
    assert_eq!(
        texts(r#"code:not(section[heading="Install"] > code)"#),
        ["build()", "quoted", "other()"]
    );
    assert_eq!(headings("section:not(section[level=1])"), ["From source"]);
    assert_eq!(texts("code:not(section, table)").len(), 5);
    assert_eq!(
        select("item:not(task)"),
        ["- plain item", "1. first", "2. second"]
    );
}

#[test]
fn is_pseudo_class() {
    assert_eq!(select(":is(blockquote, table)").len(), 2);
    assert_eq!(texts("code:is(blockquote code)"), ["quoted"]);
}

#[test]
fn nth_child_pseudo_class() {
    assert_eq!(select("item:nth-child(2)"), ["- [x] ship it", "2. second"]);
    assert_eq!(
        select("item:nth-child(odd)"),
        ["- [ ] write docs", "- plain item", "1. first"]
    );
    assert_eq!(select("item:nth-child(even)"), select("item:nth-child(2n)"));
    assert_eq!(
        select("item:nth-child( 2n + 1 )"),
        select("item:nth-child(odd)")
    );
    assert_eq!(
        select("item:nth-child(-n+1)"),
        ["- [ ] write docs", "1. first"]
    );
    assert_eq!(select("item:nth-child(n+3)"), ["- plain item"]);
    assert_eq!(select("item:first-child"), select("item:nth-child(1)"));
    assert_eq!(select("item:nth-child(+3)"), ["- plain item"]);
}

// --- Plans ---------------------------------------------------------------------------

#[test]
fn compiled_plans() {
    let code = || nodes(NodeType::CodeBlock);
    assert_eq!(
        plan(r#"section[heading="Install"] code[lang=rust]"#),
        QueryPlan::section(TextMatcher::exact("Install")).descendant(QueryPlan::select(
            code().with(attr(Attribute::Lang, TextMatcher::exact("rust")))
        ))
    );
    assert_eq!(
        plan("h2"),
        QueryPlan::select(nodes(NodeType::Heading).with(Predicate::Level(2)))
    );
    assert_eq!(
        plan("task"),
        QueryPlan::select(nodes(NodeType::ListItem).with(Predicate::Task(None)))
    );
    assert_eq!(
        plan("ol"),
        QueryPlan::select(nodes(NodeType::List).with(Predicate::Ordered(true)))
    );
    assert_eq!(plan("section"), QueryPlan::section(TextMatcher::Any));
    assert_eq!(
        plan("section[level=2]"),
        QueryPlan::section(TextMatcher::Any)
            .filter(attr(Attribute::Level, TextMatcher::exact("2")))
    );
    assert_eq!(
        plan("[url^=http i]"),
        QueryPlan::select(Selector::any().with(attr(
            Attribute::Url,
            TextMatcher::prefix("http").ignore_case()
        )))
    );
    assert_eq!(
        plan("[lang]"),
        QueryPlan::select(Selector::any().with(attr(Attribute::Lang, TextMatcher::Any)))
    );
}

#[test]
fn combinator_and_list_plans() {
    let sel = |ty| QueryPlan::select(nodes(ty));
    assert_eq!(
        plan("blockquote > p code"),
        sel(NodeType::BlockQuote)
            .child(sel(NodeType::Paragraph))
            .descendant(sel(NodeType::CodeBlock))
    );
    assert_eq!(
        plan("table, blockquote"),
        QueryPlan::union([sel(NodeType::Table), sel(NodeType::BlockQuote)])
    );
}

#[test]
fn pseudo_class_plans() {
    let code = || QueryPlan::select(nodes(NodeType::CodeBlock));
    assert_eq!(
        plan("section:has(> code)"),
        QueryPlan::section(TextMatcher::Any)
            .filter(Predicate::Has(Box::new(QueryPlan::Scope.child(code()))))
    );
    assert_eq!(
        plan("section:has(code)"),
        QueryPlan::section(TextMatcher::Any).filter(Predicate::Has(Box::new(code())))
    );
    assert_eq!(
        plan("code:not([lang=sh])"),
        QueryPlan::select(nodes(NodeType::CodeBlock).with(Predicate::Not(Box::new(
            Selector::any().with(attr(Attribute::Lang, TextMatcher::exact("sh")))
        ))))
    );
    assert_eq!(
        plan("code:not(blockquote code)"),
        QueryPlan::select(nodes(NodeType::CodeBlock).with(Predicate::Not(Box::new(
            Selector::any().with(Predicate::Is(Box::new(
                QueryPlan::select(nodes(NodeType::BlockQuote)).descendant(code())
            )))
        ))))
    );
    assert_eq!(
        plan("section:not([heading=A])"),
        QueryPlan::section(TextMatcher::Any).filter(Predicate::Not(Box::new(
            Selector::any().with(Predicate::Heading(TextMatcher::exact("A")))
        )))
    );
    assert_eq!(
        plan("item:nth-child(-2n+3)"),
        QueryPlan::select(nodes(NodeType::ListItem).with(Predicate::NthChild { a: -2, b: 3 }))
    );
    assert_eq!(
        plan("item:first-child"),
        QueryPlan::select(nodes(NodeType::ListItem).with(Predicate::NthChild { a: 0, b: 1 }))
    );
    assert_eq!(
        plan("item:nth-child(odd)"),
        QueryPlan::select(nodes(NodeType::ListItem).with(Predicate::NthChild { a: 2, b: 1 }))
    );
    assert_eq!(
        plan("task:not(:nth-child(1))"),
        QueryPlan::select(nodes(NodeType::ListItem).with(Predicate::Task(None)).with(
            Predicate::Not(Box::new(
                Selector::any().with(Predicate::NthChild { a: 0, b: 1 })
            ))
        ))
    );
}

#[test]
fn css_queries_record_their_syntax_and_source() {
    let query = compile("code");
    assert_eq!(query.syntax(), Syntax::Css);
    assert_eq!(query.source(), "code");
    assert!(query.plan().is_some());
}

// --- Errors --------------------------------------------------------------------------

#[test]
fn syntax_errors_point_at_the_problem() {
    let cases: &[(&str, usize, &str)] = &[
        (
            "",
            0,
            "expected a selector (a type, `*`, `[attribute]` or `:pseudo-class`)",
        ),
        (
            "   ",
            3,
            "expected a selector (a type, `*`, `[attribute]` or `:pseudo-class`)",
        ),
        ("bogus", 0, "unknown type `bogus`"),
        ("code[bogus=1]", 5, "unknown attribute `bogus`"),
        ("code[lang=rust", 14, "expected `]`"),
        (
            "code[lang|=rust]",
            9,
            "expected an operator (`=`, `^=`, `$=`, `*=`, `~=`) or `]`",
        ),
        ("code[lang=]", 10, "expected a value"),
        ("code[lang=\"rust]", 16, "expected `\"`"),
        ("code[lang=rust x]", 15, "expected `]`"),
        ("code[]", 5, "expected an attribute name"),
        ("code:hover", 5, "unknown pseudo-class `hover`"),
        ("code:has", 8, "expected `(`"),
        ("code:has(code", 13, "expected `)`"),
        ("item:nth-child(x)", 15, "expected `an+b`, `odd` or `even`"),
        ("item:nth-child()", 15, "expected `an+b`, `odd` or `even`"),
        ("code]", 4, "expected a combinator, `,` or end of query"),
        (
            "code >",
            6,
            "expected a selector (a type, `*`, `[attribute]` or `:pseudo-class`)",
        ),
        (
            "code,",
            5,
            "expected a selector (a type, `*`, `[attribute]` or `:pseudo-class`)",
        ),
        (
            "> code",
            0,
            "expected a selector (a type, `*`, `[attribute]` or `:pseudo-class`)",
        ),
        (
            "section:has()",
            12,
            "expected a selector (a type, `*`, `[attribute]` or `:pseudo-class`)",
        ),
        (
            "code:not(> p)",
            9,
            "expected a selector (a type, `*`, `[attribute]` or `:pseudo-class`)",
        ),
    ];
    for &(css, offset, message) in cases {
        let err = syntax_error(css);
        assert_eq!((err.offset(), err.message()), (offset, message), "{css}");
        assert_eq!(err.query(), css);
    }
}

#[test]
fn syntax_errors_render_a_caret_and_convert_to_diagnostics() {
    let err = syntax_error("section code[lnag=rust]");
    assert_eq!(
        err.render(),
        "section code[lnag=rust]\n             ^ unknown attribute `lnag`"
    );
    let diagnostic = err.to_diagnostic("doc.md", 100);
    assert_eq!(diagnostic.code, "query");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.range.start, 113);
    assert_eq!(
        diagnostic.message,
        "invalid query: unknown attribute `lnag`"
    );
}
