//! Fixture tests for `mrq_core::transform::Includer`: each top-level
//! `tests/fixtures/transform/include/<name>.md` expands to `<name>.out.md`, with the
//! diagnostics listed in `<name>.diag.txt` (none if absent). The included files live in
//! the subdirectories. The symbol includes in `tests/fixtures/transform/symbols/` work the
//! same way, with the `tree-sitter` feature.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::diag::Diagnostic;
use mrq_core::source::PositionEncoding;
use mrq_core::transform::{Expanded, Includer};
use mrq_core::validate::codes;

fn fixture_dir() -> PathBuf {
    fixtures("include")
}

/// The fixture directory `tests/fixtures/transform/<kind>`.
fn fixtures(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transform")
        .join(kind)
}

/// The host fixtures: the `.md` files directly in `dir`, except outputs.
fn hosts(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter_map(|file| {
            let name = file.strip_suffix(".md")?;
            (!name.contains('.')).then(|| name.to_owned())
        })
        .collect();
    names.sort();
    names
}

fn expand(name: &str) -> Expanded {
    expand_in(&fixture_dir(), name)
}

fn expand_in(dir: &Path, name: &str) -> Expanded {
    let doc = Document::load(dir.join(format!("{name}.md"))).unwrap();
    Includer::new().root(dir).expand(&doc)
}

/// One `file:line:col code: message` line per diagnostic, 1-based, with the file relative
/// to `dir` and that directory (canonicalized) written as `<dir>`.
fn diagnostic_lines(dir: &Path, diagnostics: &[Diagnostic]) -> String {
    let canonical = fs::canonicalize(dir).unwrap();
    let mut out = String::new();
    for diagnostic in diagnostics {
        let doc = Document::load(&diagnostic.file).unwrap();
        let position = doc
            .source()
            .offset_to_position(diagnostic.range.start, PositionEncoding::Utf8)
            .unwrap();
        let file = diagnostic
            .file
            .strip_prefix(dir)
            .unwrap_or(&diagnostic.file);
        let message = diagnostic
            .message
            .replace(&*canonical.to_string_lossy(), "<dir>");
        writeln!(
            out,
            "{}:{}:{} {}: {message}",
            file.display(),
            position.line + 1,
            position.col + 1,
            diagnostic.code,
        )
        .unwrap();
    }
    out
}

/// Expands every host fixture in `dir` and compares it with its output and diagnostics.
fn check_fixtures(dir: &Path) {
    let names = hosts(dir);
    assert!(names.len() >= 5, "{names:?}");
    for name in names {
        let expected = fs::read_to_string(dir.join(format!("{name}.out.md")))
            .unwrap_or_else(|_| panic!("{name}.out.md is missing"));
        let diag = fs::read_to_string(dir.join(format!("{name}.diag.txt"))).unwrap_or_default();
        let expanded = expand_in(dir, &name);
        assert_eq!(expanded.text, expected, "{name}");
        assert_eq!(diagnostic_lines(dir, &expanded.diagnostics), diag, "{name}");
    }
}

#[test]
fn fixtures_expand_to_their_expected_output() {
    check_fixtures(&fixture_dir());
}

#[cfg(feature = "tree-sitter")]
#[test]
fn symbol_fixtures_expand_to_their_expected_output() {
    check_fixtures(&fixtures("symbols"));
}

#[test]
fn expanding_an_expansion_changes_nothing() {
    for name in ["nested", "section", "options", "code"] {
        let dir = fixture_dir();
        let expanded = expand(name);
        let again = Document::parse(expanded.text.clone());
        let again = Includer::new().root(&dir).expand(&again);
        assert_eq!(again.text, expanded.text, "{name}");
    }
}

/// A cycle through a symlinked directory is found, and its chain shows canonical paths.
#[cfg(unix)]
#[test]
fn cycles_through_symlinks_show_canonical_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("real")).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    fs::write(
        root.join("a.md"),
        "<!-- mrq:include path=link/b.md -->\n<!-- mrq:end -->\n",
    )
    .unwrap();
    fs::write(
        root.join("real/b.md"),
        "<!-- mrq:include path=../a.md -->\n<!-- mrq:end -->\n",
    )
    .unwrap();
    let doc = Document::load(root.join("a.md")).unwrap();
    let expanded = Includer::new().root(root).expand(&doc);
    let canonical = fs::canonicalize(root).unwrap();
    let [cycle] = expanded.diagnostics.as_slice() else {
        panic!("{:?}", expanded.diagnostics);
    };
    assert_eq!(cycle.code, codes::INCLUDE_CYCLE);
    assert_eq!(cycle.file, root.join("link/b.md"));
    assert_eq!(
        cycle.message,
        format!(
            "`mrq:include` cycle: {a} includes {b}, which includes {a}",
            a = canonical.join("a.md").display(),
            b = canonical.join("real/b.md").display(),
        )
    );
}
