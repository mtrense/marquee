//! Directive parsing: `<!-- mrq:include … -->`, `<!-- mrq:toc … -->` and
//! `<!-- mrq:end -->` comments become directive nodes, their attributes are parsed, and
//! start and end markers are paired into [`DirectiveRegion`]s. Problems are diagnostics
//! with spans.

use mrq_core::Document;
use mrq_core::ast::json::JsonKind;
use mrq_core::ast::{Comment, CommentSyntax, Directive, NodeId, NodeKind, NodeType, Span};
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::directive::{DIRECTIVE_ERROR_CODE, DirectiveKind, DirectiveRegion};
use mrq_core::document::DocumentOptions;

fn parse_as(flavor: &str, src: &str) -> Document {
    DocumentOptions::new().flavor(flavor).parse(src).unwrap()
}

/// The nodes of type `ty` in document order.
fn nodes(doc: &Document, ty: NodeType) -> Vec<NodeId> {
    let ast = doc.ast();
    ast.descendants(ast.root())
        .filter(|&id| ast.kind(id).node_type() == ty)
        .collect()
}

fn source(doc: &Document, span: Span) -> &str {
    &doc.text()[span.start..span.end]
}

fn directive(doc: &Document, id: NodeId) -> &Directive {
    match doc.ast().kind(id) {
        NodeKind::IncludeDirective(d) | NodeKind::TocDirective(d) => d,
        other => panic!("not a directive: {other:?}"),
    }
}

/// `(name, value, &src[span])` of every attribute.
fn attributes(doc: &Document, id: NodeId) -> Vec<(&str, &str, &str)> {
    directive(doc, id)
        .attributes
        .iter()
        .map(|a| (a.name.as_str(), a.value.as_str(), source(doc, a.span)))
        .collect()
}

/// The single diagnostic, which must have the directive code.
fn only_diagnostic(doc: &Document) -> &Diagnostic {
    assert_eq!(doc.diagnostics().len(), 1, "{:#?}", doc.diagnostics());
    let diagnostic = &doc.diagnostics()[0];
    assert_eq!(diagnostic.code, DIRECTIVE_ERROR_CODE);
    diagnostic
}

// --- Recognition and attributes ------------------------------------------------------

#[test]
fn include_and_toc_regions_pair_with_their_end_markers() {
    let src = "# T\n\n<!-- mrq:include path=\"a.md\" shift=1 -->\nold\n<!-- mrq:end -->\n\n\
               <!-- mrq:toc depth=2 -->\n- x\n<!-- mrq:end -->\n";
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics(), []);
    let includes = nodes(&doc, NodeType::IncludeDirective);
    let tocs = nodes(&doc, NodeType::TocDirective);
    let ends = nodes(&doc, NodeType::DirectiveEnd);
    assert_eq!((includes.len(), tocs.len(), ends.len()), (1, 1, 2));
    assert_eq!(
        doc.ast().node(includes[0]).source(src),
        "<!-- mrq:include path=\"a.md\" shift=1 -->"
    );
    assert_eq!(doc.ast().node(ends[0]).source(src), "<!-- mrq:end -->");

    let regions = doc.directives();
    assert_eq!(regions.len(), 2);
    let include: &DirectiveRegion = &regions[0];
    assert_eq!(include.kind, DirectiveKind::Include);
    assert_eq!((include.start, include.end), (includes[0], Some(ends[0])));
    assert!(!include.is_bare());
    assert_eq!(source(&doc, include.content), "\nold\n");
    assert_eq!(
        source(&doc, include.span(doc.ast())),
        "<!-- mrq:include path=\"a.md\" shift=1 -->\nold\n<!-- mrq:end -->"
    );
    assert_eq!(include.directive(doc.ast()).get("path"), Some("a.md"));
    let content = include.content_nodes(doc.ast());
    assert_eq!(content.len(), 1);
    assert_eq!(doc.ast().node(content[0]).source(src), "old");

    let toc = &regions[1];
    assert_eq!(toc.kind, DirectiveKind::Toc);
    assert_eq!((toc.start, toc.end), (tocs[0], Some(ends[1])));
    assert_eq!(source(&doc, toc.content), "\n- x\n");
    assert_eq!(toc.directive(doc.ast()).get("depth"), Some("2"));
}

