//! The round trip of `mrq_core::transform::split`: expanding the `index.md` of a split
//! document with [`Includer::expand`] gives the document back, for every Markdown file in
//! `tests/corpus/` and `tests/fixtures/` at every depth, and for generated documents.
//!
//! Split into the document's own directory, the expansion is the document as it was;
//! split into a subdirectory, it is the document with its relative links rebased to
//! `index.md` there.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_core::source::{ByteRange, SourceFile, apply_edits};
use mrq_core::transform::{
    IncludeTarget, Includer, LinkRewrite, Loader, Split, SplitOptions, rewrite_links, split,
};
use proptest::prelude::*;

/// Files that do not round-trip, with the reason. Documents with `mrq:include`
/// directives are left out anyway: expanding the index expands theirs too.
const KNOWN: &[(&str, &str)] = &[];

/// The parts of a split, in memory, by path.
struct Parts(HashMap<PathBuf, String>);

impl Loader for Parts {
    fn load(&self, target: &IncludeTarget) -> io::Result<String> {
        target
            .file()
            .and_then(|path| self.0.get(path))
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}

/// The expansion of `split`'s index, read with `options`.
fn interpolate(split: &Split, options: &DocumentOptions) -> String {
    let parts = split
        .parts
        .iter()
        .map(|part| (part.path.clone(), part.text.clone()))
        .collect();
    let index = options
        .parse_source(SourceFile::new(
            Some(split.index.path.clone()),
            split.index.text.clone(),
        ))
        .unwrap();
    let expanded = Includer::new()
        .document_options(options.clone())
        .loader(Parts(parts))
        .expand(&index);
    assert_eq!(expanded.diagnostics, [], "{}", split.index.text);
    expanded.text
}

/// `doc`'s text with its relative links rebased to `to`.
fn rebased(doc: &Document, to: &Path) -> String {
    let rewrite = LinkRewrite::new(doc.name(), to);
    let edits = rewrite_links(doc, ByteRange::new(0, doc.text().len()), &rewrite);
    apply_edits(doc.text(), &edits).unwrap()
}

/// The first depth (1–6) and directory at which `doc` does not round-trip, with what the
/// expansion gave and what it should be.
fn round_trip_failure(
    doc: &Document,
    options: &DocumentOptions,
) -> Option<(u8, PathBuf, String, String)> {
    let dir = doc.path().and_then(Path::parent).unwrap_or(Path::new(""));
    for depth in 1..=6 {
        for out in [dir.to_path_buf(), dir.join("split")] {
            let split = split(doc, &SplitOptions::new().depth(depth).dir(&out));
            let expected = if out == dir {
                doc.text().to_owned()
            } else {
                rebased(doc, &split.index.path)
            };
            let actual = interpolate(&split, options);
            if actual.trim_end_matches('\n') != expected.trim_end_matches('\n') {
                return Some((depth, out, actual, expected));
            }
        }
    }
    None
}

/// Every `.md` file under `dir`, recursively, sorted.
fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
    out.sort();
}

#[test]
fn the_fixture_corpus_round_trips() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    markdown_files(&root.join("corpus"), &mut files);
    markdown_files(&root.join("fixtures"), &mut files);
    assert!(files.len() > 300, "only {} files", files.len());
    let mut failures = Vec::new();
    let mut checked = 0;
    for path in &files {
        let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
        let relative = relative.replace('\\', "/");
        // The flavor fixtures are read in their flavor.
        let mut options = DocumentOptions::new();
        for flavor in ["obsidian", "mkdocs"] {
            if relative.split('/').any(|dir| dir == flavor) {
                options = options.flavor(flavor);
            }
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(doc) = options.parse_source(SourceFile::new(Some(path.clone()), text)) else {
            continue;
        };
        if doc.text().contains("mrq:include") {
            continue;
        }
        checked += 1;
        let known = KNOWN.iter().any(|(name, _)| *name == relative);
        match round_trip_failure(&doc, &options) {
            Some((depth, out, actual, expected)) if !known => failures.push(format!(
                "{relative} at depth {depth} into {}:\n--- expanded\n{actual}\n--- expected\n{expected}",
                out.display()
            )),
            None if known => failures.push(format!("{relative} round-trips: remove it from KNOWN")),
            _ => {}
        }
    }
    assert!(checked > 250, "only {checked} files checked");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// A generated document: headings at levels 1–4 from a few titles (so anchors collide),
/// paragraphs linking to their anchors, quotes with headings, setext headings and
/// assets, separated by one or two blank lines or none.
fn document() -> impl Strategy<Value = String> {
    let title = prop::sample::select(vec!["Setup", "Usage", "Über uns", "C++ & Rust", "Notes"]);
    let anchor = prop::sample::select(vec![
        "setup",
        "setup-1",
        "setup-2",
        "usage",
        "usage-1",
        "über-uns",
        "c--rust",
        "notes-1",
        "nowhere",
    ]);
    let block = prop_oneof![
        (1..=4usize, title.clone())
            .prop_map(|(level, title)| format!("{} {title}", "#".repeat(level))),
        title.clone().prop_map(|title| format!("{title}\n---")),
        (anchor.clone(), anchor)
            .prop_map(|(a, b)| format!("See [a](#{a}) and [b][r{b}].\n\n[r{b}]: #{b}")),
        Just("![x](img/x.png) and [up](../up.md#top)".to_owned()),
        (1..=3usize, title)
            .prop_map(|(level, title)| format!("> {} {title}\n> text", "#".repeat(level))),
    ];
    let gap = prop::sample::select(vec!["\n", "\n\n", "\n\n\n"]);
    prop::collection::vec((block, gap), 0..12).prop_map(|blocks| {
        blocks
            .into_iter()
            .flat_map(|(block, gap)| [block, gap.to_owned()])
            .collect()
    })
}

proptest! {
    #[test]
    fn generated_documents_round_trip(text in document()) {
        let options = DocumentOptions::new();
        let doc = options
            .parse_source(SourceFile::new(Some("docs/guide.md".into()), text))
            .unwrap();
        let failure = round_trip_failure(&doc, &options);
        prop_assert!(failure.is_none(), "{failure:?}");
    }
}
