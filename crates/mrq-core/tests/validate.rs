//! Acceptance tests for `mrq_core::validate`: link classification and resolution over
//! parsed documents.

use std::path::PathBuf;

use mrq_core::ast::NodeType;
use mrq_core::document::{Document, DocumentOptions, Workspace};
use mrq_core::source::SourceFile;
use mrq_core::validate::{self, DocumentLink, Target};

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
