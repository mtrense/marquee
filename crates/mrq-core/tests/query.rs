//! Acceptance tests for the query engine (`mrq_core::query`): hand-built
//! [`QueryPlan`]s evaluated over parsed documents.

use mrq_core::Document;
use mrq_core::ast::{Node, NodeType, TaskState};
use mrq_core::document::DocumentOptions;
use mrq_core::query::{
    self, Attribute, Match, Predicate, Query, QueryError, QueryPlan, Selector, Syntax, TextMatcher,
};

/// No results.
const NONE: [&str; 0] = [];

/// The source slices of the plan's matches, in result order.
fn run(doc: &Document, plan: &QueryPlan) -> Vec<String> {
    plan.evaluate(doc)
        .iter()
        .map(|m| m.source(doc.text()).to_owned())
        .collect()
}

fn nodes(ty: NodeType) -> QueryPlan {
    QueryPlan::select(Selector::new(ty))
}

fn any_section() -> QueryPlan {
    QueryPlan::section(TextMatcher::Any)
}

const GUIDE: &str = "\
intro

# Install

Get it:

```sh
make install
```

## From source

```rust
fn main() {}
```

### Linux

```sh
apt install x
```

# Usage

```sh
mrq --help
```
";

// --- Select --------------------------------------------------------------------------

#[test]
fn select_finds_nodes_of_a_type_in_document_order() {
    let doc = Document::parse(GUIDE);
    let found = run(&doc, &nodes(NodeType::CodeBlock));
    assert_eq!(found.len(), 4);
    assert!(found[0].contains("make install"));
    assert!(found[3].contains("mrq --help"));
}

#[test]
fn any_selector_with_a_predicate_matches_every_type() {
    let doc = Document::parse("A *b*\n\n- c\n");
    let plan = QueryPlan::select(Selector::any().with(Predicate::Text(TextMatcher::exact("b"))));
    // The emphasis and its text node both have the plain text `b`.
    let types: Vec<NodeType> = plan
        .evaluate(&doc)
        .iter()
        .map(|m| m.node().unwrap().node_type())
        .collect();
    assert_eq!(types, [NodeType::Emphasis, NodeType::Text]);
}

#[test]
fn attribute_predicates_read_kind_data() {
    let doc = Document::parse(GUIDE);
    let sh = QueryPlan::select(
        Selector::new(NodeType::CodeBlock)
            .with(Predicate::Attr(Attribute::Lang, TextMatcher::exact("sh"))),
    );
    assert_eq!(run(&doc, &sh).len(), 3);

    let doc = Document::parse("[a](https://x.org) [b](./b.md) ![c](http://img)\n");
    let external = QueryPlan::select(
        Selector::new(NodeType::Link)
            .with(Predicate::Attr(Attribute::Url, TextMatcher::prefix("http"))),
    );
    assert_eq!(run(&doc, &external), ["[a](https://x.org)"]);
    let any_url =
        QueryPlan::select(Selector::any().with(Predicate::Attr(Attribute::Url, TextMatcher::Any)));
    assert_eq!(run(&doc, &any_url).len(), 3, "links and images have a url");
}

#[test]
fn attribute_absent_never_matches() {
    let doc = Document::parse("```\nplain\n```\n");
    let plan = QueryPlan::select(
        Selector::new(NodeType::CodeBlock).with(Predicate::Attr(Attribute::Lang, TextMatcher::Any)),
    );
    assert_eq!(run(&doc, &plan), NONE);
}

#[test]
fn multi_valued_attributes_match_any_value() {
    let doc = DocumentOptions::new()
        .flavor("mkdocs")
        .parse("# Title {#top .wide .red}\n\n# Other\n")
        .unwrap();
    let plan = QueryPlan::select(
        Selector::new(NodeType::Heading)
            .with(Predicate::Attr(Attribute::Class, TextMatcher::exact("red"))),
    );
    assert_eq!(run(&doc, &plan).len(), 1);
    let by_id = QueryPlan::select(
        Selector::new(NodeType::Heading)
            .with(Predicate::Attr(Attribute::Id, TextMatcher::exact("top"))),
    );
    assert_eq!(run(&doc, &by_id).len(), 1);
}

#[test]
fn task_and_list_predicates() {
    let doc = Document::parse("- [ ] open\n- [x] done\n- plain\n\n1. first\n2. second\n");
    let items = |pred: Predicate| {
        run(
            &doc,
            &QueryPlan::select(Selector::new(NodeType::ListItem).with(pred)),
        )
    };
    assert_eq!(
        items(Predicate::Task(Some(TaskState::Open))),
        ["- [ ] open"]
    );
    assert_eq!(
        items(Predicate::Task(Some(TaskState::Done))),
        ["- [x] done"]
    );
    assert_eq!(items(Predicate::Task(None)).len(), 2);
    assert_eq!(items(Predicate::Ordered(true)).len(), 2);
    assert_eq!(items(Predicate::Ordered(false)).len(), 3);
    let lists = run(
        &doc,
        &QueryPlan::select(Selector::new(NodeType::List).with(Predicate::Ordered(true))),
    );
    assert_eq!(lists.len(), 1);
}

