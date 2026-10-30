//! The mkdocs flavor: AST snapshots of the fixtures in `tests/fixtures/mkdocs` (with
//! `&src[span]` next to every node, checked against the node kind), plus targeted tests
//! for admonitions (nested, in containers), heading attributes, definition lists,
//! abbreviations, math, the slugger and heading anchors.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use mrq_core::ast::{
    AbbreviationDefinition, Admonition, Ast, Fold, HeadingAttrs, Math, NodeId, NodeKind, NodeType,
};
use mrq_core::flavor::{self, Flavor, Mkdocs, MkdocsSlugger, Registry, Slugger, mkdocs_slug};
use mrq_core::parse::ParseOptions;
use proptest::prelude::*;

fn parse(src: &str) -> Ast {
    flavor::parse(&Mkdocs, src, Path::new("page.md")).ast
}

/// `(type, &src[span])` of every node below `id`, `id` included, in document order.
fn sources_below<'a>(ast: &Ast, id: NodeId, src: &'a str) -> Vec<(NodeType, &'a str)> {
    ast.descendants(id)
        .map(|id| (ast.kind(id).node_type(), ast.node(id).source(src)))
        .collect()
}

/// The ids of type `ty`, in document order.
fn ids_of(ast: &Ast, ty: NodeType) -> Vec<NodeId> {
    ast.descendants(ast.root())
        .filter(|&id| ast.kind(id).node_type() == ty)
        .collect()
}

/// The kinds of type `ty`, in document order.
fn kinds_of(ast: &Ast, ty: NodeType) -> Vec<NodeKind> {
    ids_of(ast, ty)
        .into_iter()
        .map(|id| ast.kind(id).clone())
        .collect()
}

/// The types of the root's children.
fn top_level(ast: &Ast) -> Vec<NodeType> {
    ast.children(ast.root())
        .iter()
        .map(|&id| ast.kind(id).node_type())
        .collect()
}

/// Structural span invariants plus per-kind expectations on `&src[span]`; one message
/// per violation.
fn span_violations(ast: &Ast, src: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let root = ast.root();
    for id in ast.descendants(root) {
        let span = ast.span(id);
        let what = format!("{} {}..{}", ast.kind(id).node_type(), span.start, span.end);
        if span.start > span.end || span.end > src.len() {
            errors.push(format!("{what}: out of bounds"));
            continue;
        }
        if !src.is_char_boundary(span.start) || !src.is_char_boundary(span.end) {
            errors.push(format!("{what}: not on char boundaries"));
            continue;
        }
        if id != root && span.is_empty() && !matches!(ast.kind(id), NodeKind::TableCell) {
            errors.push(format!("{what}: empty"));
        }
        if let Some(parent) = ast.parent(id) {
            let outer = ast.span(parent);
            if span.start < outer.start || span.end > outer.end {
                errors.push(format!("{what}: outside parent {outer:?}"));
            }
        }
        if let Some(prev) = ast.prev_sibling(id)
            && ast.span(prev).end > span.start
        {
            errors.push(format!("{what}: overlaps previous sibling"));
        }
        let slice = &src[span.start..span.end];
        if !kind_matches(ast, id, src, slice) {
            errors.push(format!("{what} {slice:?}: source does not match the node"));
        }
    }
    errors
}

/// Whether `slice` is plausible source for node `id`.
fn kind_matches(ast: &Ast, id: NodeId, src: &str, slice: &str) -> bool {
    match ast.kind(id) {
        // Escapes and entities make text differ from its source (and comrak's spans of
        // text with `\|` before a table can miss the pipe, a parser gap).
        NodeKind::Text(literal) => slice == literal || slice.contains(['\\', '&']),
        NodeKind::Admonition(_) => slice.starts_with("!!!") || slice.starts_with("???"),
        NodeKind::AbbreviationDefinition(def) => {
            slice.starts_with(&format!("*[{}]", def.abbr)) && slice.ends_with(&def.expansion)
        }
        NodeKind::Math(_) => {
            (slice.starts_with('$') && slice.ends_with('$'))
                || (slice.starts_with("\\(") && slice.ends_with("\\)"))
                || (slice.starts_with("\\[") && slice.ends_with("\\]"))
        }
        NodeKind::Heading(heading) => heading.attrs.as_ref().is_none_or(|attrs| {
            let outer = ast.span(id);
            let text = &src[attrs.span.start..attrs.span.end];
            outer.start <= attrs.span.start
                && attrs.span.end <= outer.end
                && text.starts_with('{')
                && text.ends_with('}')
                && ast
                    .children(id)
                    .iter()
                    .all(|&child| ast.span(child).end <= attrs.span.start)
        }),
        NodeKind::CodeBlock(code) => match code.fence {
            Some(fence) => slice.starts_with(fence.char),
            None => true,
        },
        NodeKind::Paragraph => !slice.starts_with(char::is_whitespace),
        _ => true,
    }
}

