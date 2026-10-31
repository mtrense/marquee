//! Fixture tests for `mrq_core::transform::rewrite_links`: each
//! `tests/fixtures/transform/links/<name>.md`, written in `from` and rewritten for `to`
//! with the [`LinkMap`] that [`FIXTURES`] gives, becomes `<name>.out.md`.

use std::fs;
use std::path::{Path, PathBuf};

use mrq_core::document::DocumentOptions;
use mrq_core::source::{ByteRange, SourceFile, apply_edits};
use mrq_core::transform::{LinkMap, LinkRewrite, Location, rewrite_links};
use mrq_core::validate::{Target, links};
use mrq_core::{Document, Workspace};

/// A fixture: name, flavor, the file it is written in, the file it moves to, and its map.
struct Fixture {
    name: &'static str,
    flavor: &'static str,
    from: &'static str,
    to: &'static str,
    map: fn() -> LinkMap,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "rebase-up",
        flavor: "gfm",
        from: "docs/guide/page.md",
        to: "page.md",
        map: LinkMap::new,
    },
    Fixture {
        name: "rebase-down",
        flavor: "gfm",
        from: "my docs/page.md",
        to: "out/part/page.md",
        map: LinkMap::new,
    },
    Fixture {
        name: "wikilinks",
        flavor: "obsidian",
        from: "vault/notes/page.md",
        to: "vault/page.md",
        map: LinkMap::new,
    },
    Fixture {
        name: "merge",
        flavor: "gfm",
        from: "docs/install.md",
        to: "guide.md",
        map: merge_map,
    },
    Fixture {
        name: "split",
        flavor: "gfm",
        from: "guide.md",
        to: "guide/02-install.md",
        map: split_map,
    },
];

/// `docs/intro.md` and `docs/install.md` merged into `guide.md`, where `install.md`'s
/// `#setup` collides with one in `intro.md`.
fn merge_map() -> LinkMap {
    let mut map = LinkMap::new();
    map.insert_file("docs/intro.md", Location::output().with_anchor("intro"));
    map.insert_file("docs/install.md", Location::output().with_anchor("install"));
    map.insert_anchor(
        "docs/install.md",
        "setup",
        Location::output().with_anchor("setup-1"),
    );
    map
}

/// `guide.md` split into `guide/01-intro.md` and `guide/02-install.md`.
fn split_map() -> LinkMap {
    let mut map = LinkMap::new();
    for (anchor, file) in [
        ("intro", "guide/01-intro.md"),
        ("install", "guide/02-install.md"),
        ("requirements", "guide/02-install.md"),
        ("über-uns", "guide/03-über uns.md"),
    ] {
        map.insert_anchor("guide.md", anchor, Location::file(file).with_anchor(anchor));
    }
    map
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transform/links")
}

fn read(fixture: &Fixture) -> Document {
    let src = fs::read_to_string(fixture_dir().join(format!("{}.md", fixture.name))).unwrap();
    DocumentOptions::new()
        .flavor(fixture.flavor)
        .parse(src)
        .unwrap()
}

fn rewritten(fixture: &Fixture) -> String {
    let doc = read(fixture);
    let map = (fixture.map)();
    let rewrite = LinkRewrite::new(fixture.from, fixture.to).map(&map);
    let edits = rewrite_links(&doc, ByteRange::new(0, doc.text().len()), &rewrite);
    apply_edits(doc.text(), &edits).unwrap()
}

#[test]
fn fixtures_rewrite_to_their_expected_output() {
    for fixture in FIXTURES {
        let expected =
            fs::read_to_string(fixture_dir().join(format!("{}.out.md", fixture.name))).unwrap();
        assert_eq!(rewritten(fixture), expected, "{}", fixture.name);
    }
}

#[test]
fn every_fixture_is_listed() {
    for entry in fs::read_dir(fixture_dir()).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap();
        let name = file.split('.').next().unwrap();
        assert!(
            FIXTURES.iter().any(|fixture| fixture.name == name),
            "{file} is not in FIXTURES"
        );
    }
}

/// The resolved targets of every link of `src` read as the file `path`.
fn targets(src: &str, path: &str) -> Vec<Target> {
    let doc = DocumentOptions::new()
        .parse_source(SourceFile::new(Some(PathBuf::from(path)), src))
        .unwrap();
    links(&doc, &Workspace::new(""))
        .into_iter()
        .map(|link| link.target)
        .collect()
}

#[test]
fn rebased_links_resolve_to_the_same_files() {
    for name in ["rebase-up", "rebase-down"] {
        let fixture = FIXTURES
            .iter()
            .find(|fixture| fixture.name == name)
            .unwrap();
        let before = targets(read(fixture).text(), fixture.from);
        let after = targets(&rewritten(fixture), fixture.to);
        assert_eq!(after, before, "{}", fixture.name);
    }
}
