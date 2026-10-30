//! The obsidian flavor: AST snapshots of the fixtures in `tests/fixtures/obsidian` (with
//! `&src[span]` next to every node, checked against the node kind), plus targeted tests
//! for wikilinks, embeds, tags, callouts, `%%` comments, math and the slugger.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use mrq_core::ast::{
    Ast, Callout, Comment, CommentSyntax, Fold, NodeId, NodeKind, NodeType, Tag, WikiLink,
};
use mrq_core::flavor::{self, Flavor, Obsidian, ObsidianSlugger, Registry, Slugger};
use mrq_core::parse::ParseOptions;
use proptest::prelude::*;

fn parse(src: &str) -> Ast {
    flavor::parse(&Obsidian, src, Path::new("note.md")).ast
}

/// `(type, &src[span])` of every node below the root, in document order.
fn sources<'a>(ast: &Ast, src: &'a str) -> Vec<(NodeType, &'a str)> {
    ast.descendants(ast.root())
        .skip(1)
        .map(|id| (ast.kind(id).node_type(), ast.node(id).source(src)))
        .collect()
}

/// The kinds of type `ty`, in document order.
fn kinds_of(ast: &Ast, ty: NodeType) -> Vec<NodeKind> {
    ast.descendants(ast.root())
        .filter(|&id| ast.kind(id).node_type() == ty)
        .map(|id| ast.kind(id).clone())
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
        if !kind_matches(ast, id, slice) {
            errors.push(format!("{what} {slice:?}: source does not match the node"));
        }
    }
    errors
}

/// Whether `slice` is plausible source for node `id`.
fn kind_matches(ast: &Ast, id: NodeId, slice: &str) -> bool {
    match ast.kind(id) {
        // Escapes and entities make text differ from its source (and comrak's spans of
        // text with `\|` before a table can miss the pipe, a parser gap); the literals of
        // split text are checked in `tags_split_text_with_escapes_into_correct_literals`.
        NodeKind::Text(literal) => slice == literal || slice.contains(['\\', '&']),
        NodeKind::WikiLink(_) => slice.starts_with("[[") && slice.ends_with("]]"),
        NodeKind::Embed(_) => slice.starts_with("![[") && slice.ends_with("]]"),
        NodeKind::Tag(tag) => slice == format!("#{}", tag.name),
        NodeKind::Comment(Comment {
            syntax: CommentSyntax::Percent,
            literal,
        }) => slice == format!("%%{literal}%%"),
        NodeKind::Comment(comment) => slice
            .trim_end()
            .ends_with(&format!("{}-->", comment.literal)),
        NodeKind::Callout(_) | NodeKind::BlockQuote => slice.starts_with('>'),
        NodeKind::Math(_) => slice.starts_with('$') && slice.ends_with('$'),
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

/// Snapshots every fixture in `tests/fixtures/obsidian` and checks its spans.
#[test]
fn fixture_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/obsidian");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 5, "fixtures missing in {}", dir.display());
    for path in paths {
        let src = fs::read_to_string(&path).unwrap();
        let parsed = flavor::parse(&Obsidian, &src, &path);
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
    assert_eq!(registry.get("obsidian").unwrap().name(), "obsidian");
    let options = Obsidian.parse_options();
    let mut expected = ParseOptions::gfm();
    expected.alerts = false;
    expected.highlight = true;
    assert_eq!(options, expected);
    assert!(options.math);
    assert_eq!(
        Obsidian
            .passes()
            .iter()
            .map(|p| p.name())
            .collect::<Vec<_>>(),
        ["callouts", "block_comments", "inlines"]
    );
}

fn wikilink(target: &str, anchor: Option<&str>, alias: Option<&str>) -> WikiLink {
    WikiLink {
        target: target.into(),
        anchor: anchor.map(Into::into),
        alias: alias.map(Into::into),
    }
}

#[test]
fn wikilinks_split_target_anchor_and_alias() {
    let src = "[[a]] [[a#h|x]] [[#h]] [[a#^b]] [[ a | b ]] [[a#]] [[a\\|t]]\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::WikiLink),
        [
            wikilink("a", None, None),
            wikilink("a", Some("h"), Some("x")),
            wikilink("", Some("h"), None),
            wikilink("a", Some("^b"), None),
            wikilink("a", None, Some("b")),
            wikilink("a", None, None),
            wikilink("a", None, Some("t")),
        ]
        .map(NodeKind::WikiLink)
    );
    let para = sources(&ast, src);
    assert_eq!(para[1], (NodeType::WikiLink, "[[a]]"));
    assert_eq!(para[2], (NodeType::Text, " "));
    assert_eq!(para.last(), Some(&(NodeType::WikiLink, "[[a\\|t]]")));
}

#[test]
fn embeds_carry_sizes_and_anchors() {
    let src = "![[i.png|100x200]] ![[n#h]] ![[n#^q]] !\\[[no]]\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::Embed),
        [
            wikilink("i.png", None, Some("100x200")),
            wikilink("n", Some("h"), None),
            wikilink("n", Some("^q"), None),
        ]
        .map(NodeKind::Embed)
    );
    assert_eq!(
        sources(&ast, src)[1],
        (NodeType::Embed, "![[i.png|100x200]]")
    );
    assert_eq!(kinds_of(&ast, NodeType::WikiLink), []);
}

