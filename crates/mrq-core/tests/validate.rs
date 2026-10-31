//! Acceptance tests for `mrq_core::validate`: link classification and resolution, and the
//! internal checks over parsed documents and a fixture repository.

use std::path::PathBuf;

use mrq_core::ast::NodeType;
use mrq_core::diag::Severity;
use mrq_core::document::{Document, DocumentOptions, Workspace};
use mrq_core::source::SourceFile;
use mrq_core::validate::{self, Checker, DocumentLink, Target, codes};

/// `text` parsed as `site/docs/a.md` (flavor from the frontmatter, else gfm).
fn doc_at(text: &str) -> Document {
    let source = SourceFile::new(Some(PathBuf::from("site/docs/a.md")), text);
    DocumentOptions::new().parse_source(source).unwrap()
}

/// (node type, source of the link, reference label, target) of every link.
fn summary(doc: &Document) -> Vec<(NodeType, &str, Option<String>, Target)> {
    let ws = Workspace::new("site");
    validate::links(doc, &ws)
        .into_iter()
        .map(|link: DocumentLink| {
            (
                link.kind,
                &doc.text()[link.span.start..link.span.end],
                link.reference,
                link.target,
            )
        })
        .collect()
}

fn file(path: &str, anchor: Option<&str>) -> Target {
    Target::File {
        path: PathBuf::from(path),
        anchor: anchor.map(Into::into),
    }
}

#[test]
fn links_and_images_are_classified_and_resolved() {
    let doc = doc_at(
        "# Intro\n\n[same](#intro) [other](b.md#setup) ![logo](../img/logo%20big.png)\n\
         [root](/index.md) <https://x.org> <me@x.org> [x]()\n",
    );
    assert_eq!(
        summary(&doc),
        [
            (
                NodeType::Link,
                "[same](#intro)",
                None,
                Target::Anchor("intro".into())
            ),
            (
                NodeType::Link,
                "[other](b.md#setup)",
                None,
                file("site/docs/b.md", Some("setup"))
            ),
            (
                NodeType::Image,
                "![logo](../img/logo%20big.png)",
                None,
                file("site/img/logo big.png", None)
            ),
            (
                NodeType::Link,
                "[root](/index.md)",
                None,
                file("site/index.md", None)
            ),
            (
                NodeType::Link,
                "<https://x.org>",
                None,
                Target::External("https://x.org".into())
            ),
            (
                NodeType::Link,
                "<me@x.org>",
                None,
                Target::Mailto("me@x.org".into())
            ),
            (NodeType::Link, "[x]()", None, Target::Empty),
        ]
    );
}

#[test]
fn reference_links_carry_their_label_and_the_definition_target() {
    let doc =
        doc_at("[full][Docs] [Docs][] [Docs] ![img][Pic]\n\n[docs]: ../c.md#x\n[pic]: p.png\n");
    let c = file("site/c.md", Some("x"));
    assert_eq!(
        summary(&doc),
        [
            (
                NodeType::Link,
                "[full][Docs]",
                Some("Docs".into()),
                c.clone()
            ),
            (NodeType::Link, "[Docs][]", Some("Docs".into()), c.clone()),
            (NodeType::Link, "[Docs]", Some("Docs".into()), c),
            (
                NodeType::Image,
                "![img][Pic]",
                Some("Pic".into()),
                file("site/docs/p.png", None)
            ),
        ]
    );
}

#[test]
fn gfm_bare_urls_are_external_or_mailto() {
    let doc = doc_at("Visit www.example.com or https://x.org/a.md, or mail me@x.org.\n");
    let targets: Vec<_> = summary(&doc)
        .into_iter()
        .map(|(.., target)| target)
        .collect();
    assert_eq!(
        targets,
        [
            Target::External("http://www.example.com".into()),
            Target::External("https://x.org/a.md".into()),
            Target::Mailto("me@x.org".into()),
        ]
    );
}

#[test]
fn obsidian_wikilinks_and_embeds() {
    let doc = doc_at(
        "---\nmrq:\n  flavor: obsidian\n---\n# Top\n\n\
         [[Other Note#Part 1|alias]] [[#Top]] ![[img/pic.png|100]] [md](Other%20Note.md)\n",
    );
    assert_eq!(
        summary(&doc),
        [
            (
                NodeType::WikiLink,
                "[[Other Note#Part 1|alias]]",
                None,
                Target::WikiLink {
                    name: "Other Note".into(),
                    anchor: Some("Part 1".into()),
                }
            ),
            (
                NodeType::WikiLink,
                "[[#Top]]",
                None,
                Target::Anchor("Top".into())
            ),
            (
                NodeType::Embed,
                "![[img/pic.png|100]]",
                None,
                Target::WikiLink {
                    name: "img/pic.png".into(),
                    anchor: None,
                }
            ),
            (
                NodeType::Link,
                "[md](Other%20Note.md)",
                None,
                file("site/docs/Other Note.md", None)
            ),
        ]
    );
}