/// [`Ast::dump`] with `&src[span]` appended to every line.
fn dump_with_source(ast: &Ast, src: &str) -> String {
    let mut out = String::new();
    let lines = ast.dump(ast.root());
    for (line, id) in lines.lines().zip(ast.descendants(ast.root())) {
        let _ = writeln!(out, "{line}  @ {:?}", ast.node(id).source(src));
    }
    out
}

/// Snapshots every fixture in `tests/fixtures/mkdocs` and checks its spans.
#[test]
fn fixture_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mkdocs");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 7, "fixtures missing in {}", dir.display());
    for path in paths {
        let src = fs::read_to_string(&path).unwrap();
        let parsed = flavor::parse(&Mkdocs, &src, &path);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let errors = span_violations(&parsed.ast, &src);
        assert!(
            errors.is_empty(),
            "{}:\n{}\n{}",
            path.display(),
            errors.join("\n"),
            parsed.ast.dump(parsed.ast.root())
        );
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        insta::assert_snapshot!(name, dump_with_source(&parsed.ast, &src));
    }
}

#[test]
fn flavor_is_registered_with_its_options() {
    let registry = Registry::builtin();
    assert_eq!(
        registry.names().collect::<Vec<_>>(),
        ["commonmark", "gfm", "obsidian", "mkdocs"]
    );
    assert_eq!(registry.get("mkdocs").unwrap().name(), "mkdocs");
    let mut expected = ParseOptions::gfm();
    expected.alerts = false;
    expected.highlight = true;
    expected.description_lists = true;
    assert_eq!(Mkdocs.parse_options(), expected);
    assert_eq!(
        Mkdocs.passes().iter().map(|p| p.name()).collect::<Vec<_>>(),
        [
            "admonitions",
            "heading_attrs",
            "definition_lists",
            "abbreviations",
            "math"
        ]
    );
}

fn admonition(kind: &str, classes: &[&str], title: Option<&str>, fold: Option<Fold>) -> NodeKind {
    NodeKind::Admonition(Admonition {
        kind: kind.into(),
        classes: classes.iter().map(|&c| c.into()).collect(),
        title: title.map(Into::into),
        fold,
    })
}

#[test]
fn admonition_headers_parse_kind_classes_title_and_fold() {
    let src = "!!! note\n\n!!! warning \"Careful *now*\"\n\n??? tip\n\n???+ info inline end \"T\"\n\n\
               !!! danger \"\"\n\n!!!x  \n\n!!! a \"b \"c\" d\"\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::Admonition),
        [
            admonition("note", &[], None, None),
            admonition("warning", &[], Some("Careful *now*"), None),
            admonition("tip", &[], None, Some(Fold::Closed)),
            admonition("info", &["inline", "end"], Some("T"), Some(Fold::Open)),
            admonition("danger", &[], Some(""), None),
            admonition("x", &[], None, None),
            admonition("a", &[], Some("b \"c\" d"), None),
        ]
    );
    assert_eq!(top_level(&ast), [NodeType::Admonition; 7]);
}

#[test]
fn malformed_headers_stay_paragraphs() {
    let src = "!!!\n\n!!! \"Only a title\"\n\n!!!  two-spaces\n\n!!! a \"open\n\n!!! a! b\n\n\
               ?? a\n\nx !!! a\n";
    let ast = parse(src);
    assert_eq!(kinds_of(&ast, NodeType::Admonition), []);
    assert_eq!(top_level(&ast), [NodeType::Paragraph; 7]);
}