#[test]
fn wikilinks_do_not_cross_lines_or_enter_code_and_links() {
    let src = "[[a\nb]] `[[c]]` [x](u \"[[d]]\") [[e]f]] [[]]\n\n    [[f]]\n";
    let ast = parse(src);
    assert!(
        kinds_of(&ast, NodeType::WikiLink).is_empty(),
        "{}",
        ast.dump(ast.root())
    );
}

fn tag(name: &str) -> NodeKind {
    NodeKind::Tag(Tag { name: name.into() })
}

#[test]
fn tags_follow_obsidian_rules() {
    let src = "#a b #b/c #1 #1x x#y \\#z #d. (#e) #f,#g #ü &#35;h ## #\n\n# T #i\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::Tag),
        [
            tag("a"),
            tag("b/c"),
            tag("1x"),
            tag("d"),
            tag("f"),
            tag("ü"),
            tag("i")
        ]
    );
    // The heading's own `#` marker is not a tag.
    let heading = ast.children(ast.root())[1];
    assert_eq!(
        sources(&ast, src)
            .iter()
            .filter(|(ty, _)| *ty == NodeType::Tag)
            .map(|(_, s)| *s)
            .collect::<Vec<_>>(),
        ["#a", "#b/c", "#1x", "#d", "#f", "#ü", "#i"]
    );
    assert_eq!(ast.node(heading).source(src), "# T #i");
}

#[test]
fn tags_split_text_with_escapes_into_correct_literals() {
    let src = "a\\*b #t c\\_d &amp; #u e\n";
    let ast = parse(src);
    let texts: Vec<_> = kinds_of(&ast, NodeType::Text);
    assert_eq!(
        texts,
        ["a*b ", " c_d & ", " e"].map(|t| NodeKind::Text(t.into()))
    );
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

fn callout(kind: &str, title: Option<&str>, fold: Option<Fold>) -> NodeKind {
    NodeKind::Callout(Callout {
        kind: kind.into(),
        title: title.map(Into::into),
        fold,
    })
}

#[test]
fn callouts_parse_kind_fold_and_title() {
    let src = "> [!tip]- Closed *T*\n> body\n\n> [!faq]+\n> x\n\n> [!NOTE]\n> y\n\n> [!x] Only\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::Callout),
        [
            callout("tip", Some("Closed *T*"), Some(Fold::Closed)),
            callout("faq", None, Some(Fold::Open)),
            callout("NOTE", None, None),
            callout("x", Some("Only"), None),
        ]
    );
    let first = ast.children(ast.root())[0];
    assert_eq!(
        sources(&ast, src).into_iter().take(3).collect::<Vec<_>>(),
        [
            (NodeType::Callout, "> [!tip]- Closed *T*\n> body"),
            (NodeType::Paragraph, "body"),
            (NodeType::Text, "body"),
        ]
    );
    assert_eq!(ast.children(first).len(), 1);
    // A marker-only callout has no body.
    let last = *ast.children(ast.root()).last().unwrap();
    assert_eq!(ast.children(last), []);
}

#[test]
fn nested_callouts_and_plain_quotes() {
    let src = "> [!a]\n> > [!b] T\n> > in\n\n> no [!c]\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::Callout),
        [callout("a", None, None), callout("b", Some("T"), None)]
    );
    assert_eq!(kinds_of(&ast, NodeType::BlockQuote).len(), 1);
}