#[test]
fn stdin_documents_resolve_against_the_root() {
    let doc = Document::parse("[a](b/c.md) [r](/d.md)\n");
    let targets: Vec<_> = summary(&doc)
        .into_iter()
        .map(|(.., target)| target)
        .collect();
    assert_eq!(
        targets,
        [file("site/b/c.md", None), file("site/d.md", None)]
    );
}

/// `text` parsed as `path` with `flavor`.
fn doc_with(flavor: &str, path: &str, text: &str) -> Document {
    let source = SourceFile::new(Some(PathBuf::from(path)), text);
    DocumentOptions::new()
        .flavor(flavor)
        .parse_source(source)
        .unwrap()
}

/// (code, severity, source of the primary span) of every diagnostic `Checker` reports
/// for `doc` in a workspace under `site` holding `others`.
fn check(doc: &Document, others: Vec<Document>) -> Vec<(String, Severity, String)> {
    let mut ws = Workspace::new("site");
    for other in others {
        ws.insert(other);
    }
    Checker::new(&ws)
        .check(doc)
        .into_iter()
        .map(|diag| {
            assert_eq!(diag.file, doc.name());
            let source = doc.text()[diag.range.start..diag.range.end].to_owned();
            (diag.code, diag.severity, source)
        })
        .collect()
}

fn found(code: &str, severity: Severity, source: &str) -> (String, Severity, String) {
    (code.into(), severity, source.into())
}

#[test]
fn same_document_anchors_go_through_the_slugger() {
    let doc = doc_at("# Intro: Setup!\n\n[ok](#intro-setup) [bad](#intro) [top](#)\n");
    assert_eq!(
        check(&doc, vec![]),
        [found(
            codes::MISSING_ANCHOR,
            Severity::Error,
            "[bad](#intro)"
        )]
    );
}

#[test]
fn files_assets_and_cross_file_anchors() {
    let doc = doc_at(
        "[ok](b.md) [ok](b.md#setup) [bad](b.md#nope) [gone](c.md#x) ![gone](img.png)\n\
         [web](https://x.org/missing.md) [mail](mailto:a@b.org) [self](a.md#a) [self](a.md#nope)\n\n\
         # A\n",
    );
    let b = doc_at_path("site/docs/b.md", "# Setup\n");
    assert_eq!(
        check(&doc, vec![b, doc.clone()]),
        [
            found(codes::MISSING_ANCHOR, Severity::Error, "[bad](b.md#nope)"),
            found(codes::MISSING_FILE, Severity::Error, "[gone](c.md#x)"),
            found(codes::MISSING_ASSET, Severity::Error, "![gone](img.png)"),
            found(codes::MISSING_ANCHOR, Severity::Error, "[self](a.md#nope)"),
        ]
    );
}

fn doc_at_path(path: &str, text: &str) -> Document {
    doc_with("gfm", path, text)
}

#[test]
fn duplicate_anchors_are_reported_at_the_repeat() {
    let doc = doc_with("mkdocs", "site/a.md", "# One {#x}\n\n# Two {#x}\n\n# One\n");
    let diags = Checker::new(&Workspace::new("site")).check(&doc);
    assert_eq!(diags.len(), 1);
    let diag = &diags[0];
    assert_eq!(
        (diag.code.as_str(), diag.severity),
        (codes::DUPLICATE_ANCHOR, Severity::Warning)
    );
    assert_eq!(&doc.text()[diag.range.start..diag.range.end], "# Two {#x}");
    assert_eq!(
        &doc.text()[diag.labels[0].range.start..diag.labels[0].range.end],
        "# One {#x}"
    );
}

#[test]
fn obsidian_anchors_repeat_regardless_of_case() {
    let doc = doc_with("obsidian", "site/a.md", "# Setup\n\n# setup\n");
    assert_eq!(
        check(&doc, vec![]),
        [found(codes::DUPLICATE_ANCHOR, Severity::Warning, "# setup")]
    );
}

#[test]
fn undefined_and_unused_footnotes() {
    let doc = doc_at("a[^1] b[^Two] c[^nope] d\\[^esc]\n\n[^1]: x\n[^two]: y\n[^unused]: z\n");
    assert_eq!(
        check(&doc, vec![]),
        [
            found(codes::UNDEFINED_FOOTNOTE, Severity::Error, "[^nope]"),
            found(codes::UNUSED_FOOTNOTE, Severity::Warning, "[^unused]"),
        ]
    );
}

