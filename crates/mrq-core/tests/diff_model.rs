//! Acceptance tests for the normalized block model of `mrq_core::diff`: a section tree
//! whose blocks are in a canonical form, so formatting-only changes compare equal and
//! real changes do not.

use mrq_core::Document;
use mrq_core::ast::NodeType;
use mrq_core::diff::{self, Section};

/// Every section's path with the canonical forms of its heading and blocks, in document
/// order: what a comparison of two documents looks at.
fn shape(src: &str) -> Vec<(Vec<String>, Vec<String>)> {
    let root = diff::normalize(&Document::parse(src));
    root.iter()
        .map(|section| {
            let blocks = section
                .heading
                .iter()
                .chain(&section.blocks)
                .map(|block| block.canonical.clone())
                .collect();
            (section.path.clone(), blocks)
        })
        .collect()
}

fn assert_same(a: &str, b: &str) {
    assert_eq!(shape(a), shape(b), "{a:?} and {b:?} should compare equal");
}

fn assert_different(a: &str, b: &str) {
    assert_ne!(shape(a), shape(b), "{a:?} and {b:?} should differ");
}

#[test]
fn sections_nest_by_heading_level() {
    let root = diff::normalize(&Document::parse(
        "intro\n\n# A\n\none\n\n### B *x*\n\ntwo\n\n## C\n\n# D\n",
    ));
    assert_eq!(root.level, 0);
    assert!(root.heading.is_none());
    assert_eq!(root.blocks.len(), 1);
    let paths: Vec<_> = root.iter().map(|s| (s.level, s.path.join(" / "))).collect();
    assert_eq!(
        paths,
        [
            (0, String::new()),
            (1, "A".to_owned()),
            (3, "A / B x".to_owned()),
            (2, "A / C".to_owned()),
            (1, "D".to_owned()),
        ]
    );
    let a = &root.children[0];
    assert_eq!(a.title, "A");
    assert_eq!(a.heading.as_ref().unwrap().canonical, "# A");
    assert_eq!(a.blocks[0].canonical, "one");
    assert_eq!(
        a.children[0].heading.as_ref().unwrap().canonical,
        "### B *x*"
    );
}

#[test]
fn spans_and_lines_point_into_the_original() {
    let src = "# A\n\nfirst\nline\n\n- x\n- y\n\n# B\n";
    let doc = Document::parse(src);
    let root = diff::normalize(&doc);
    let a = &root.children[0];
    assert_eq!(
        &src[a.span.start..a.span.end].trim_end(),
        &"# A\n\nfirst\nline\n\n- x\n- y"
    );
    let paragraph = &a.blocks[0];
    assert_eq!(paragraph.kind, NodeType::Paragraph);
    assert_eq!(
        &src[paragraph.span.start..paragraph.span.end],
        "first\nline"
    );
    assert_eq!(paragraph.lines, 2..4);
    assert_eq!(paragraph.canonical, "first line");
    let list = &a.blocks[1];
    assert_eq!(list.kind, NodeType::List);
    assert_eq!(list.lines, 5..7);
    assert_eq!(root.children[1].span.start, src.find("# B").unwrap());
}

#[test]
fn reflowed_paragraphs_compare_equal() {
    assert_same(
        "# T\n\nThe quick brown fox\njumps over the lazy dog.\n",
        "# T\n\nThe quick\nbrown fox jumps over\nthe lazy   dog.\n",
    );
    assert_same("> one\n> two\n", "> one two\n");
    assert_same("- item that\n  wraps\n", "- item that wraps\n");
}

#[test]
fn bullet_markers_compare_equal() {
    let dash = "- a\n- b\n  - c\n";
    assert_same(dash, "* a\n* b\n  * c\n");
    assert_same(dash, "+ a\n+ b\n    + c\n");
}

#[test]
fn ordered_list_numbering_compares_equal() {
    assert_same("1. a\n2. b\n3. c\n", "1. a\n1. b\n1. c\n");
    assert_same("1. a\n2. b\n", "1) a\n7) b\n");
}