#[test]
fn attribute_values_may_be_quoted_unquoted_escaped_or_bare() {
    let src = r##"<!-- mrq:include path="a b.md" lang='rust' shift=-1 select="# A | ```sh" title="say \"hi\"" q='it\'s' bs="a\\b\c" empty="" dedent -->"##;
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics(), []);
    let include = nodes(&doc, NodeType::IncludeDirective)[0];
    assert_eq!(
        attributes(&doc, include),
        [
            ("path", "a b.md", r#"path="a b.md""#),
            ("lang", "rust", "lang='rust'"),
            ("shift", "-1", "shift=-1"),
            ("select", "# A | ```sh", r##"select="# A | ```sh""##),
            ("title", r#"say "hi""#, r#"title="say \"hi\"""#),
            ("q", "it's", r"q='it\'s'"),
            ("bs", r"a\b\c", r#"bs="a\\b\c""#),
            ("empty", "", r#"empty="""#),
            ("dedent", "true", "dedent"),
        ]
    );
}

#[test]
fn whitespace_and_unknown_attributes_are_kept_as_written() {
    let src = "<!--mrq:toc   frobnicate=yes\n  depth=3-->\n<!--mrq:end-->\n";
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics(), []);
    let toc = nodes(&doc, NodeType::TocDirective)[0];
    assert_eq!(
        attributes(&doc, toc),
        [
            ("frobnicate", "yes", "frobnicate=yes"),
            ("depth", "3", "depth=3")
        ]
    );
    assert_eq!(doc.directives().len(), 1);
}

#[test]
fn directives_are_recognized_in_every_flavor_and_container() {
    let src = "> <!-- mrq:include\n> path=x.md -->\n> <!-- mrq:end -->\n\n\
               - <!-- mrq:toc -->\n  <!-- mrq:end -->\n";
    for flavor in ["commonmark", "gfm", "obsidian", "mkdocs"] {
        let doc = parse_as(flavor, src);
        assert_eq!(doc.diagnostics(), [], "{flavor}");
        let include = nodes(&doc, NodeType::IncludeDirective)[0];
        assert_eq!(attributes(&doc, include), [("path", "x.md", "path=x.md")]);
        assert_eq!(doc.directives().len(), 2, "{flavor}");
        assert!(doc.directives().iter().all(|r| !r.is_bare()), "{flavor}");
    }
}

#[test]
fn attribute_spans_survive_crlf_line_endings() {
    let src = "> <!-- mrq:include\r\n>   path=x.md\r\n> shift=1 -->\r\n> <!-- mrq:end -->\r\n";
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics(), []);
    let include = nodes(&doc, NodeType::IncludeDirective)[0];
    assert_eq!(
        attributes(&doc, include),
        [("path", "x.md", "path=x.md"), ("shift", "1", "shift=1")]
    );
}

#[test]
fn directives_in_mkdocs_admonition_bodies_have_source_spans() {
    let src = "!!! note\n    <!-- mrq:include path=\"in.md\"\n        shift=2 -->\n    \
               <!-- mrq:end -->\n";
    let doc = parse_as("mkdocs", src);
    assert_eq!(doc.diagnostics(), []);
    let include = nodes(&doc, NodeType::IncludeDirective)[0];
    assert_eq!(
        attributes(&doc, include),
        [
            ("path", "in.md", "path=\"in.md\""),
            ("shift", "2", "shift=2")
        ]
    );
    assert_eq!(doc.directives().len(), 1);
}

#[test]
fn code_blocks_and_other_comments_are_left_alone() {
    let src = "```\n<!-- mrq:end -->\n```\n\n    <!-- mrq:end -->\n\n<!-- note -->\n\n\
               <!-- mrq:end --> <b>\n";
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics(), []);
    assert_eq!(nodes(&doc, NodeType::DirectiveEnd), []);
    assert_eq!(nodes(&doc, NodeType::Comment).len(), 1);
    assert_eq!(doc.directives(), []);
}

#[test]
fn inline_directive_comments_are_a_warning_and_stay_comments() {
    let src = "Text <!-- mrq:toc --> more.\n";
    let doc = Document::parse(src);
    let diagnostic = only_diagnostic(&doc);
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(source(&doc, diagnostic.range), "<!-- mrq:toc -->");
    let comment = nodes(&doc, NodeType::Comment)[0];
    assert_eq!(
        doc.ast().kind(comment),
        &NodeKind::Comment(Comment {
            syntax: CommentSyntax::Html,
            literal: " mrq:toc ".into(),
        })
    );
    assert_eq!(doc.directives(), []);
}

// --- Malformed directives -----------------------------------------------------------

#[test]
fn an_unknown_directive_name_is_an_error_and_stays_a_comment() {
    let src = "<!-- mrq:incldue path=a.md -->\n";
    let doc = Document::parse(src);
    let diagnostic = only_diagnostic(&doc);
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(source(&doc, diagnostic.range), "mrq:incldue");
    assert!(
        diagnostic.message.contains("incldue"),
        "{}",
        diagnostic.message
    );
    assert_eq!(nodes(&doc, NodeType::Comment).len(), 1);
    assert_eq!(doc.directives(), []);
}

#[test]
fn malformed_attributes_are_errors_with_spans() {
    let cases = [
        (
            "<!-- mrq:include path=\"a.md -->",
            "\"a.md ",
            "unterminated",
        ),
        ("<!-- mrq:include path='a -->", "'a ", "unterminated"),
        ("<!-- mrq:include path= -->", "path=", "value"),
        ("<!-- mrq:include =a -->", "=", "name"),
        ("<!-- mrq:include p@th=a -->", "@", "unexpected"),
        ("<!-- mrq:include path=\"a\"x -->", "x", "after"),
        ("<!-- mrq: -->", "mrq:", "name"),
    ];
    for (src, at, message) in cases {
        let doc = Document::parse(src);
        let diagnostic = only_diagnostic(&doc);
        assert_eq!(diagnostic.severity, Severity::Error, "{src}");
        assert_eq!(source(&doc, diagnostic.range), at, "{src}");
        assert!(
            diagnostic.message.contains(message),
            "{src}: {}",
            diagnostic.message
        );
        assert_eq!(nodes(&doc, NodeType::Comment).len(), 1, "{src}");
    }
}

#[test]
fn a_duplicate_attribute_is_an_error_but_the_first_one_wins() {
    let src = "<!-- mrq:include path=a.md path=b.md -->\n<!-- mrq:end -->\n";
    let doc = Document::parse(src);
    let diagnostic = only_diagnostic(&doc);
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(source(&doc, diagnostic.range), "path=b.md");
    assert_eq!(source(&doc, diagnostic.labels[0].range), "path=a.md");
    let region = &doc.directives()[0];
    assert_eq!(region.directive(doc.ast()).get("path"), Some("a.md"));
    assert_eq!(region.directive(doc.ast()).attributes.len(), 2);
}

#[test]
fn attributes_on_an_end_marker_are_a_warning() {
    let src = "<!-- mrq:toc -->\n<!-- mrq:end depth=2 -->\n";
    let doc = Document::parse(src);
    let diagnostic = only_diagnostic(&doc);
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(source(&doc, diagnostic.range), "depth=2");
    assert_eq!(nodes(&doc, NodeType::DirectiveEnd).len(), 1);
    assert_eq!(doc.directives().len(), 1);
}

// --- Pairing ------------------------------------------------------------------------

#[test]
fn a_start_marker_without_an_end_is_a_bare_region() {
    let src = "<!-- mrq:include path=a.md -->\n\nText.\n";
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics(), []);
    let region = &doc.directives()[0];
    assert!(region.is_bare());
    assert_eq!(region.end, None);
    let start = doc.ast().span(region.start);
    assert_eq!(region.content, Span::new(start.end, start.end));
    assert_eq!(region.span(doc.ast()), start);
    assert_eq!(region.content_nodes(doc.ast()), []);
}