#[test]
fn level_predicate_and_attribute() {
    let doc = Document::parse(GUIDE);
    let h2 = QueryPlan::select(Selector::new(NodeType::Heading).with(Predicate::Level(2)));
    assert_eq!(run(&doc, &h2), ["## From source"]);
    let h1 = QueryPlan::select(
        Selector::new(NodeType::Heading)
            .with(Predicate::Attr(Attribute::Level, TextMatcher::exact("1"))),
    );
    assert_eq!(run(&doc, &h1), ["# Install", "# Usage"]);
}

#[test]
fn has_not_and_nth_child() {
    let doc = Document::parse("- a\n- b [x](y)\n- c\n- d\n");
    let item = Selector::new(NodeType::ListItem);
    let with_link = QueryPlan::select(
        item.clone()
            .with(Predicate::Has(Box::new(nodes(NodeType::Link)))),
    );
    assert_eq!(run(&doc, &with_link), ["- b [x](y)"]);
    let without_link = QueryPlan::select(item.clone().with(Predicate::Not(Box::new(
        Selector::any().with(Predicate::Has(Box::new(nodes(NodeType::Link)))),
    ))));
    assert_eq!(run(&doc, &without_link), ["- a", "- c", "- d"]);
    let even = QueryPlan::select(item.clone().with(Predicate::NthChild { a: 2, b: 0 }));
    assert_eq!(run(&doc, &even), ["- b [x](y)", "- d"]);
    let third = QueryPlan::select(item.clone().with(Predicate::NthChild { a: 0, b: 3 }));
    assert_eq!(run(&doc, &third), ["- c"]);
    let first_two = QueryPlan::select(item.with(Predicate::NthChild { a: -1, b: 2 }));
    assert_eq!(run(&doc, &first_two), ["- a", "- b [x](y)"]);
}

#[test]
fn not_with_a_type_excludes_that_type() {
    let doc = Document::parse("# H\n\np\n\n> q\n");
    let blocks = QueryPlan::child(
        nodes(NodeType::Document),
        QueryPlan::select(
            Selector::any().with(Predicate::Not(Box::new(Selector::new(NodeType::Paragraph)))),
        ),
    );
    // The heading's parent is its section, so only the document-level blocks outside
    // any section are its children; here everything is in section `H`.
    assert_eq!(run(&doc, &blocks), NONE);
    let in_section = QueryPlan::child(
        any_section(),
        QueryPlan::select(
            Selector::any().with(Predicate::Not(Box::new(Selector::new(NodeType::Paragraph)))),
        ),
    );
    assert_eq!(run(&doc, &in_section), ["# H", "> q"]);
}

// --- Text matchers -------------------------------------------------------------------

#[test]
fn text_matchers() {
    let cases = [
        (TextMatcher::Any, "", true),
        (TextMatcher::substring("stall"), "Install", true),
        (TextMatcher::substring("STALL"), "Install", false),
        (
            TextMatcher::substring("STALL").ignore_case(),
            "Install",
            true,
        ),
        (TextMatcher::exact("Install"), "Install", true),
        (TextMatcher::exact("Install"), "Install it", false),
        (TextMatcher::exact("install").ignore_case(), "INSTALL", true),
        (TextMatcher::prefix("In"), "Install", true),
        (TextMatcher::prefix("st"), "Install", false),
        (TextMatcher::suffix("all"), "Install", true),
        (TextMatcher::suffix("In"), "Install", false),
        (TextMatcher::word("red"), "wide red", true),
        (TextMatcher::word("re"), "wide red", false),
        (TextMatcher::regex("^API v[0-9]+$").unwrap(), "API v2", true),
        (TextMatcher::regex("^API").unwrap(), "The API", false),
    ];
    for (matcher, text, expected) in cases {
        assert_eq!(matcher.matches(text), expected, "{matcher:?} on {text:?}");
    }
}

#[test]
fn invalid_regex_is_an_error() {
    let err = TextMatcher::regex("(unclosed").unwrap_err();
    assert!(matches!(err, QueryError::Regex { .. }), "{err:?}");
    assert!(err.to_string().contains("(unclosed"), "{err}");
}

#[test]
fn regex_matchers_compare_by_pattern() {
    assert_eq!(
        TextMatcher::regex("a+").unwrap(),
        TextMatcher::regex("a+").unwrap()
    );
    assert_ne!(
        TextMatcher::regex("a+").unwrap(),
        TextMatcher::regex("b+").unwrap()
    );
}

