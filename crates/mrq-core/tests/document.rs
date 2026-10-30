//! Acceptance tests for `mrq_core::document`: parsing and loading documents, flavor
//! resolution, the anchor index, and workspaces of several files.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mrq_core::Document;
use mrq_core::ast::{Ast, NodeType, Span};
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::{DocumentError, DocumentOptions, Workspace};
use mrq_core::flavor::{
    FLAVOR_ERROR_CODE, Flavor, FlavorError, FlavorOrigin, GithubSlugger, Pass, PassContext,
    Registry, Slugger,
};
use mrq_core::frontmatter::FRONTMATTER_ERROR_CODE;
use mrq_core::input::{Input, InputResolver};
use mrq_core::parse::ParseOptions;
use mrq_core::query::{Query, QueryError};
use tempfile::TempDir;

fn write(dir: &TempDir, rel: &str, content: &str) -> PathBuf {
    let path = dir.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    path
}

fn codes(doc: &Document) -> Vec<&str> {
    doc.diagnostics().iter().map(|d| d.code.as_str()).collect()
}

// --- Document::parse and flavor resolution -----------------------------------------

#[test]
fn parse_defaults_to_gfm_without_a_path() {
    let doc = Document::parse("# Title\n\n- [ ] task\n");
    assert_eq!(doc.flavor().name(), "gfm");
    assert_eq!(doc.flavor_origin(), FlavorOrigin::Default);
    assert_eq!(doc.path(), None);
    assert!(!doc.is_writable());
    assert_eq!(doc.text(), "# Title\n\n- [ ] task\n");
    assert_eq!(doc.source().line_count(), 4);
    assert!(doc.frontmatter().is_none());
    assert_eq!(doc.diagnostics(), []);
    let first = doc.ast().children(doc.ast().root())[0];
    assert_eq!(doc.ast().node(first).node_type(), NodeType::Heading);
    assert_eq!(doc.ast().node(first).source(doc.text()), "# Title");
}

#[test]
fn frontmatter_picks_the_flavor() {
    let doc = Document::parse("---\nmrq:\n  flavor: obsidian\n---\n[[Note]]\n");
    assert_eq!(doc.flavor().name(), "obsidian");
    assert_eq!(doc.flavor_origin(), FlavorOrigin::Frontmatter);
    assert_eq!(
        doc.frontmatter().unwrap().get_str(&["mrq", "flavor"]),
        Some("obsidian")
    );
    let types: Vec<NodeType> = doc
        .ast()
        .descendants(doc.ast().root())
        .map(|id| doc.ast().kind(id).node_type())
        .collect();
    assert!(types.contains(&NodeType::WikiLink), "{types:?}");
}

#[test]
fn a_bad_frontmatter_flavor_is_a_diagnostic_and_falls_back() {
    let src = "---\nmrq:\n  flavor: nope\n---\n# A\n";
    let doc = Document::parse(src);
    assert_eq!(doc.flavor().name(), "gfm");
    assert_eq!(doc.flavor_origin(), FlavorOrigin::Default);
    assert_eq!(codes(&doc), [FLAVOR_ERROR_CODE]);
    let diagnostic = &doc.diagnostics()[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.file, Path::new("<stdin>"));
    assert_eq!(&src[diagnostic.range.start..diagnostic.range.end], "nope");

    let doc = DocumentOptions::new()
        .config_flavor("mkdocs")
        .parse("---\nmrq:\n  flavor: [x]\n---\n")
        .unwrap();
    assert_eq!(doc.flavor().name(), "mkdocs");
    assert_eq!(doc.flavor_origin(), FlavorOrigin::Config);
    assert_eq!(codes(&doc), [FLAVOR_ERROR_CODE]);
}

#[test]
fn malformed_frontmatter_is_a_diagnostic() {
    let doc = Document::parse("---\ntitle: [unclosed\n---\n# Body\n");
    assert_eq!(codes(&doc), [FRONTMATTER_ERROR_CODE]);
    assert!(doc.frontmatter().unwrap().error().is_some());
    assert_eq!(doc.flavor().name(), "gfm");
}

