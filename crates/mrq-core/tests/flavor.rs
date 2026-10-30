//! Flavors: the registry, flavor resolution, passes, and GitHub slug compatibility.
//!
//! `fixtures/slug/github-slugger.json` is the test corpus of
//! [github-slugger](https://github.com/Flet/github-slugger) (commit `3461c43`, ISC licence,
//! see `fixtures/slug/LICENSE-github-slugger`), whose expected slugs were checked against
//! anchors rendered by github.com. Its cases run in order through one slugger, so
//! duplicate tracking carries over from case to case.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use mrq_core::ast::{Ast, NodeKind, NodeType};
use mrq_core::config::FlavorName;
use mrq_core::diag::Severity;
use mrq_core::flavor::{
    self, Commonmark, FLAVOR_ERROR_CODE, Flavor, FlavorError, FlavorOrigin, Gfm, GithubSlugger,
    Pass, PassContext, Registry, Slugger, github_slug,
};
use mrq_core::frontmatter::Frontmatter;
use mrq_core::parse::ParseOptions;
use proptest::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct SlugCase {
    name: String,
    input: String,
    expected: String,
}

#[test]
fn github_slugger_corpus() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slug/github-slugger.json");
    let cases: Vec<SlugCase> =
        serde_json::from_str(&std::fs::read_to_string(path).expect("corpus exists"))
            .expect("corpus is JSON");
    assert!(cases.len() > 70);
    let mut slugger = GithubSlugger::new();
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|case| {
            let mut actual = slugger.slug(&case.input);
            if case.name == "Unassigned" {
                // github-slugger's tables are Unicode 13; letters assigned since then
                // are kept here. Everything else must match.
                actual.retain(|c| case.expected.contains(c));
            }
            (actual != case.expected).then(|| {
                format!(
                    "{}: {:?} -> {actual:?}, expected {:?}",
                    case.name, case.input, case.expected
                )
            })
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Markdown headings and the anchor github.com gives them, in document order.
const GITHUB_ANCHORS: &[(&str, &str)] = &[
    ("# Hello World", "hello-world"),
    ("## Installation & Setup", "installation--setup"),
    ("## `mrq toc`", "mrq-toc"),
    ("### What's new?", "whats-new"),
    ("## C++ / C#", "c--c"),
    ("## Step 1: Install", "step-1-install"),
    ("## 🚀 Quick Start", "-quick-start"),
    ("## Über uns", "über-uns"),
    ("## 日本語の見出し", "日本語の見出し"),
    ("## *Emphasis* and **strong**", "emphasis-and-strong"),
    ("## [Link text](https://example.com)", "link-text"),
    ("## foo_bar_baz", "foo_bar_baz"),
    ("## Version 1.2.3", "version-123"),
    ("## Q&A", "qa"),
    ("## Foo  Bar", "foo--bar"),
    ("## ~~Old~~ New", "old-new"),
    ("## Example", "example"),
    ("## Example", "example-1"),
    ("Example\n-------", "example-2"),
    ("## Example 1", "example-1-1"),
    ("## HTTP/2 <br> support", "http2--support"),
    (
        "## Ünïcödé — dashes – and “quotes”",
        "ünïcödé--dashes--and-quotes",
    ),
];

#[test]
fn gfm_heading_anchors_match_github() {
    let headings: Vec<&str> = GITHUB_ANCHORS.iter().map(|(heading, _)| *heading).collect();
    let src = headings.join("\n\n");
    let parsed = flavor::parse(&Gfm, &src, Path::new("anchors.md"));
    let ast = &parsed.ast;
    let mut slugger = Gfm.slugger();
    let slugs: Vec<String> = ast
        .descendants(ast.root())
        .filter(|&id| ast.kind(id).node_type() == NodeType::Heading)
        .map(|id| slugger.slug(&ast.text(id)))
        .collect();
    let expected: Vec<&str> = GITHUB_ANCHORS.iter().map(|(_, slug)| *slug).collect();
    assert_eq!(slugs, expected);
}

#[test]
fn slugger_suffixes_duplicates_and_resets() {
    let mut slugger = GithubSlugger::new();
    assert_eq!(slugger.slug("Intro"), "intro");
    assert_eq!(slugger.slug("Intro"), "intro-1");
    assert_eq!(slugger.slug("intro"), "intro-2");
    assert_eq!(slugger.slug(""), "");
    assert_eq!(slugger.slug("!!"), "-1");
    slugger.reset();
    assert_eq!(slugger.slug("Intro"), "intro");
    assert_eq!(github_slug("Intro"), "intro");
    assert_eq!(
        github_slug("Intro"),
        "intro",
        "the free function keeps no state"
    );
}

proptest! {
    #[test]
    fn slugs_are_unique_within_a_document(
        headings in prop::collection::vec("[aA1 _!-]{0,4}", 0..40),
    ) {
        let mut slugger = GithubSlugger::new();
        let mut seen = HashSet::new();
        for heading in &headings {
            let slug = slugger.slug(heading);
            prop_assert!(seen.insert(slug.clone()), "{slug:?} returned twice");
        }
    }
}

#[test]
fn builtin_flavors() {
    assert_eq!(Gfm.name(), "gfm");
    assert_eq!(Gfm.parse_options(), ParseOptions::gfm());
    assert_eq!(Commonmark.name(), "commonmark");
    assert_eq!(Commonmark.parse_options(), ParseOptions::commonmark());
    assert!(Gfm.passes().is_empty() && Commonmark.passes().is_empty());
    assert!(Gfm.printer_hooks().is_none());
    // CommonMark has no anchor syntax of its own; it uses GitHub slugs as well.
    let mut slugger = Commonmark.slugger();
    assert_eq!(slugger.slug("A b"), "a-b");
    assert_eq!(slugger.slug("A b"), "a-b-1");
}

#[test]
fn parse_applies_flavor_options() {
    let src = "| a |\n| - |\n| b |\n";
    let gfm = flavor::parse(&Gfm, src, Path::new("t.md"));
    let cm = flavor::parse(&Commonmark, src, Path::new("t.md"));
    let first = |ast: &Ast| ast.kind(ast.children(ast.root())[0]).node_type();
    assert_eq!(first(&gfm.ast), NodeType::Table);
    assert_eq!(first(&cm.ast), NodeType::Paragraph);
    assert!(gfm.frontmatter.is_none() && gfm.diagnostics.is_empty());
}

#[test]
fn registry_lists_and_finds_flavors() {
    let registry = Registry::builtin();
    assert_eq!(
        registry.names().collect::<Vec<_>>(),
        ["commonmark", "gfm", "obsidian", "mkdocs"]
    );
    assert_eq!(registry.get("gfm").unwrap().name(), "gfm");
    assert_eq!(Registry::default().names().count(), 4);

    assert!(registry.get("pandoc").is_none());
    let err = registry.resolve(Some("pandoc"), None, None).unwrap_err();
    assert_eq!(
        err.to_string(),
        "unknown flavor `pandoc` (known flavors: commonmark, gfm, obsidian, mkdocs)"
    );
}

#[test]
fn config_flavor_names_resolve() {
    let registry = Registry::builtin();
    for name in [
        FlavorName::Commonmark,
        FlavorName::Gfm,
        FlavorName::Obsidian,
        FlavorName::Mkdocs,
    ] {
        let resolved = registry.resolve(None, None, Some(name.as_str())).unwrap();
        assert_eq!(resolved.flavor.name(), name.as_str());
    }
    assert_eq!(FlavorName::default().as_str(), flavor::DEFAULT_FLAVOR);
}

#[test]
fn registering_replaces_by_name() {
    #[derive(Debug)]
    struct Strict;
    impl Flavor for Strict {
        fn name(&self) -> &'static str {
            "gfm"
        }
        fn parse_options(&self) -> ParseOptions {
            ParseOptions::commonmark()
        }
        fn slugger(&self) -> Box<dyn Slugger> {
            Box::new(GithubSlugger::new())
        }
    }
    let mut registry = Registry::new();
    assert_eq!(registry.names().count(), 0);
    assert!(registry.register(Arc::new(Gfm)).is_none());
    let old = registry.register(Arc::new(Strict)).expect("replaces gfm");
    assert_eq!(old.parse_options(), ParseOptions::gfm());
    assert_eq!(
        registry.get("gfm").unwrap().parse_options(),
        ParseOptions::commonmark()
    );
    assert_eq!(registry.names().collect::<Vec<_>>(), ["gfm"]);
}

