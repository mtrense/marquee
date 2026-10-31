//! The json backend: the versioned JSON projection of the AST, pretty-printed.

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_render::json;

#[test]
fn the_output_is_the_pretty_projection() {
    let doc = Document::parse("# Title\n\nSome *text*.\n");
    let out = json::render(&doc);
    assert_eq!(out, doc.to_json_ast().to_json_string());
    assert!(out.ends_with("}\n"));
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["mrq_ast"], 1);
    assert_eq!(value["flavor"], "gfm");
    assert_eq!(value["type"], "root");
}

#[test]
fn flavor_nodes_are_projected() {
    let doc = DocumentOptions::new()
        .flavor("obsidian")
        .parse("[[Note#Part|alias]] #tag\n")
        .unwrap();
    insta::assert_snapshot!(json::render(&doc));
}