#[test]
fn an_end_marker_closes_the_nearest_start_and_earlier_ones_stay_bare() {
    let src = "<!-- mrq:include path=a.md -->\n\n<!-- mrq:toc -->\n\n<!-- mrq:include path=b.md -->\nB\n<!-- mrq:end -->\n";
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics(), []);
    let starts: Vec<(DirectiveKind, bool)> = doc
        .directives()
        .iter()
        .map(|r| (r.kind, r.is_bare()))
        .collect();
    assert_eq!(
        starts,
        [
            (DirectiveKind::Include, true),
            (DirectiveKind::Toc, true),
            (DirectiveKind::Include, false),
        ]
    );
    assert_eq!(source(&doc, doc.directives()[2].content), "\nB\n");
}

#[test]
fn an_unmatched_end_marker_is_an_error() {
    let src = "Text.\n\n<!-- mrq:end -->\n\n<!-- mrq:toc -->\n<!-- mrq:end -->\n<!-- mrq:end -->\n";
    let doc = Document::parse(src);
    assert_eq!(doc.diagnostics().len(), 2, "{:#?}", doc.diagnostics());
    let ends = nodes(&doc, NodeType::DirectiveEnd);
    for (diagnostic, end) in doc.diagnostics().iter().zip([ends[0], ends[2]]) {
        assert_eq!(diagnostic.code, DIRECTIVE_ERROR_CODE);
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.range, doc.ast().span(end));
        assert!(
            diagnostic.message.contains("mrq:end"),
            "{}",
            diagnostic.message
        );
    }
    assert_eq!(doc.directives().len(), 1);
}