fn frontmatter(src: &str) -> Frontmatter {
    Frontmatter::extract(src).expect("frontmatter")
}

#[test]
fn resolution_order() {
    let registry = Registry::builtin();
    let fm = frontmatter("---\nmrq:\n  flavor: commonmark\n---\n");
    let no_flavor = frontmatter("---\ntitle: x\n---\n");

    let resolved = registry
        .resolve(Some("gfm"), Some(&fm), Some("commonmark"))
        .unwrap();
    assert_eq!(
        (resolved.flavor.name(), resolved.origin),
        ("gfm", FlavorOrigin::Cli)
    );

    let resolved = registry.resolve(None, Some(&fm), Some("gfm")).unwrap();
    assert_eq!(
        (resolved.flavor.name(), resolved.origin),
        ("commonmark", FlavorOrigin::Frontmatter)
    );

    let resolved = registry
        .resolve(None, Some(&no_flavor), Some("commonmark"))
        .unwrap();
    assert_eq!(
        (resolved.flavor.name(), resolved.origin),
        ("commonmark", FlavorOrigin::Config)
    );

    let resolved = registry.resolve(None, None, None).unwrap();
    assert_eq!(
        (resolved.flavor.name(), resolved.origin),
        ("gfm", FlavorOrigin::Default)
    );
    assert_eq!(flavor::DEFAULT_FLAVOR, "gfm");
}