#[test]
fn undefined_and_unused_reference_definitions() {
    let doc = doc_at(
        "[x][nope] [y][Used] [z][] [*em*][gone] \\[w][esc] [shortcut]\n\n\
         [used]: https://x.org\n[unused]: https://y.org\n",
    );
    assert_eq!(
        check(&doc, vec![]),
        [
            found(codes::UNDEFINED_REFERENCE, Severity::Error, "[nope]"),
            found(codes::UNDEFINED_REFERENCE, Severity::Error, "[z][]"),
            found(codes::UNDEFINED_REFERENCE, Severity::Error, "[gone]"),
            found(
                codes::UNUSED_REFERENCE,
                Severity::Warning,
                "[unused]: https://y.org"
            ),
        ]
    );
}

#[test]
fn reference_definitions_are_recovered() {
    let doc = doc_at("[a]\n\n> [a]: <x y.md>\n");
    let defs = validate::reference_definitions(&doc);
    assert_eq!(defs.len(), 1);
    assert_eq!(
        (defs[0].label.as_str(), defs[0].url.as_str()),
        ("a", "x y.md")
    );
}

#[test]
fn wikilinks_are_looked_up_by_name_across_the_vault() {
    let doc = doc_with(
        "obsidian",
        "site/vault/Home.md",
        "# Home\n\n[[Other Note]] [[other note#part 1]] [[Other Note#Nope]] [[Missing]]\n\
         [[sub/Deep]] [[Deep.md|deep]] [[vault/sub/Deep#Down]] [[Other Note#^blk]] [[Other Note#^nope]]\n\
         [[#home]] [[#Gone]] [[#^here]] [[#^gone]] text ^here\n",
    );
    let other = doc_with(
        "obsidian",
        "site/vault/Other Note.md",
        "# Part 1\n\nA paragraph. ^blk\n",
    );
    let deep = doc_with("obsidian", "site/vault/sub/Deep.md", "## Down\n");
    assert_eq!(
        check(&doc, vec![other, deep]),
        [
            found(
                codes::MISSING_ANCHOR,
                Severity::Error,
                "[[Other Note#Nope]]"
            ),
            found(codes::UNRESOLVED_WIKILINK, Severity::Warning, "[[Missing]]"),
            found(
                codes::MISSING_ANCHOR,
                Severity::Error,
                "[[Other Note#^nope]]"
            ),
            found(codes::MISSING_ANCHOR, Severity::Error, "[[#Gone]]"),
            found(codes::MISSING_ANCHOR, Severity::Error, "[[#^gone]]"),
        ]
    );
}

/// Every check over `tests/fixtures/validate`, a small repository with a gfm README, a
/// mkdocs page, an Obsidian vault, blog posts with a frontmatter schema (from its
/// `.mrq.yaml`) and a book whose `mrq:` directives are stale, missing or cyclic, rendered
/// by the plain emitter with paths relative to the fixture.
#[cfg(feature = "frontmatter-schema")]
#[test]
fn fixture_repository_diagnostics() {
    use mrq_core::config::ConfigLoader;
    use mrq_core::diag::emit_plain;
    use mrq_core::input::InputResolver;
    use mrq_core::validate::FrontmatterSchemas;
    use std::path::Path;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/validate");
    let config = ConfigLoader::new(&root)
        .explicit(Some(root.join(".mrq.yaml")))
        .load()
        .unwrap();
    let schemas = FrontmatterSchemas::from_config(&config).unwrap();
    let inputs = InputResolver::new()
        .base_dir(&root)
        .resolve(&["."])
        .unwrap();
    let ws = Workspace::from_inputs(&root, DocumentOptions::new(), &inputs).unwrap();
    let diagnostics = Checker::new(&ws)
        .with_frontmatter_schemas(&schemas)
        .check_workspace();
    let sources: Vec<SourceFile> = ws.documents().map(|doc| doc.source().clone()).collect();
    let mut out = Vec::new();
    emit_plain(&diagnostics, &sources, &mut out).unwrap();
    let prefix = format!("{}/", root.display()).replace('\\', "/");
    let out = String::from_utf8(out)
        .unwrap()
        .replace('\\', "/")
        .replace(&prefix, "");
    insta::assert_snapshot!(out);
}

#[test]
fn check_workspace_covers_every_document_in_order() {
    let mut ws = Workspace::new("site");
    ws.insert(doc_at_path("site/b.md", "[x](#nope)\n"));
    ws.insert(doc_at_path("site/a.md", "[x](missing.md)\n"));
    let diags = Checker::new(&ws).check_workspace();
    let summary: Vec<_> = diags
        .iter()
        .map(|diag| (diag.file.to_str().unwrap(), diag.code.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            ("site/b.md", codes::MISSING_ANCHOR),
            ("site/a.md", codes::MISSING_FILE)
        ]
    );
}