// --- Sections ------------------------------------------------------------------------

#[test]
fn sections_run_to_the_next_heading_of_the_same_or_higher_level() {
    let doc = Document::parse(GUIDE);
    let found = run(&doc, &any_section());
    assert_eq!(found.len(), 4);
    assert!(found[0].starts_with("# Install\n"));
    assert!(found[0].ends_with("apt install x\n```"), "{:?}", found[0]);
    assert!(found[1].starts_with("## From source\n"));
    assert!(found[1].ends_with("apt install x\n```"));
    assert_eq!(found[2], "### Linux\n\n```sh\napt install x\n```");
    assert_eq!(found[3], "# Usage\n\n```sh\nmrq --help\n```");
}

#[test]
fn section_without_content_is_its_heading() {
    let doc = Document::parse("# A\n## B\n# C\n");
    assert_eq!(run(&doc, &any_section()), ["# A\n## B", "## B", "# C"]);
}

#[test]
fn sections_list_heading_level_and_content() {
    let doc = Document::parse(GUIDE);
    let all = query::sections(doc.ast());
    assert_eq!(all.len(), 4);
    let install = &all[0];
    assert_eq!(install.level(), 1);
    assert_eq!(install.heading().text(), "Install");
    let content: Vec<NodeType> = install.content().map(Node::node_type).collect();
    assert_eq!(
        content,
        [
            NodeType::Paragraph,
            NodeType::CodeBlock,
            NodeType::Heading,
            NodeType::CodeBlock,
            NodeType::Heading,
            NodeType::CodeBlock,
        ]
    );
    assert_eq!(all[2].heading().text(), "Linux");
    assert_eq!(all[2].level(), 3);
    assert_eq!(
        install.text(),
        "Install\nGet it:\nmake install\nFrom source\nfn main() {}\nLinux\napt install x"
    );
}

#[test]
fn sections_stay_inside_their_container() {
    let doc = Document::parse("# Out\n\n> # In\n>\n> quoted\n\nafter\n");
    let found = run(&doc, &any_section());
    assert_eq!(
        found,
        ["# Out\n\n> # In\n>\n> quoted\n\nafter", "# In\n>\n> quoted"]
    );
}

#[test]
fn section_matchers_match_the_heading_text() {
    let doc = Document::parse(GUIDE);
    let headings = |m: TextMatcher| -> Vec<String> {
        QueryPlan::section(m)
            .evaluate(&doc)
            .iter()
            .map(|m| m.section().unwrap().heading().text())
            .collect()
    };
    assert_eq!(
        headings(TextMatcher::substring("SOURCE").ignore_case()),
        ["From source"]
    );
    assert_eq!(headings(TextMatcher::exact("Usage")), ["Usage"]);
    assert_eq!(
        headings(TextMatcher::regex("^[IL]").unwrap()),
        ["Install", "Linux"]
    );
    assert_eq!(headings(TextMatcher::exact("usage")), NONE);
}

// --- Combinators ---------------------------------------------------------------------

#[test]
fn pipe_selects_within_each_result() {
    // README: `# Install | ```sh`
    let doc = Document::parse(GUIDE);
    let plan = QueryPlan::section(TextMatcher::substring("install").ignore_case()).pipe(
        QueryPlan::select(
            Selector::new(NodeType::CodeBlock)
                .with(Predicate::Attr(Attribute::Lang, TextMatcher::exact("sh"))),
        ),
    );
    assert_eq!(
        run(&doc, &plan),
        ["```sh\nmake install\n```", "```sh\napt install x\n```"]
    );
}

#[test]
fn pipe_includes_the_context_but_descendant_does_not() {
    let doc = Document::parse(GUIDE);
    let install = QueryPlan::section(TextMatcher::exact("Install"));
    let piped: Vec<String> = install
        .clone()
        .pipe(any_section())
        .evaluate(&doc)
        .iter()
        .map(|m| m.section().unwrap().heading().text())
        .collect();
    assert_eq!(piped, ["Install", "From source", "Linux"]);
    let nested: Vec<String> = install
        .descendant(any_section())
        .evaluate(&doc)
        .iter()
        .map(|m| m.section().unwrap().heading().text())
        .collect();
    assert_eq!(nested, ["From source", "Linux"]);
}

#[test]
fn pipe_on_nodes_includes_the_node_itself() {
    let doc = Document::parse("> a\n>\n> > b\n");
    let quote = nodes(NodeType::BlockQuote);
    assert_eq!(run(&doc, &quote.clone().pipe(quote.clone())).len(), 2);
    assert_eq!(run(&doc, &quote.clone().descendant(quote)), ["> b"]);
}