#[test]
fn toml_frontmatter_flavor() {
    let fm = frontmatter("+++\n[mrq]\nflavor = \"commonmark\"\n+++\n");
    let resolved = Registry::builtin().resolve(None, Some(&fm), None).unwrap();
    assert_eq!(resolved.flavor.name(), "commonmark");
}

#[test]
fn unknown_frontmatter_flavor_points_at_the_value() {
    let src = "---\nmrq:\n  flavor: obsidain\n---\n";
    let fm = frontmatter(src);
    let err = Registry::builtin()
        .resolve(Some("gfm"), Some(&fm), None)
        .map(|r| r.flavor.name())
        .expect("--flavor wins without looking at the frontmatter");
    assert_eq!(err, "gfm");

    let err = Registry::builtin()
        .resolve(None, Some(&fm), Some("gfm"))
        .unwrap_err();
    let FlavorError::Unknown { name, origin, .. } = &err else {
        panic!("unexpected {err:?}");
    };
    assert_eq!(
        (name.as_str(), *origin),
        ("obsidain", FlavorOrigin::Frontmatter)
    );
    let span = err.span().expect("frontmatter errors have a span");
    assert_eq!(&src[span.start..span.end], "obsidain");

    let diag = err.to_diagnostic("doc.md");
    assert_eq!(diag.code, FLAVOR_ERROR_CODE);
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.range, span);
    assert!(
        diag.message
            .contains("known flavors: commonmark, gfm, obsidian, mkdocs")
    );
}

#[test]
fn non_string_frontmatter_flavor_is_an_error() {
    let src = "---\nmrq:\n  flavor: [gfm]\n---\n";
    let err = Registry::builtin()
        .resolve(None, Some(&frontmatter(src)), None)
        .unwrap_err();
    assert!(matches!(err, FlavorError::NotAString { .. }), "{err:?}");
    let span = err.span().unwrap();
    assert_eq!(&src[span.start..span.end], "[gfm]");
    assert_eq!(err.to_string(), "frontmatter `mrq.flavor` must be a string");
}

