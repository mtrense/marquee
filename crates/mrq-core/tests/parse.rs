//! Parser conformance: every `CommonMark` and GFM spec example parses into a tree whose
//! spans are well-formed and point at the right source, plus snapshot fixtures that show
//! `&src[span]` for every node.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use mrq_core::ast::{Ast, Event, LinkForm, NodeId, NodeKind};
use mrq_core::parse::{ComrakParser, ParseOptions, Parser};
use proptest::prelude::*;

const FENCE: &str = "````````````````````````````````";

/// One example of a spec file.
struct Example {
    /// 1-based example number within its file.
    number: usize,
    /// Line of the opening fence, for messages.
    line: usize,
    /// The Markdown input, with the spec's `→` replaced by tabs.
    markdown: String,
}

/// Extracts the examples of a spec file in the `spec.txt` format shared by `CommonMark`
/// and GFM: a fence of 32 backticks plus `example`, Markdown, a `.` line, HTML, a fence.
fn examples(file: &str) -> Vec<Example> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/spec")
        .join(file);
    let text = fs::read_to_string(&path).unwrap();
    let mut examples = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        if !line.starts_with(FENCE) || !line[FENCE.len()..].trim_start().starts_with("example") {
            continue;
        }
        let mut markdown = String::new();
        for (_, line) in lines.by_ref() {
            if line == "." {
                break;
            }
            markdown.push_str(line);
            markdown.push('\n');
        }
        for (_, line) in lines.by_ref() {
            if line.starts_with(FENCE) {
                break;
            }
        }
        examples.push(Example {
            number: examples.len() + 1,
            line: index + 1,
            markdown: markdown.replace('→', "\t"),
        });
    }
    examples
}

/// Every extension the parser knows, for robustness runs.
fn everything() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.highlight = true;
    options.superscript = true;
    options.subscript = true;
    options.description_lists = true;
    options
}

/// Checks the structural span invariants of `ast` and the per-kind expectations on
/// `&src[span]`; returns one message per violation.
fn span_violations(ast: &Ast, src: &str) -> Vec<String> {
    span_violations_with(ast, src, true)
}

/// [`span_violations`]; unless `strict`, only bounds, nesting, order and char boundaries
/// are checked (empty spans and source that doesn't match the kind are allowed).
fn span_violations_with(ast: &Ast, src: &str, strict: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let root = ast.root();
    if ast.span(root) != (0..src.len()).into() {
        errors.push(format!(
            "root span {:?} != 0..{}",
            ast.span(root),
            src.len()
        ));
    }
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
        // Cells the parser pads a short row with have no source: an empty span at the
        // end of the row.
        if strict && id != root && span.is_empty() && !matches!(ast.kind(id), NodeKind::TableCell) {
            errors.push(format!("{what}: empty"));
        }
        if let Some(parent) = ast.parent(id) {
            let outer = ast.span(parent);
            if span.start < outer.start || span.end > outer.end {
                errors.push(format!(
                    "{what}: outside parent {}..{}",
                    outer.start, outer.end
                ));
            }
        }
        if let Some(prev) = ast.prev_sibling(id)
            && ast.span(prev).end > span.start
        {
            errors.push(format!(
                "{what}: overlaps previous sibling {:?}",
                ast.span(prev)
            ));
        }
        if !strict {
            continue;
        }
        if let Some(message) = kind_violation(ast, id, &src[span.start..span.end]) {
            errors.push(format!(
                "{what} {:?}: {message}",
                &src[span.start..span.end]
            ));
        }
    }
    errors
}

/// What is wrong with `slice` as the source of node `id`, if anything.
fn kind_violation(ast: &Ast, id: NodeId, slice: &str) -> Option<&'static str> {
    let ok = match ast.kind(id) {
        NodeKind::Text(literal) => {
            // Escapes and entities make text differ from its source; otherwise it matches.
            slice.contains(['\\', '&']) || literal.contains(['\\', '&']) || slice == literal
        }
        NodeKind::Code(_) => slice.starts_with('`') && slice.ends_with('`'),
        NodeKind::Emphasis => {
            (slice.starts_with('*') && slice.ends_with('*'))
                || (slice.starts_with('_') && slice.ends_with('_'))
        }
        NodeKind::Strong => {
            (slice.starts_with("**") && slice.ends_with("**"))
                || (slice.starts_with("__") && slice.ends_with("__"))
        }
        NodeKind::Strikethrough | NodeKind::Subscript => {
            slice.starts_with('~') && slice.ends_with('~')
        }
        NodeKind::Superscript => slice.starts_with('^') && slice.ends_with('^'),
        NodeKind::Highlight => slice.starts_with("==") && slice.ends_with("=="),
        NodeKind::Link(link) => match &link.form {
            LinkForm::Inline | LinkForm::Reference { .. } => slice.starts_with('['),
            LinkForm::Autolink => slice.starts_with('<') && slice.ends_with('>'),
            LinkForm::Bare => link.url.ends_with(slice) || slice.ends_with(&*link.url),
            _ => false,
        },
        NodeKind::Image(_) => slice.starts_with("!["),
        // Continuation lines lose their indentation in the literal.
        NodeKind::HtmlInline(literal) => {
            slice == literal
                || (slice.contains(['\n', '\r']) && slice.starts_with('<') && slice.ends_with('>'))
        }
        NodeKind::Comment(comment) => slice
            .trim_end()
            .ends_with(&format!("{}-->", comment.literal)),
        NodeKind::FootnoteReference(r) => slice == format!("[^{}]", r.label),
        NodeKind::Math(_) => slice.starts_with('$') && slice.ends_with('$'),
        NodeKind::SoftBreak => slice
            .trim_start_matches([' ', '\t'])
            .trim_end_matches(['\r', '\n'])
            .is_empty(),
        NodeKind::LineBreak => {
            (slice.starts_with("  ") || slice.starts_with('\\')) && slice.ends_with(['\r', '\n'])
        }
        NodeKind::BlockQuote | NodeKind::Callout(_) => slice.starts_with('>'),
        NodeKind::ListItem(_) => {
            slice.starts_with(|c: char| "-+*".contains(c) || c.is_ascii_digit())
        }
        NodeKind::ThematicBreak => slice.chars().all(|c| " \t-*_".contains(c)),
        NodeKind::Heading(h) if !h.setext => slice.trim_start().starts_with('#'),
        NodeKind::Heading(_) => slice.trim_end().ends_with(['=', '-']),
        NodeKind::CodeBlock(code) => code.fence.is_none_or(|f| slice.starts_with(f.char)),
        NodeKind::FootnoteDefinition(d) => slice.starts_with(&format!("[^{}]:", d.label)),
        _ => true,
    };
    (!ok).then_some("source does not match the node")
}