#[test]
fn cli_flavor_outranks_frontmatter_and_config() {
    let options = DocumentOptions::new()
        .flavor("commonmark")
        .config_flavor("mkdocs");
    let doc = options
        .parse("---\nmrq:\n  flavor: obsidian\n---\n")
        .unwrap();
    assert_eq!(doc.flavor().name(), "commonmark");
    assert_eq!(doc.flavor_origin(), FlavorOrigin::Cli);

    let doc = DocumentOptions::new()
        .config_flavor("mkdocs")
        .parse("# A\n")
        .unwrap();
    assert_eq!(doc.flavor().name(), "mkdocs");
    assert_eq!(doc.flavor_origin(), FlavorOrigin::Config);
}

#[test]
fn an_unknown_cli_or_config_flavor_is_an_error() {
    let err = DocumentOptions::new()
        .flavor("nope")
        .parse("# A\n")
        .unwrap_err();
    assert!(
        matches!(&err, FlavorError::Unknown { name, origin: FlavorOrigin::Cli, .. } if name == "nope"),
        "{err:?}"
    );
    let err = DocumentOptions::new()
        .config_flavor("nope")
        .parse("# A\n")
        .unwrap_err();
    assert!(
        matches!(
            err,
            FlavorError::Unknown {
                origin: FlavorOrigin::Config,
                ..
            }
        ),
        "{err:?}"
    );
}

/// A flavor whose only pass reports one diagnostic at the start of the document.
#[derive(Debug)]
struct Noisy;

#[derive(Debug)]
struct Complain;

impl Pass for Complain {
    fn name(&self) -> &'static str {
        "complain"
    }

    fn run(&self, _ast: &mut Ast, cx: &mut PassContext<'_>) {
        let diagnostic = Diagnostic::new(
            "noisy",
            Severity::Warning,
            cx.file(),
            Span::new(0, 1),
            "hello",
        );
        cx.report(diagnostic);
    }
}

impl Flavor for Noisy {
    fn name(&self) -> &'static str {
        "noisy"
    }

    fn parse_options(&self) -> ParseOptions {
        ParseOptions::gfm()
    }

    fn slugger(&self) -> Box<dyn Slugger> {
        Box::new(GithubSlugger::new())
    }

    fn passes(&self) -> &[&dyn Pass] {
        &[&Complain]
    }
}

#[test]
fn custom_registries_and_pass_diagnostics_come_through() {
    let mut registry = Registry::builtin();
    registry.register(Arc::new(Noisy));
    let options = DocumentOptions::new().registry(registry).flavor("noisy");
    let source = mrq_core::source::SourceFile::new(Some("notes/a.md".into()), "---\n: [\n---\n");
    let doc = options.parse_source(source).unwrap();
    assert_eq!(doc.flavor().name(), "noisy");
    assert_eq!(doc.path(), Some(Path::new("notes/a.md")));
    assert!(doc.is_writable());
    // Frontmatter errors first, then what the passes reported; both name the file.
    assert_eq!(codes(&doc), [FRONTMATTER_ERROR_CODE, "noisy"]);
    assert!(
        doc.diagnostics()
            .iter()
            .all(|d| d.file == Path::new("notes/a.md"))
    );
}

// --- Document::load and inputs -------------------------------------------------------

#[test]
fn load_reads_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        &dir,
        "a.md",
        "---\nmrq:\n  flavor: mkdocs\n---\n# A {#top}\n",
    );
    let doc = Document::load(&path).unwrap();
    assert_eq!(doc.path(), Some(path.as_path()));
    assert!(doc.is_writable());
    assert_eq!(doc.flavor().name(), "mkdocs");
    assert!(doc.anchors().contains("top"));
}

#[test]
fn load_reports_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let err = Document::load(dir.path().join("missing.md")).unwrap_err();
    assert!(matches!(err, DocumentError::Input(_)), "{err:?}");
    assert!(err.to_string().contains("missing.md"), "{err}");

    let err = DocumentOptions::new()
        .flavor("nope")
        .load(write(&dir, "a.md", "# A\n"))
        .unwrap_err();
    assert!(matches!(err, DocumentError::Flavor(_)), "{err:?}");
}