#[test]
fn a_second_end_after_a_superseded_sibling_start_is_unmatched_not_nested() {
    let src =
        "<!-- mrq:toc -->\n\n<!-- mrq:include path=a.md -->\n<!-- mrq:end -->\n<!-- mrq:end -->\n";
    let doc = Document::parse(src);
    let diagnostic = only_diagnostic(&doc);
    assert_eq!(
        diagnostic.range,
        doc.ast().span(nodes(&doc, NodeType::DirectiveEnd)[1])
    );
    assert!(
        diagnostic.message.contains("without"),
        "{}",
        diagnostic.message
    );
    assert_eq!(diagnostic.labels, []);
    let bare: Vec<bool> = doc
        .directives()
        .iter()
        .map(DirectiveRegion::is_bare)
        .collect();
    assert_eq!(bare, [true, false]);
}

#[test]
fn an_end_marker_nested_deeper_than_its_start_is_an_error() {
    let src = "<!-- mrq:include path=a.md -->\n\n- item\n\n  <!-- mrq:end -->\n";
    let doc = Document::parse(src);
    let diagnostic = only_diagnostic(&doc);
    assert_eq!(diagnostic.severity, Severity::Error);
    let end = nodes(&doc, NodeType::DirectiveEnd)[0];
    let start = nodes(&doc, NodeType::IncludeDirective)[0];
    assert_eq!(diagnostic.range, doc.ast().span(end));
    assert_eq!(diagnostic.labels[0].range, doc.ast().span(start));
    assert!(
        diagnostic.message.contains("nested"),
        "{}",
        diagnostic.message
    );
    // The broken start is no region: tools must not refresh or extend it.
    assert_eq!(doc.directives(), []);
}

#[test]
fn an_end_marker_outside_its_start_container_is_an_error() {
    let src = "> <!-- mrq:toc -->\n> - a\n\n<!-- mrq:end -->\n";
    let doc = Document::parse(src);
    let diagnostic = only_diagnostic(&doc);
    let start = nodes(&doc, NodeType::TocDirective)[0];
    assert_eq!(source(&doc, diagnostic.range), "<!-- mrq:end -->");
    assert_eq!(diagnostic.labels[0].range, doc.ast().span(start));
    assert_eq!(doc.directives(), []);
}

// --- JSON projection ----------------------------------------------------------------

#[test]
fn the_json_projection_shows_directive_nodes() {
    let doc = Document::parse("<!-- mrq:include path=a.md -->\n<!-- mrq:end -->\n");
    let json = doc.to_json_ast();
    let children = json.root.children.as_ref().unwrap();
    let JsonKind::IncludeDirective(include) = &children[0].kind else {
        panic!("{:?}", children[0].kind);
    };
    assert_eq!(include.attributes[0].name, "path");
    assert_eq!(include.attributes[0].value, "a.md");
    assert_eq!(include.attributes[0].position.start.column, 18);
    assert_eq!(children[1].kind, JsonKind::DirectiveEnd);
}