#[test]
fn body_is_reparsed_and_ends_at_the_first_less_indented_line() {
    let src = "!!! note \"T\"\n    one\n    *two*\n\n        code\n\n\n    - item\nafter\n\nnext\n";
    let ast = parse(src);
    assert_eq!(
        top_level(&ast),
        [
            NodeType::Admonition,
            NodeType::Paragraph,
            NodeType::Paragraph
        ]
    );
    let note = ast.children(ast.root())[0];
    assert_eq!(
        sources_below(&ast, note, src),
        [
            (
                NodeType::Admonition,
                "!!! note \"T\"\n    one\n    *two*\n\n        code\n\n\n    - item"
            ),
            (NodeType::Paragraph, "one\n    *two*"),
            (NodeType::Text, "one"),
            (NodeType::SoftBreak, "\n"),
            (NodeType::Emphasis, "*two*"),
            (NodeType::Text, "two"),
            (NodeType::CodeBlock, "    code"),
            (NodeType::List, "- item"),
            (NodeType::ListItem, "- item"),
            (NodeType::Paragraph, "item"),
            (NodeType::Text, "item"),
        ]
    );
    let NodeKind::CodeBlock(code) = ast.kind(ids_of(&ast, NodeType::CodeBlock)[0]) else {
        unreachable!()
    };
    assert_eq!(code.literal, "code\n");
    // The first line with less indent is not part of the body.
    let after = ast.children(ast.root())[1];
    assert_eq!(ast.node(after).source(src), "after");
    assert_eq!(ast.text(ast.root()), "T\none two\ncode\nitem\nafter\nnext");
}

