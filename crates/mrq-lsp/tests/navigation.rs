//! Navigation: definition, references, document and workspace symbols, and hover, over
//! a fixture workspace of linked documents.

mod support;

use mrq_lsp::ls_types::{
    DocumentSymbol, HoverProviderCapability, Location, OneOf, Range, SymbolKind,
};
use support::{Fixture, TestClient, location, range};

/// A gfm workspace: `a.md` links into `b.md` (directly, by reference and to the file),
/// to its own heading and to a missing file, with a footnote; `b.md` links back and to
/// its own heading; `c.md` links to `b.md` and the web.
const A: &str = "# Guide

See [setup](b.md#setup), [top](b.md) and [here](#usage).
A note[^n], a [ref][r] and [gone](gone.md).

## Usage

[r]: b.md#setup

[^n]: The note text.
";

const B: &str = "# B

## Setup

Setup text.

### Deep

## Other

See [back](a.md#usage) and [self](#setup).
";

const C: &str = "[c](b.md) [w](https://example.org)\n";

fn workspace() -> Fixture {
    Fixture::new(&[("a.md", A), ("b.md", B), ("c.md", C)])
}

async fn client_for(fixture: &Fixture) -> TestClient {
    let mut client = TestClient::start();
    client.initialize(&[fixture.root()]).await;
    client
}

#[tokio::test]
async fn initialize_advertises_navigation() {
    let fixture = Fixture::new(&[]);
    let mut client = TestClient::start();
    let capabilities = client.initialize(&[fixture.root()]).await.capabilities;
    assert_eq!(capabilities.definition_provider, Some(OneOf::Left(true)));
    assert_eq!(capabilities.references_provider, Some(OneOf::Left(true)));
    assert_eq!(
        capabilities.document_symbol_provider,
        Some(OneOf::Left(true))
    );
    assert_eq!(
        capabilities.workspace_symbol_provider,
        Some(OneOf::Left(true))
    );
    assert_eq!(
        capabilities.hover_provider,
        Some(HoverProviderCapability::Simple(true))
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn definition_follows_links_references_and_footnotes() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let (a, b) = (fixture.uri("a.md"), fixture.uri("b.md"));
    // To the heading line of an anchor in another file, or the top of the file.
    let setup = location(&b, (2, 0), (2, 8));
    assert_eq!(client.definition(&a, (2, 6)).await, Some(setup.clone()));
    assert_eq!(
        client.definition(&a, (2, 27)).await,
        Some(location(&b, (0, 0), (0, 3)))
    );
    // A heading of the document itself.
    assert_eq!(
        client.definition(&a, (2, 45)).await,
        Some(location(&a, (5, 0), (5, 8)))
    );
    // A reference link goes to its definition, and the definition to its target.
    assert_eq!(
        client.definition(&a, (3, 16)).await,
        Some(location(&a, (7, 0), (7, 15)))
    );
    assert_eq!(client.definition(&a, (7, 2)).await, Some(setup));
    // A footnote reference goes to the definition's first line.
    assert_eq!(
        client.definition(&a, (3, 7)).await,
        Some(location(&a, (9, 0), (9, 20)))
    );
    // A missing file, plain text and the web lead nowhere.
    assert_eq!(client.definition(&a, (3, 30)).await, None);
    assert_eq!(client.definition(&a, (2, 1)).await, None);
    assert_eq!(client.definition(&fixture.uri("c.md"), (0, 12)).await, None);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn definition_uses_the_text_of_open_documents() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let a = client
        .open(&fixture.path("a.md"), "Moved: [x](b.md#other)\n")
        .await;
    assert_eq!(
        client.definition(&a, (0, 8)).await,
        Some(location(&fixture.uri("b.md"), (8, 0), (8, 8)))
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn definition_follows_wikilinks() {
    let fixture = Fixture::new(&[
        (".mrq.yaml", "flavor: obsidian\n"),
        ("n.md", "# Note\n\n[[o#Part]] [[o]] [[#Note]] [[missing]]\n"),
        ("o.md", "# O\n\n## Part\n\ntext ^blk\n"),
    ]);
    let mut client = client_for(&fixture).await;
    let (n, o) = (fixture.uri("n.md"), fixture.uri("o.md"));
    assert_eq!(
        client.definition(&n, (2, 2)).await,
        Some(location(&o, (2, 0), (2, 7)))
    );
    assert_eq!(
        client.definition(&n, (2, 12)).await,
        Some(location(&o, (0, 0), (0, 3)))
    );
    assert_eq!(
        client.definition(&n, (2, 19)).await,
        Some(location(&n, (0, 0), (0, 6)))
    );
    assert_eq!(client.definition(&n, (2, 30)).await, None);
    // References to a heading include wikilinks.
    assert_eq!(
        client.references(&o, (2, 4), false).await,
        [location(&n, (2, 0), (2, 10))]
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn references_to_a_heading_come_from_every_file() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let (a, b) = (fixture.uri("a.md"), fixture.uri("b.md"));
    let expected = [
        location(&a, (2, 4), (2, 23)),
        location(&a, (3, 14), (3, 22)),
        location(&a, (7, 0), (7, 15)),
        location(&b, (10, 27), (10, 41)),
    ];
    assert_eq!(client.references(&b, (2, 4), false).await, expected);
    let with_heading = client.references(&b, (2, 4), true).await;
    assert_eq!(with_heading[..3], expected[..3]);
    assert_eq!(with_heading[3], location(&b, (2, 0), (2, 8)));
    assert_eq!(with_heading[4..], expected[3..]);
    // On a link: the references to its target.
    assert_eq!(client.references(&a, (2, 6), false).await, expected);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn references_to_a_file_are_the_links_to_it() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let (a, b) = (fixture.uri("a.md"), fixture.uri("b.md"));
    let expected = [
        location(&a, (2, 4), (2, 23)),
        location(&a, (2, 25), (2, 36)),
        location(&a, (3, 14), (3, 22)),
        location(&a, (7, 0), (7, 15)),
        location(&fixture.uri("c.md"), (0, 0), (0, 9)),
    ];
    // Outside headings and links; and on a link to the file.
    assert_eq!(client.references(&b, (4, 2), true).await, expected);
    assert_eq!(client.references(&a, (2, 27), false).await, expected);
    assert!(client.shutdown().await);
}

/// (name, range, selection range, children) of `symbols`, recursively.
fn outline(symbols: &[DocumentSymbol]) -> Vec<(String, Range, Range, Vec<String>)> {
    let mut out = Vec::new();
    for symbol in symbols {
        assert_eq!(symbol.kind, SymbolKind::STRING);
        let children = symbol.children.as_deref().unwrap_or_default();
        out.push((
            symbol.name.clone(),
            symbol.range,
            symbol.selection_range,
            children.iter().map(|child| child.name.clone()).collect(),
        ));
        out.extend(outline(children));
    }
    out
}

#[tokio::test]
async fn document_symbols_are_the_nested_headings() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let symbols = client.document_symbols(&fixture.uri("b.md")).await;
    let entry = |name: &str, section, heading, children: &[&str]| {
        let children = children.iter().map(|&c| c.to_owned()).collect();
        (name.to_owned(), section, heading, children)
    };
    assert_eq!(
        outline(&symbols),
        [
            entry(
                "B",
                range((0, 0), (11, 0)),
                range((0, 0), (0, 3)),
                &["Setup", "Other"]
            ),
            entry(
                "Setup",
                range((2, 0), (8, 0)),
                range((2, 0), (2, 8)),
                &["Deep"]
            ),
            entry("Deep", range((6, 0), (8, 0)), range((6, 0), (6, 8)), &[]),
            entry("Other", range((8, 0), (11, 0)), range((8, 0), (8, 8)), &[]),
        ]
    );
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn workspace_symbols_match_headings_fuzzily() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let symbols = client.workspace_symbols("stp").await;
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Setup");
    assert_eq!(symbols[0].kind, SymbolKind::STRING);
    assert_eq!(symbols[0].container_name.as_deref(), Some("b.md"));
    assert_eq!(
        symbols[0].location,
        OneOf::Left(location(&fixture.uri("b.md"), (2, 0), (2, 8)))
    );
    let all: Vec<_> = client
        .workspace_symbols("")
        .await
        .into_iter()
        .map(|symbol| symbol.name)
        .collect();
    assert_eq!(all, ["Guide", "Usage", "B", "Setup", "Deep", "Other"]);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn hover_previews_link_targets_and_footnotes() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let a = fixture.uri("a.md");
    let section = "`b.md#setup`\n\n## Setup\n\nSetup text.".to_owned();
    assert_eq!(
        client.hover(&a, (2, 6)).await,
        Some((section.clone(), Some(range((2, 4), (2, 23)))))
    );
    // A reference link and its definition show the definition's destination.
    assert_eq!(
        client.hover(&a, (3, 16)).await,
        Some((section.clone(), Some(range((3, 14), (3, 22)))))
    );
    assert_eq!(
        client.hover(&a, (7, 2)).await,
        Some((section, Some(range((7, 0), (7, 15)))))
    );
    // A file: its first lines.
    let (file, _) = client.hover(&a, (2, 27)).await.unwrap();
    assert!(file.starts_with("`b.md`\n\n# B\n\n## Setup\n"), "{file}");
    // A footnote: the definition's text.
    assert_eq!(
        client.hover(&a, (3, 7)).await,
        Some(("The note text.".to_owned(), Some(range((3, 6), (3, 10)))))
    );
    // The web: the URL; a missing file and plain text: nothing.
    assert_eq!(
        client.hover(&fixture.uri("c.md"), (0, 12)).await,
        Some((
            "`https://example.org`".to_owned(),
            Some(range((0, 10), (0, 34)))
        ))
    );
    assert_eq!(client.hover(&a, (3, 30)).await, None);
    assert_eq!(client.hover(&a, (2, 1)).await, None);
    assert!(client.shutdown().await);
}

#[tokio::test]
async fn requests_for_unknown_documents_find_nothing() {
    let fixture = workspace();
    let mut client = client_for(&fixture).await;
    let unknown = fixture.uri("none.md");
    assert_eq!(client.definition(&unknown, (0, 0)).await, None);
    assert_eq!(
        client.references(&unknown, (0, 0), true).await,
        Vec::<Location>::new()
    );
    assert_eq!(client.hover(&unknown, (0, 0)).await, None);
    assert_eq!(
        client.document_symbols(&unknown).await,
        Vec::<DocumentSymbol>::new()
    );
    assert!(client.shutdown().await);
}