#[test]
fn stdin_documents_are_not_writable() {
    let source = Input::stdin()
        .read_with(&mut Cursor::new("# From stdin\n"))
        .unwrap();
    let doc = DocumentOptions::new().parse_source(source).unwrap();
    assert_eq!(doc.path(), None);
    assert!(!doc.is_writable());
    assert_eq!(doc.name(), Path::new("<stdin>"));

    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, "b.md", "# B\n");
    let doc = DocumentOptions::new().read(&Input::file(&path)).unwrap();
    assert_eq!(doc.name(), path.as_path());
}

// --- Anchors and toc -------------------------------------------------------------------

#[test]
fn anchors_follow_the_flavor_slugger() {
    let doc = Document::parse("# Example\n\n## Example\n\ntext\n\n### Hello, World!\n");
    let anchors: Vec<&str> = doc.anchors().iter().map(|(_, anchor)| anchor).collect();
    assert_eq!(anchors, ["example", "example-1", "hello-world"]);
    assert_eq!(doc.anchors().len(), 3);
    let headings: Vec<_> = doc.anchors().iter().map(|(id, _)| id).collect();
    assert_eq!(doc.anchors().get("example-1"), Some(headings[1]));
    assert_eq!(doc.anchors().anchor_of(headings[2]), Some("hello-world"));
    assert!(doc.anchors().contains("example"));
    // GitHub anchors are case-sensitive.
    assert!(!doc.anchors().contains("Example"));
    assert!(!doc.anchors().is_case_insensitive());
}

#[test]
fn anchors_are_built_once() {
    let doc = Document::parse("# A\n");
    assert!(std::ptr::eq(doc.anchors(), doc.anchors()));
    let clone = doc.clone();
    assert_eq!(clone.anchors().get("a"), doc.anchors().get("a"));
}

#[test]
fn explicit_ids_win_and_duplicates_keep_the_first() {
    let src =
        "---\nmrq:\n  flavor: mkdocs\n---\n# Intro {#start}\n\n## Start\n\n## Other {#start}\n";
    let doc = Document::parse(src);
    let anchors: Vec<&str> = doc.anchors().iter().map(|(_, a)| a).collect();
    assert_eq!(anchors, ["start", "start_1", "start"]);
    let first = doc.anchors().iter().next().unwrap().0;
    assert_eq!(doc.anchors().get("start"), Some(first));
}

#[test]
fn obsidian_anchors_compare_case_insensitively() {
    let doc = DocumentOptions::new()
        .flavor("obsidian")
        .parse("# My Heading\n")
        .unwrap();
    assert!(doc.anchors().is_case_insensitive());
    assert!(doc.anchors().contains("My Heading"));
    assert!(doc.anchors().contains("my heading"));
    assert!(doc.anchors().contains("MY HEADING"));
    assert!(!doc.anchors().contains("my-heading"));
}

#[test]
fn toc_lists_headings_with_level_text_and_slug() {
    let src = "# Guide\n\ntext\n\n## Install *now*\n\n### Linux\n";
    let doc = Document::parse(src);
    let entries = doc.toc();
    let toc: Vec<(u8, &str, &str, &str)> = entries
        .iter()
        .map(|h| {
            (
                h.level,
                h.text.as_str(),
                h.slug.as_str(),
                &src[h.span.start..h.span.end],
            )
        })
        .collect();
    assert_eq!(
        toc,
        [
            (1, "Guide", "guide", "# Guide"),
            (2, "Install now", "install-now", "## Install *now*"),
            (3, "Linux", "linux", "### Linux"),
        ]
    );
    assert_eq!(doc.toc()[0].node, doc.anchors().get("guide").unwrap());
}

// --- Query (CSS and jq front-ends stubbed until M3.3–M3.4) -----------------------------

#[test]
fn selector_queries_select_nodes() {
    let doc = Document::parse("# Install\n\n```sh\nmake\n```\n");
    let query = Query::selector("# Install | ```sh").unwrap();
    assert_eq!(query.source(), "# Install | ```sh");
    let found = doc.select(&query).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].text(), "make");
}

#[test]
fn css_and_jq_queries_are_not_implemented_yet() {
    let doc = Document::parse("# Install\n");
    for query in [Query::css("section").unwrap(), Query::jq(".").unwrap()] {
        let err = doc.select(&query).unwrap_err();
        assert!(matches!(err, QueryError::NotImplemented), "{err:?}");
    }
}

// --- Workspace -------------------------------------------------------------------------