/// Parses every example of `file` with `options` (also with CRLF line endings) and
/// asserts the span invariants.
fn check_spec(file: &str, options: ParseOptions) {
    let parser = ComrakParser::new(options);
    let examples = examples(file);
    assert!(
        examples.len() > 600,
        "{file}: only {} examples",
        examples.len()
    );
    let mut failures = Vec::new();
    for example in &examples {
        for src in [
            example.markdown.clone(),
            example.markdown.replace('\n', "\r\n"),
        ] {
            let ast = parser.parse(&src);
            let errors = span_violations(&ast, &src);
            if !errors.is_empty() {
                failures.push(format!(
                    "{file} example {} (line {}) {src:?}:\n  {}\n{}",
                    example.number,
                    example.line,
                    errors.join("\n  "),
                    ast.dump(ast.root())
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn commonmark_spec_examples_have_valid_spans() {
    check_spec("commonmark-spec.txt", ParseOptions::commonmark());
}

#[test]
fn gfm_spec_examples_have_valid_spans() {
    check_spec("gfm-spec.txt", ParseOptions::gfm());
}

#[test]
fn spec_examples_have_valid_spans_with_every_extension() {
    check_spec("commonmark-spec.txt", everything());
    check_spec("gfm-spec.txt", everything());
}

/// One line per node: indentation, type, span and the source slice it covers.
fn source_dump(ast: &Ast, src: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for event in ast.traverse(ast.root()) {
        match event {
            Event::Enter(id) => {
                let span = ast.span(id);
                let _ = writeln!(
                    out,
                    "{:indent$}{} {}..{} {:?}",
                    "",
                    ast.kind(id).node_type(),
                    span.start,
                    span.end,
                    &src[span.start..span.end],
                    indent = depth * 2
                );
                depth += 1;
            }
            Event::Leave(_) => depth -= 1,
        }
    }
    out
}

/// Snapshots `&src[span]` of every node for each fixture in `tests/fixtures/parse`
/// (parsed with every extension), and checks the invariants on them too.
#[test]
fn fixture_sources() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parse");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    assert_ne!(paths.len(), 0, "no fixtures in {}", dir.display());
    let parser = ComrakParser::new(everything());
    for path in paths {
        let src = fs::read_to_string(&path).unwrap();
        let ast = parser.parse(&src);
        let errors = span_violations(&ast, &src);
        assert!(
            errors.is_empty(),
            "{}:\n{}",
            path.display(),
            errors.join("\n")
        );
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        insta::assert_snapshot!(format!("source_{name}"), source_dump(&ast, &src));
    }
}

/// Markdown-ish text: syntax characters, line endings, tabs and multi-byte characters.
fn markdownish() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        4 => "[a-z]{1,4}",
        2 => Just(" ".to_owned()),
        2 => Just("\n".to_owned()),
        1 => Just("\r\n".to_owned()),
        1 => Just("\r".to_owned()),
        1 => Just("\t".to_owned()),
        1 => Just("é".to_owned()),
        1 => Just("🦀".to_owned()),
        3 => prop::sample::select(vec![
            "#", "> ", "- ", "1. ", "* ", "*", "_", "`", "```", "~~", "~", "==", "^", "$", "|",
            "[", "]", "(", ")", "<", ">", "!", "\\", "&amp;", "[^a]", "[^a]: ", "[a]: /u\n",
            "- [ ] ", "<!-- c -->", "<div>", "---", "===", ": ", "[!NOTE]", "www.x.org", "](", "\"", "'", "    ",
        ])
        .prop_map(str::to_owned),
    ];
    prop::collection::vec(piece, 0..60).prop_map(|pieces| pieces.concat())
}

proptest! {
    #[test]
    fn arbitrary_input_has_valid_spans(src in markdownish()) {
        let ast = ComrakParser::new(everything()).parse(&src);
        // After a tab that is partly consumed as indentation, comrak counts columns with
        // the tab expanded. Inline positions on such lines are corrected where text, code
        // or HTML on the line anchors them; otherwise the spans are still well-formed but
        // may miss the node's delimiters or be empty.
        let errors = span_violations_with(&ast, &src, !src.contains('\t'));
        prop_assert!(errors.is_empty(), "{src:?}:\n{}\n{}", errors.join("\n"), ast.dump(ast.root()));
    }
}