fn percent(literal: &str) -> NodeKind {
    NodeKind::Comment(Comment {
        syntax: CommentSyntax::Percent,
        literal: literal.into(),
    })
}

#[test]
fn percent_comments_inline_and_as_blocks() {
    let src = "a %%b *c*%% d\n\n%%whole%%\n\n%%\nx\n\ny\n%%\n\nz\n";
    let ast = parse(src);
    assert_eq!(
        kinds_of(&ast, NodeType::Comment),
        [percent("b *c*"), percent("whole"), percent("\nx\n\ny\n")]
    );
    let blocks: Vec<_> = ast
        .children(ast.root())
        .iter()
        .map(|&id| (ast.kind(id).node_type(), ast.node(id).source(src)))
        .collect();
    assert_eq!(
        blocks,
        [
            (NodeType::Paragraph, "a %%b *c*%% d"),
            (NodeType::Comment, "%%whole%%"),
            (NodeType::Comment, "%%\nx\n\ny\n%%"),
            (NodeType::Paragraph, "z"),
        ]
    );
    assert_eq!(ast.text(ast.root()), "a  d\nz");
}

#[test]
fn unclosed_and_code_percent_signs_stay_text() {
    let src = "a %% b `%%` c\n\n%%\nnever closed\n";
    let ast = parse(src);
    assert_eq!(kinds_of(&ast, NodeType::Comment), []);
}

#[test]
fn math_is_parsed_and_opaque() {
    let src = "$[[a]] #b$ and $$c$$\n";
    let ast = parse(src);
    assert_eq!(kinds_of(&ast, NodeType::Math).len(), 2);
    assert_eq!(kinds_of(&ast, NodeType::WikiLink), []);
    assert_eq!(kinds_of(&ast, NodeType::Tag), []);
}

#[test]
fn passes_run_inside_fragments_and_frontmatter_offsets() {
    let src = "---\nx: 1\n---\n> [!n] #t [[p]]\n";
    let ast = parse(src);
    assert_eq!(
        sources(&ast, src)
            .into_iter()
            .filter(|(ty, _)| matches!(ty, NodeType::Callout | NodeType::FrontMatter))
            .count(),
        2
    );
    assert_eq!(span_violations(&ast, src), Vec::<String>::new());
}

#[test]
fn slugger_uses_heading_text() {
    let mut slugger = Obsidian.slugger();
    assert_eq!(slugger.slug("My  Heading "), "My Heading");
    assert_eq!(slugger.slug("A: b | c # d ^ e"), "A b c d e");
    assert_eq!(slugger.slug("My Heading"), "My Heading-1");
    assert_eq!(slugger.slug("Über"), "Über");
    slugger.reset();
    assert_eq!(slugger.slug("My Heading"), "My Heading");
    let mut fresh = ObsidianSlugger::new();
    assert_eq!(fresh.slug("[[x]] %%y%%"), "x y");
}

/// Obsidian-ish text: flavor syntax pieces mixed with Markdown and line endings.
fn obsidianish() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        4 => "[a-z]{1,4}",
        2 => Just(" ".to_owned()),
        2 => Just("\n".to_owned()),
        1 => Just("\r\n".to_owned()),
        1 => Just("é".to_owned()),
        3 => prop::sample::select(vec![
            "#", "#t", "[[", "]]", "![[", "|", "\\|", "%%", "> ", "> [!note]", "[!tip]-", "- ",
            "*", "`", "$", "\\", "&amp;", "&#35;", "[", "]", "(", ")", "^", "```\n", "# ",
            // After a blank line: comrak drops the `\|` escapes of a paragraph right
            // before a table, and the parser's spans there miss them (a parser gap).
            "\n\n| a | b |\n| - | - |\n",
        ])
        .prop_map(str::to_owned),
    ];
    prop::collection::vec(piece, 0..50).prop_map(|pieces| pieces.concat())
}

proptest! {
    #[test]
    fn arbitrary_input_keeps_valid_spans(src in obsidianish()) {
        let ast = parse(&src);
        let errors = span_violations(&ast, &src);
        prop_assert!(errors.is_empty(), "{src:?}:\n{}\n{}", errors.join("\n"), ast.dump(ast.root()));
    }
}