#[test]
fn workspace_resolves_relative_to_the_linking_file() {
    let ws = Workspace::new("site");
    let from = Path::new("site/docs/guide/a.md");
    assert_eq!(
        ws.resolve(Some(from), "b.md"),
        Path::new("site/docs/guide/b.md")
    );
    assert_eq!(
        ws.resolve(Some(from), "./img/x.png"),
        Path::new("site/docs/guide/img/x.png")
    );
    assert_eq!(
        ws.resolve(Some(from), "../../README.md"),
        Path::new("site/README.md")
    );
    assert_eq!(
        ws.resolve(Some(from), "/index.md"),
        Path::new("site/index.md")
    );
    assert_eq!(ws.resolve(None, "x/../y.md"), Path::new("site/y.md"));
    // `..` past the start stays.
    assert_eq!(
        Workspace::new(".").resolve(Some(Path::new("a.md")), "../b.md"),
        Path::new("../b.md")
    );
    assert_eq!(
        Workspace::new(".").resolve(None, "/b.md"),
        Path::new("b.md")
    );
}

#[test]
fn workspace_loads_lazily_and_caches_by_normalized_path() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(&dir, "docs/a.md", "# A\n");
    let mut ws = Workspace::new(dir.path());
    assert!(ws.is_empty());
    assert!(ws.get(&a).is_none());

    assert_eq!(ws.load(&a).unwrap().text(), "# A\n");
    fs::write(&a, "# Changed\n").unwrap();
    let dotted = dir.path().join("docs/./x/../a.md");
    assert_eq!(ws.load(&dotted).unwrap().text(), "# A\n");
    assert_eq!(ws.len(), 1);
    assert_eq!(ws.get(&dotted).unwrap().path(), Some(a.as_path()));

    let missing = ws.load(dir.path().join("docs/missing.md")).unwrap_err();
    assert!(matches!(missing, DocumentError::Input(_)));
    assert_eq!(ws.len(), 1);

    // A link from a.md to a sibling resolves and loads on demand.
    write(&dir, "docs/b.md", "## B\n");
    let target = ws.resolve(Some(&a), "b.md");
    assert_eq!(ws.load(&target).unwrap().anchors().len(), 1);
    assert_eq!(ws.len(), 2);
}

#[test]
fn workspace_from_inputs_keeps_input_order_and_options() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "b.md", "# B\n");
    write(&dir, "a/z.md", "# Z\n");
    let inputs = InputResolver::new()
        .base_dir(dir.path())
        .resolve(&["b.md", "a"])
        .unwrap();
    let options = DocumentOptions::new().flavor("obsidian");
    let ws = Workspace::from_inputs(dir.path(), options, &inputs).unwrap();
    let names: Vec<&Path> = ws
        .documents()
        .map(|doc| doc.path().unwrap().strip_prefix(dir.path()).unwrap())
        .collect();
    assert_eq!(names, [Path::new("b.md"), Path::new("a/z.md")]);
    assert!(ws.documents().all(|doc| doc.flavor().name() == "obsidian"));
    assert_eq!(ws.root(), dir.path());
    assert_eq!(ws.options().flavor_name(), Some("obsidian"));
}

#[test]
fn workspace_insert_replaces_documents_with_the_same_path() {
    let options = DocumentOptions::new();
    let mut ws = Workspace::with_options(".", options.clone());
    let source = |path: &str, text: &str| {
        mrq_core::source::SourceFile::new(Some(path.into()), text.to_owned())
    };
    assert!(
        ws.insert(options.parse_source(source("docs/a.md", "# 1\n")).unwrap())
            .is_none()
    );
    let old = ws
        .insert(
            options
                .parse_source(source("./docs/a.md", "# 2\n"))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(old.text(), "# 1\n");
    assert_eq!(ws.len(), 1);
    assert_eq!(ws.get("docs/a.md").unwrap().text(), "# 2\n");

    // Pathless (stdin) documents are kept too, at most one of them.
    ws.insert(Document::parse("one"));
    assert!(ws.insert(Document::parse("two")).is_some());
    assert_eq!(ws.len(), 2);
    assert_eq!(ws.documents().last().unwrap().text(), "two");

    assert_eq!(ws.remove("docs/./a.md").unwrap().text(), "# 2\n");
    assert_eq!(ws.len(), 1);
}