#[test]
fn stale_toc_regions_are_reported_at_the_start_marker() {
    let doc = doc_at(
        "# Title\n\n<!-- mrq:toc -->\n- [Title](#title)\n<!-- mrq:end -->\n\n\
         <!-- mrq:toc depth=1 -->\n\n- [Title](#title)\n\n<!-- mrq:end -->\n\n\
         <!-- mrq:toc depth=9 -->\n<!-- mrq:end -->\n\n<!-- mrq:toc depth=2 -->\n\n## Install\n",
    );
    assert_eq!(
        check(&doc, vec![]),
        [
            found(codes::STALE_TOC, Severity::Warning, "<!-- mrq:toc -->"),
            found("directive", Severity::Error, "depth=9"),
            found(
                codes::STALE_TOC,
                Severity::Warning,
                "<!-- mrq:toc depth=2 -->"
            ),
        ]
    );
}

#[test]
fn include_targets_must_exist() {
    let doc = doc_at(
        "<!-- mrq:include -->\n\n<!-- mrq:include path=b.md -->\n<!-- mrq:end -->\n\n\
         <!-- mrq:include path=\"../gone.md\" shift=1 -->\n\n\
         <!-- mrq:include path=https://x.org/remote.md -->\n\n\
         <!-- mrq:include path=/docs/b.md -->\n",
    );
    let b = doc_at_path("site/docs/b.md", "# B\n");
    assert_eq!(
        check(&doc, vec![b]),
        [
            found(
                codes::MISSING_INCLUDE,
                Severity::Error,
                "<!-- mrq:include -->"
            ),
            found(
                codes::MISSING_INCLUDE,
                Severity::Error,
                "path=\"../gone.md\""
            ),
        ]
    );
}

#[test]
fn include_cycles_are_reported_with_the_chain() {
    let a = doc_at_path(
        "site/a.md",
        "<!-- mrq:include path=sub/b.md -->\n<!-- mrq:end -->\n\n<!-- mrq:include path=c.md -->\n",
    );
    let b = doc_at_path("site/sub/b.md", "<!-- mrq:include path=../a.md -->\n");
    let c = doc_at_path("site/c.md", "<!-- mrq:include path=./c.md -->\n");
    let d = doc_at_path("site/d.md", "<!-- mrq:include path=sub/b.md -->\n");
    let mut ws = Workspace::new("site");
    for doc in [a.clone(), b, c, d.clone()] {
        ws.insert(doc);
    }
    let mut checker = Checker::new(&ws);
    let diags = checker.check(&a);
    let summary: Vec<_> = diags
        .iter()
        .map(|diag| {
            (
                diag.code.as_str(),
                &a.text()[diag.range.start..diag.range.end],
                diag.help.as_deref().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [(
            codes::INCLUDE_CYCLE,
            "path=sub/b.md",
            "site/a.md includes site/sub/b.md, which includes site/a.md"
        )]
    );
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(
        diags[0].message,
        "`mrq:include` cycle: `sub/b.md` includes this file again"
    );
    let c_diags = checker.check(ws.get("site/c.md").unwrap());
    assert_eq!(
        c_diags
            .iter()
            .map(|d| d.help.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["site/c.md includes itself"]
    );
    // d includes the a/b cycle but is not part of it: a and b report it.
    assert_eq!(checker.check(&d), vec![]);
}

#[test]
fn stale_include_regions_are_reported_at_the_start_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("chapter.md"), "# Chapter\n\nText.\n").unwrap();
    let host = "<!-- mrq:include path=chapter.md -->\n## Old\n<!-- mrq:end -->\n\n\
                <!-- mrq:include path=chapter.md shift=1 -->\n\n## Chapter\n\nText.\n\n<!-- mrq:end -->\n\n\
                <!-- mrq:include path=chapter.md -->\n\n\
                <!-- mrq:include path=chapter.md section=Nowhere -->\n<!-- mrq:end -->\n\n\
                <!-- mrq:include path=https://example.com/remote.md -->\n<!-- mrq:end -->\n";
    let path = root.join("host.md");
    std::fs::write(&path, host).unwrap();
    let doc = Document::load(&path).unwrap();
    let ws = Workspace::new(root);
    let diags = Checker::new(&ws).check(&doc);
    let summary: Vec<_> = diags
        .iter()
        .map(|diag| {
            (
                diag.code.as_str(),
                diag.severity,
                &host[diag.range.start..diag.range.end],
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                codes::STALE_INCLUDE,
                Severity::Warning,
                "<!-- mrq:include path=chapter.md -->"
            ),
            (
                mrq_core::transform::INCLUDE_CODE,
                Severity::Error,
                "section=Nowhere"
            ),
        ]
    );
    assert_eq!(diags[0].message, "`mrq:include` region is out of date");
    assert_eq!(
        diags[0].help.as_deref(),
        Some("run `mrq interpolate --in-place` to regenerate it")
    );
}