#[test]
fn unknown_cli_and_config_flavors_have_no_span() {
    let registry = Registry::builtin();
    let err = registry.resolve(Some("nope"), None, None).unwrap_err();
    assert!(matches!(
        &err,
        FlavorError::Unknown {
            origin: FlavorOrigin::Cli,
            ..
        }
    ));
    assert_eq!(err.span(), None);
    let err = registry.resolve(None, None, Some("pandoc")).unwrap_err();
    assert!(matches!(
        &err,
        FlavorError::Unknown {
            origin: FlavorOrigin::Config,
            ..
        }
    ));
}

/// Turns every paragraph that starts with `!!` into a blockquote whose content is the
/// rest of the paragraph, re-parsed as a nested document; warns about empty ones.
struct Bang;

impl Pass for Bang {
    fn name(&self) -> &'static str {
        "bang"
    }

    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
        let paragraphs: Vec<_> = ast
            .descendants(ast.root())
            .filter(|&id| {
                ast.kind(id).node_type() == NodeType::Paragraph
                    && ast.node(id).source(cx.src()).starts_with("!!")
            })
            .collect();
        for id in paragraphs {
            let span = ast.span(id);
            let body = mrq_core::ast::Span::new(span.start + 2, span.end);
            if cx.src()[body.start..body.end].trim().is_empty() {
                cx.report(mrq_core::diag::Diagnostic::new(
                    "bang",
                    Severity::Warning,
                    cx.file(),
                    span,
                    "empty bang",
                ));
                continue;
            }
            let nested = cx.parse_fragment(body);
            let quote = ast.create(NodeKind::BlockQuote, span);
            for &child in nested.children(nested.root()) {
                let copy = ast.import(&nested, child, |s| s);
                ast.append_child(quote, copy).unwrap();
            }
            ast.replace(id, quote).unwrap();
        }
    }
}

#[derive(Debug)]
struct Banged;

impl Flavor for Banged {
    fn name(&self) -> &'static str {
        "banged"
    }
    fn parse_options(&self) -> ParseOptions {
        ParseOptions::gfm()
    }
    fn slugger(&self) -> Box<dyn Slugger> {
        Box::new(GithubSlugger::new())
    }
    fn passes(&self) -> &[&dyn Pass] {
        &[&Bang]
    }
}

#[test]
fn passes_rewrite_the_tree_and_report_diagnostics() {
    let src = "---\nx: 1\n---\n!! !!*hi* ~~a~~\n\n!!\n";
    let parsed = flavor::parse(&Banged, src, Path::new("bang.md"));
    let ast = &parsed.ast;
    assert!(parsed.frontmatter.is_some());
    let children: Vec<_> = ast
        .children(ast.root())
        .iter()
        .map(|&id| ast.kind(id).node_type())
        .collect();
    assert_eq!(
        children,
        [
            NodeType::FrontMatter,
            NodeType::BlockQuote,
            NodeType::Paragraph
        ]
    );

    // The nested document was parsed with the flavor's options and passes (the inner
    // `!!` became a quote too), and its spans point into the full source.
    let quote = ast.children(ast.root())[1];
    let inner: Vec<_> = ast
        .descendants(quote)
        .map(|id| (ast.kind(id).node_type(), ast.node(id).source(src)))
        .collect();
    assert_eq!(
        inner,
        [
            (NodeType::BlockQuote, "!! !!*hi* ~~a~~"),
            (NodeType::BlockQuote, "!!*hi* ~~a~~"),
            (NodeType::Paragraph, "*hi* ~~a~~"),
            (NodeType::Emphasis, "*hi*"),
            (NodeType::Text, "hi"),
            (NodeType::Text, " "),
            (NodeType::Strikethrough, "~~a~~"),
            (NodeType::Text, "a"),
        ]
    );

    assert_eq!(parsed.diagnostics.len(), 1);
    let diag = &parsed.diagnostics[0];
    assert_eq!(
        (diag.code.as_str(), diag.file.as_path()),
        ("bang", Path::new("bang.md"))
    );
    assert_eq!(&src[diag.range.start..diag.range.end], "!!");
}