#[test]
fn lazy_lines_after_the_body_are_reparsed() {
    let src = "- !!! a\n      x\n  y\n  !!! b\n\n      z\n";
    let ast = parse(src);
    let item = ids_of(&ast, NodeType::ListItem)[0];
    let blocks: Vec<_> = ast
        .children(item)
        .iter()
        .map(|&id| (ast.kind(id).node_type(), ast.node(id).source(src)))
        .collect();
    assert_eq!(
        blocks,
        [
            (NodeType::Admonition, "!!! a\n      x"),
            (NodeType::Paragraph, "y"),
            (NodeType::Admonition, "!!! b\n\n      z"),
        ]
    );
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

#[test]
fn nested_admonitions_nest() {
    let src =
        "!!! a\n    !!! b\n        ??? c\n            deep\n\n        in b\n\n    in a\n\nout\n";
    let ast = parse(src);
    let ids = ids_of(&ast, NodeType::Admonition);
    assert_eq!(ids.len(), 3);
    assert_eq!(ast.parent(ids[0]), Some(ast.root()));
    assert_eq!(ast.parent(ids[1]), Some(ids[0]));
    assert_eq!(ast.parent(ids[2]), Some(ids[1]));
    assert_eq!(ast.node(ids[2]).source(src), "??? c\n            deep");
    assert_eq!(
        ast.node(ids[1]).source(src),
        "!!! b\n        ??? c\n            deep\n\n        in b"
    );
    let texts: Vec<_> = ids_of(&ast, NodeType::Paragraph)
        .into_iter()
        .map(|id| (ast.node(id).source(src), ast.ancestors(id).count()))
        .collect();
    assert_eq!(texts, [("deep", 4), ("in b", 3), ("in a", 2), ("out", 1)]);
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

#[test]
fn admonitions_in_list_items_quotes_and_definitions() {
    let src = "- x\n\n    !!! note\n        in item\n\n> !!! tip\n>     quoted\n>\n>     more\n\n\
               Term\n:   !!! info\n        defined\n";
    let ast = parse(src);
    let ids = ids_of(&ast, NodeType::Admonition);
    let parents: Vec<_> = ids
        .iter()
        .map(|&id| ast.kind(ast.parent(id).unwrap()).node_type())
        .collect();
    assert_eq!(
        parents,
        [
            NodeType::ListItem,
            NodeType::BlockQuote,
            NodeType::DefinitionDetails
        ]
    );
    let bodies: Vec<_> = ids
        .iter()
        .map(|&id| {
            ast.children(id)
                .iter()
                .map(|&child| ast.node(child).source(src))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        bodies,
        [vec!["in item"], vec!["quoted", "more"], vec!["defined"]]
    );
    assert_eq!(
        ast.node(ids[1]).source(src),
        "!!! tip\n>     quoted\n>\n>     more"
    );
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

#[test]
fn tabs_indent_bodies_and_crlf_is_kept() {
    let src = "!!! note\r\n\tTabbed body.\r\n\r\n\t!!! tip\r\n\t\tNested.\r\n";
    let ast = parse(src);
    let ids = ids_of(&ast, NodeType::Admonition);
    assert_eq!(ids.len(), 2);
    assert_eq!(ast.parent(ids[1]), Some(ids[0]));
    assert_eq!(ast.node(ids[1]).source(src), "!!! tip\r\n\t\tNested.");
    assert_eq!(ast.text(ast.root()), "Tabbed body.\nNested.");
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

#[test]
fn flavor_syntax_inside_bodies_is_recognized() {
    let src =
        "!!! note\n    ## Head {#h}\n\n    *[A]: Alpha\n\n    \\(x\\) A\n\n    T\n    :   d\n";
    let ast = parse(src);
    let heading = ids_of(&ast, NodeType::Heading)[0];
    let NodeKind::Heading(h) = ast.kind(heading) else {
        unreachable!()
    };
    assert_eq!(h.attrs.as_ref().unwrap().id.as_deref(), Some("h"));
    assert_eq!(ast.text(heading), "Head");
    assert_eq!(kinds_of(&ast, NodeType::AbbreviationDefinition).len(), 1);
    assert_eq!(kinds_of(&ast, NodeType::Math).len(), 1);
    assert_eq!(kinds_of(&ast, NodeType::DefinitionList).len(), 1);
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

fn attrs(ast: &Ast, heading: NodeId) -> Option<HeadingAttrs> {
    match ast.kind(heading) {
        NodeKind::Heading(h) => h.attrs.clone(),
        _ => None,
    }
}

#[test]
fn heading_attributes_are_parsed_and_cut_from_the_text() {
    let src = "# A *b* {: #id .c1 .c2 k=v q=\"x y\" s='z' bare }\n\nT {.c}\n=\n\n## n{#x}\n\n## e {}\n\n## \\{#no}\n";
    let ast = parse(src);
    let headings = ids_of(&ast, NodeType::Heading);
    let first = attrs(&ast, headings[0]).unwrap();
    assert_eq!(first.id.as_deref(), Some("id"));
    assert_eq!(first.classes, ["c1", "c2"]);
    assert_eq!(
        first.attributes,
        [
            ("k".into(), "v".into()),
            ("q".into(), "x y".into()),
            ("s".into(), "z".into()),
            ("bare".into(), "bare".into())
        ]
    );
    assert_eq!(
        &src[first.span.start..first.span.end],
        "{: #id .c1 .c2 k=v q=\"x y\" s='z' bare }"
    );
    assert_eq!(ast.text(headings[0]), "A b");
    // The text node after the emphasis was only ` {…}`: it is gone.
    assert_eq!(
        sources_below(&ast, headings[0], src).last(),
        Some(&(NodeType::Text, "b"))
    );
    let setext = attrs(&ast, headings[1]).unwrap();
    assert_eq!((setext.id, setext.classes), (None, vec!["c".to_owned()]));
    assert_eq!(ast.text(headings[1]), "T");
    for &id in &headings[2..] {
        assert_eq!(attrs(&ast, id), None, "{}", ast.node(id).source(src));
    }
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

#[test]
fn definition_lists_group_details_under_their_term() {
    let src = "T\n:   a\n:   b\n\nU\n:   c\n";
    let ast = parse(src);
    let items = ids_of(&ast, NodeType::DefinitionItem);
    let shapes: Vec<Vec<NodeType>> = items
        .iter()
        .map(|&item| {
            ast.children(item)
                .iter()
                .map(|&c| ast.kind(c).node_type())
                .collect()
        })
        .collect();
    assert_eq!(
        shapes,
        [
            vec![
                NodeType::DefinitionTerm,
                NodeType::DefinitionDetails,
                NodeType::DefinitionDetails
            ],
            vec![NodeType::DefinitionTerm, NodeType::DefinitionDetails],
        ]
    );
    assert_eq!(ast.node(items[0]).source(src), "T\n:   a\n:   b");
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

fn abbr(abbr: &str, expansion: &str) -> NodeKind {
    NodeKind::AbbreviationDefinition(AbbreviationDefinition {
        abbr: abbr.into(),
        expansion: expansion.into(),
    })
}

#[test]
fn abbreviation_definitions_leave_the_paragraph() {
    let src = "x HTML\n*[HTML]: Hyper *Text*\n*[A B] :  b \ny\n\n*[C]:\n\n*[D] x\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::AbbreviationDefinition),
        [
            abbr("HTML", "Hyper *Text*"),
            abbr("A B", "b"),
            abbr("C", "")
        ]
    );
    let blocks: Vec<_> = ast
        .children(ast.root())
        .iter()
        .map(|&id| (ast.kind(id).node_type(), ast.node(id).source(src)))
        .collect();
    assert_eq!(
        blocks,
        [
            (NodeType::Paragraph, "x HTML"),
            (NodeType::AbbreviationDefinition, "*[HTML]: Hyper *Text*"),
            (NodeType::AbbreviationDefinition, "*[A B] :  b"),
            (NodeType::Paragraph, "y"),
            (NodeType::AbbreviationDefinition, "*[C]:"),
            (NodeType::Paragraph, "*[D] x"),
        ]
    );
    assert_eq!(ast.text(ast.root()), "x HTML\ny\n*[D] x");
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

fn math(display: bool, literal: &str) -> NodeKind {
    NodeKind::Math(Math {
        display,
        literal: literal.into(),
    })
}

#[test]
fn math_dollars_and_backslash_delimiters() {
    let src = "$a$ \\(b *c*\\) \\[d\\] \\\\(e\\\\) `\\(f\\)` \\(open\n\n\\[\ng\n\\]\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::Math),
        [
            math(false, "a"),
            math(false, "b *c*"),
            math(true, "d"),
            math(true, "\ng\n")
        ]
    );
    let first = ast.children(ast.root())[0];
    assert_eq!(
        ast.text(first),
        "a b *c* d \\(e\\) \\(f\\) (open",
        "{}",
        ast.dump(first)
    );
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

#[test]
fn slugs_follow_python_markdown() {
    assert_eq!(mkdocs_slug("Hello, World!"), "hello-world");
    assert_eq!(mkdocs_slug("  Ünïcödé  Straße "), "unicode-strae");
    assert_eq!(mkdocs_slug("a - b -- c_d"), "a-b-c_d");
    assert_eq!(mkdocs_slug("C++ & C#"), "c-c");
    assert_eq!(mkdocs_slug("ﬁ ½"), "fi-12");
    assert_eq!(mkdocs_slug("日本語"), "");

    let mut slugger = MkdocsSlugger::new();
    let slugs: Vec<_> = ["A", "a", "a_1", "", "日本", "a"]
        .iter()
        .map(|t| slugger.slug(t))
        .collect();
    assert_eq!(slugs, ["a", "a_1", "a_2", "_1", "_2", "a_3"]);
    slugger.reset();
    assert_eq!(slugger.slug("a"), "a");
    assert_eq!(Mkdocs.slugger().slug("x y"), "x-y");
}

#[test]
fn heading_anchors_honour_explicit_ids() {
    let src = "# Intro\n\n## Setup {#intro}\n\n## Intro\n\n## Usage {.c}\n";
    let ast = parse(src);
    let anchors: Vec<_> = flavor::heading_anchors(&ast, &mut *Mkdocs.slugger())
        .into_iter()
        .map(|(id, anchor)| (ast.text(id), anchor))
        .collect();
    assert_eq!(
        anchors,
        [
            ("Intro".to_owned(), "intro_1".to_owned()),
            ("Setup".to_owned(), "intro".to_owned()),
            ("Intro".to_owned(), "intro_2".to_owned()),
            ("Usage".to_owned(), "usage".to_owned()),
        ]
    );
}

/// mkdocs-ish text: flavor syntax pieces mixed with Markdown, indentation and line endings.
fn mkdocsish() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        4 => "[a-z]{1,4}",
        2 => Just(" ".to_owned()),
        3 => Just("\n".to_owned()),
        1 => Just("\r\n".to_owned()),
        3 => Just("    ".to_owned()),
        1 => Just("\t".to_owned()),
        1 => Just("é".to_owned()),
        3 => prop::sample::select(vec![
            "!!! note\n", "??? tip \"T\"\n", "???+ x", "!!! a b", "\"", "> ", "- ", "1. ",
            ":   ", "*[", "]:", "{#i}", " {.c}", "{", "}", "# ", "\\(", "\\)", "\\[", "\\]",
            "$", "*", "`", "\\", "&amp;", "```\n", "=\n",
        ])
        .prop_map(str::to_owned),
    ];
    prop::collection::vec(piece, 0..60).prop_map(|pieces| pieces.concat())
}

proptest! {
    #[test]
    fn arbitrary_input_keeps_valid_spans(src in mkdocsish()) {
        let ast = parse(&src);
        let errors = span_violations(&ast, &src);
        prop_assert!(errors.is_empty(), "{src:?}:\n{}\n{}", errors.join("\n"), ast.dump(ast.root()));
    }
}
