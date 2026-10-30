//! Acceptance tests for `mrq_core::toc`: rendering a document's outline as indented
//! text, as a nested Markdown list of anchor links, and as JSON.

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_core::toc::{self, JsonToc};

const GUIDE: &str = "# Guide\n\nintro\n\n## Install *now*\n\n### From [source](x)\n\n## Use\n";

#[test]
fn outline_indents_by_nesting() {
    let doc = Document::parse(GUIDE);
    assert_eq!(
        toc::outline(&doc.toc()),
        "Guide\n  Install now\n    From source\n  Use\n"
    );
}

#[test]
fn link_list_is_a_nested_markdown_list() {
    let doc = Document::parse(GUIDE);
    assert_eq!(
        toc::link_list(&doc.toc()),
        "- [Guide](#guide)\n  - [Install now](#install-now)\n    - [From source](#from-source)\n  - [Use](#use)\n"
    );
}

#[test]
fn empty_outlines_render_as_nothing() {
    let doc = Document::parse("just text\n");
    assert_eq!(toc::outline(&doc.toc()), "");
    assert_eq!(toc::link_list(&doc.toc()), "");
}

#[test]
fn skipped_levels_nest_one_step_and_shallower_headings_dedent() {
    // `####` under `#` is one level deeper, not three: deeper indentation would make
    // an indented code block.
    let doc = Document::parse("## A\n\n#### B\n\n### C\n\n# D\n\n## E\n");
    assert_eq!(toc::outline(&doc.toc()), "A\n  B\n  C\nD\n  E\n");
    let list = toc::link_list(&doc.toc());
    assert_eq!(
        list,
        "- [A](#a)\n  - [B](#b)\n  - [C](#c)\n- [D](#d)\n  - [E](#e)\n"
    );
    // The list parses back into two top-level items, each with one nested list.
    let back = Document::parse(list);
    let outer = back.ast().children(back.ast().root())[0];
    assert_eq!(back.ast().children(outer).len(), 2);
}

#[test]
fn link_text_is_escaped_and_parses_back_to_the_heading_text() {
    let src = "# a \\[b\\] \\*c\\* \\`d\\` \\<e\\> \\\\ snake_case \\_x\\_\n";
    let doc = Document::parse(src);
    let entries = doc.toc();
    assert_eq!(entries[0].text, "a [b] *c* `d` <e> \\ snake_case _x_");
    let list = toc::link_list(&entries);
    assert_eq!(
        list,
        "- [a \\[b\\] \\*c\\* \\`d\\` \\<e> \\\\ snake_case \\_x\\_](#a-b-c-d-e--snake_case-_x_)\n"
    );
    let back = Document::parse(list);
    let link = back
        .ast()
        .descendants(back.ast().root())
        .find(|&id| back.ast().kind(id).node_type() == mrq_core::ast::NodeType::Link)
        .unwrap();
    assert_eq!(back.ast().text(link), entries[0].text);
}

#[test]
fn link_destinations_are_percent_encoded() {
    // Obsidian anchors are the heading text, spaces and all.
    let doc = DocumentOptions::new()
        .flavor("obsidian")
        .parse("# My (first) note \\<x\\> 100%\n")
        .unwrap();
    assert_eq!(
        toc::link_list(&doc.toc()),
        "- [My (first) note \\<x> 100%](#My%20%28first%29%20note%20%3Cx%3E%20100)\n"
    );
}

#[test]
fn json_has_file_flavor_and_headings_with_positions() {
    let doc = Document::parse("---\ntitle: T\n---\n# Größe\n\n## B\n");
    let json = serde_json::to_value(JsonToc::new(&doc, &doc.toc())).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "file": null,
            "flavor": "gfm",
            "headings": [
                {
                    "level": 1,
                    "text": "Größe",
                    "slug": "größe",
                    "position": {
                        "start": { "line": 4, "column": 1, "offset": 17 },
                        "end": { "line": 4, "column": 8, "offset": 26 },
                    },
                },
                {
                    "level": 2,
                    "text": "B",
                    "slug": "b",
                    "position": {
                        "start": { "line": 6, "column": 1, "offset": 28 },
                        "end": { "line": 6, "column": 5, "offset": 32 },
                    },
                },
            ],
        })
    );
}

#[test]
fn json_file_is_the_document_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    std::fs::write(&path, "# A\n").unwrap();
    let doc = Document::load(&path).unwrap();
    let json = JsonToc::new(&doc, &doc.toc());
    assert_eq!(json.file.as_deref(), Some(path.to_str().unwrap()));
    assert_eq!(json.headings.len(), 1);
}