#[test]
fn child_matches_direct_children_only() {
    let doc = Document::parse("- a\n  - b\n- c\n");
    let plan = nodes(NodeType::List).child(nodes(NodeType::ListItem));
    // Every list item is a child of some list.
    assert_eq!(run(&doc, &plan).len(), 3);
    let top = nodes(NodeType::Document).child(nodes(NodeType::List));
    assert_eq!(run(&doc, &top), ["- a\n  - b\n- c"]);
}

#[test]
fn child_of_a_section_is_its_heading_content_and_direct_subsections() {
    let doc = Document::parse(GUIDE);
    let install = QueryPlan::section(TextMatcher::exact("Install"));
    let subsections: Vec<String> = install
        .clone()
        .child(any_section())
        .evaluate(&doc)
        .iter()
        .map(|m| m.section().unwrap().heading().text())
        .collect();
    assert_eq!(subsections, ["From source"]);
    let code = install.child(nodes(NodeType::CodeBlock));
    assert_eq!(run(&doc, &code), ["```sh\nmake install\n```"]);
    let heading = any_section().child(nodes(NodeType::Heading));
    assert_eq!(
        run(&doc, &heading).len(),
        4,
        "each section owns its heading"
    );
}

#[test]
fn chained_descendants_need_strict_nesting() {
    // CSS `blockquote blockquote > paragraph`: the inner quote must be inside the outer.
    let doc = Document::parse("> a\n");
    let quote = nodes(NodeType::BlockQuote);
    let plan = quote
        .clone()
        .descendant(quote.child(nodes(NodeType::Paragraph)));
    assert_eq!(run(&doc, &plan), NONE);
}

#[test]
fn union_is_deduplicated_in_document_order() {
    let doc = Document::parse("<b>x</b>\n\n<div>\nblock\n</div>\n\n# H\n");
    let html = QueryPlan::union([
        nodes(NodeType::HtmlBlock),
        nodes(NodeType::HtmlInline),
        nodes(NodeType::HtmlBlock),
        any_section(),
    ]);
    let found = run(&doc, &html);
    assert_eq!(found, ["<b>", "</b>", "<div>\nblock\n</div>", "# H"]);
}

#[test]
fn section_sorts_before_its_heading() {
    let doc = Document::parse("# H\n");
    let found: Vec<bool> = QueryPlan::union([nodes(NodeType::Heading), any_section()])
        .evaluate(&doc)
        .iter()
        .map(Match::is_section)
        .collect();
    assert_eq!(found, [true, false]);
}

#[test]
fn filter_applies_predicates_to_sections() {
    let doc = Document::parse(GUIDE);
    let with_rust = any_section().filter(Predicate::Has(Box::new(QueryPlan::select(
        Selector::new(NodeType::CodeBlock)
            .with(Predicate::Attr(Attribute::Lang, TextMatcher::exact("rust"))),
    ))));
    let headings: Vec<String> = with_rust
        .evaluate(&doc)
        .iter()
        .map(|m| m.section().unwrap().heading().text())
        .collect();
    assert_eq!(headings, ["Install", "From source"]);
    let level_one = any_section().filter(Predicate::Level(1));
    assert_eq!(run(&doc, &level_one).len(), 2);
}

// --- Matches and Document::select -----------------------------------------------------

#[test]
fn matches_expose_span_text_and_target() {
    let doc = Document::parse("# A\n\nx *y*\n");
    let found = any_section().evaluate(&doc);
    let section = &found[0];
    assert!(section.is_section());
    assert!(section.node().is_none());
    assert_eq!(section.span().start, 0);
    assert_eq!(section.text(), "A\nx y");
    let para = nodes(NodeType::Paragraph).evaluate(&doc);
    assert_eq!(para[0].node().unwrap().node_type(), NodeType::Paragraph);
    assert_eq!(para[0].text(), "x y");
    assert!(para[0].section().is_none());
}

#[test]
fn document_select_evaluates_plan_queries() {
    let doc = Document::parse(GUIDE);
    let query = Query::from_plan(
        QueryPlan::section(TextMatcher::exact("Usage")).pipe(nodes(NodeType::CodeBlock)),
    );
    assert_eq!(query.syntax(), Syntax::Plan);
    assert!(query.plan().is_some());
    let found = doc.select(&query).unwrap();
    let texts: Vec<String> = found.iter().map(Match::text).collect();
    assert_eq!(texts, ["mrq --help"]);
}

#[test]
fn text_queries_are_not_compiled_yet() {
    let doc = Document::parse(GUIDE);
    let query = Query::selector("# Install").unwrap();
    assert!(query.plan().is_none());
    assert!(matches!(
        doc.select(&query).unwrap_err(),
        QueryError::NotImplemented
    ));
}