#[test]
fn realigned_tables_compare_equal() {
    let tight = "|a|b|\n|-|:-:|\n|long cell|x|\n";
    let padded = "| a         |  b  |\n| --------- | :---: |\n| long cell |  x  |\n";
    assert_same(tight, padded);
    assert_same(tight, "a | b\n---|:---:\nlong cell | x\n");
    let table = &diff::normalize(&Document::parse(padded)).blocks[0];
    assert_eq!(table.kind, NodeType::Table);
    assert_eq!(
        table.canonical,
        "| a | b |\n| --- | :-: |\n| long cell | x |"
    );
}

#[test]
fn other_formatting_only_changes_compare_equal() {
    assert_same("Title\n=====\n\n_a_ __b__\n", "# Title\n\n*a* **b**\n");
    assert_same("    code\n", "```\ncode\n```\n");
    assert_same("***\n", "---\n");
    assert_same("# A #\n", "#   A\n");
}

#[test]
fn blocks_print_alone_whatever_comes_before() {
    // Adjacent lists alternate markers when printed together; alone they do not.
    let [_, list] =
        <[_; 2]>::try_from(diff::normalize(&Document::parse("- a\n\n* b\n")).blocks).unwrap();
    assert_eq!(list.canonical, "- b");
    // A thematic break first in the document is `***` in fmt, `---` here.
    let root = diff::normalize(&Document::parse("***\n\ntext\n"));
    assert_eq!(root.blocks[0].canonical, "---");
}

#[test]
fn reference_definitions_are_blocks_of_their_own() {
    let root = diff::normalize(&Document::parse("See [x].\n\n[x]:   /url   'T'\n"));
    let kinds: Vec<_> = root
        .blocks
        .iter()
        .map(|b| (b.kind, b.canonical.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (NodeType::Paragraph, "See [x]."),
            (NodeType::LinkDefinition, "[x]: /url \"T\""),
        ]
    );
    assert_different("See [x].\n\n[x]: /a\n", "See [x].\n\n[x]: /b\n");
}

#[test]
fn real_changes_compare_unequal() {
    assert_different("# T\n\nThe quick fox.\n", "# T\n\nThe slow fox.\n");
    assert_different("- a\n- b\n", "1. a\n2. b\n");
    assert_different("1. a\n2. b\n", "3. a\n4. b\n");
    assert_different("- a\n- b\n", "- a\n\n- b\n");
    assert_different("- [ ] a\n", "- [x] a\n");
    assert_different("|a|b|\n|-|-|\n|1|2|\n", "|a|b|\n|-|-|\n|1|3|\n");
    assert_different("|a|b|\n|-|-|\n|1|2|\n", "|a|b|\n|-|-:|\n|1|2|\n");
    assert_different("*a*\n", "**a**\n");
    assert_different("```rust\nx\n```\n", "```rust\n  x\n```\n");
    assert_different("# A\n\ntext\n", "## A\n\ntext\n");
    assert_different("# A\n\ntext\n", "# B\n\ntext\n");
    assert_different("one\n\ntwo\n", "one\ntwo\n");
}

#[test]
fn only_top_level_headings_open_sections() {
    let root = diff::normalize(&Document::parse("> # Quoted\n\n- # In a list\n"));
    assert_eq!(root.children, []);
    assert_eq!(root.blocks.len(), 2);
}

#[test]
fn frontmatter_line_endings_and_empty_documents() {
    assert_same(
        "---\ntitle:   x\n---\n\ntext\n",
        "---\ntitle: x\n---\n\ntext\n",
    );
    assert_different("---\ntitle: x\n---\n", "---\ntitle: y\n---\n");
    assert_same("# A\r\n\r\none\r\ntwo\r\n", "# A\n\none two\n");
    let root = diff::normalize(&Document::parse(""));
    assert_eq!((root.blocks.len(), root.children.len()), (0, 0));
}

#[test]
fn iter_visits_sections_depth_first() {
    let root: Section = diff::normalize(&Document::parse("# A\n## B\n# C\n"));
    let titles: Vec<_> = root.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["", "A", "B", "C"]);
}
