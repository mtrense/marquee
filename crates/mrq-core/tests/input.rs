//! Acceptance tests for `mrq_core::input` over temporary directory trees.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use mrq_core::input::{Input, InputError, InputKind, InputResolver};
use tempfile::TempDir;

/// Creates a temp dir containing `files` (relative paths, parents created as needed).
fn tree(files: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for file in files {
        let path = dir.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("# {file}\n")).unwrap();
    }
    dir
}

fn write(dir: &TempDir, rel: &str, content: &str) {
    let path = dir.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Resolved inputs as `/`-separated paths relative to `base`, stdin as `-`.
fn names(base: &Path, inputs: &[Input]) -> Vec<String> {
    inputs
        .iter()
        .map(|input| match input.path() {
            None => "-".to_owned(),
            Some(path) => path
                .strip_prefix(base)
                .unwrap_or(path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        })
        .collect()
}

fn resolve(dir: &TempDir, args: &[&str]) -> Vec<String> {
    let inputs = InputResolver::new()
        .base_dir(dir.path())
        .resolve(args)
        .unwrap();
    names(dir.path(), &inputs)
}

#[test]
fn explicit_files_are_kept_in_argument_order_whatever_their_extension() {
    let dir = tree(&["b.md", "a.md", "notes.txt"]);
    assert_eq!(
        resolve(&dir, &["b.md", "notes.txt", "a.md"]),
        ["b.md", "notes.txt", "a.md"]
    );
}

#[test]
fn directories_are_walked_recursively_for_markdown_in_sorted_order() {
    let dir = tree(&[
        "docs/z.md",
        "docs/a.markdown",
        "docs/B.MD",
        "docs/sub/c.md",
        "docs/image.png",
        "docs/page.mdx",
        "docs/.hidden.md",
        "docs/.hidden/d.md",
    ]);
    assert_eq!(
        resolve(&dir, &["docs"]),
        ["docs/B.MD", "docs/a.markdown", "docs/sub/c.md", "docs/z.md"]
    );
}

#[test]
fn gitignore_and_ignore_files_are_respected_without_a_git_repository() {
    let dir = tree(&[
        "keep.md",
        "build/out.md",
        "draft.md",
        "sub/local.md",
        "sub/x.md",
    ]);
    write(&dir, ".gitignore", "build/\n");
    write(&dir, ".ignore", "draft.md\n");
    write(&dir, "sub/.gitignore", "local.md\n");
    assert_eq!(resolve(&dir, &["."]), ["keep.md", "sub/x.md"]);
}

#[test]
fn config_ignore_patterns_prune_walks_and_globs_but_not_explicit_files() {
    let dir = tree(&[
        "README.md",
        "CHANGELOG.md",
        "vendor/lib/x.md",
        "docs/CHANGELOG.md",
        "docs/a.md",
    ]);
    let resolver = InputResolver::new()
        .base_dir(dir.path())
        .ignore_patterns(["vendor/**", "/CHANGELOG.md"]);
    let walked = resolver.resolve(&["."]).unwrap();
    assert_eq!(
        names(dir.path(), &walked),
        ["README.md", "docs/CHANGELOG.md", "docs/a.md"]
    );
    let globbed = resolver.resolve(&["**/*.md"]).unwrap();
    assert_eq!(
        names(dir.path(), &globbed),
        ["README.md", "docs/CHANGELOG.md", "docs/a.md"]
    );
    let explicit = resolver
        .resolve(&["CHANGELOG.md", "vendor/lib/x.md"])
        .unwrap();
    assert_eq!(
        names(dir.path(), &explicit),
        ["CHANGELOG.md", "vendor/lib/x.md"]
    );
}

#[test]
fn invalid_config_ignore_pattern_is_an_error() {
    let dir = tree(&["a.md"]);
    let err = InputResolver::new()
        .base_dir(dir.path())
        .ignore_patterns(["a/**/[z-a]"])
        .resolve(&["."])
        .unwrap_err();
    assert!(
        matches!(err, InputError::InvalidIgnorePattern { .. }),
        "{err}"
    );
}

#[test]
fn globs_expand_relative_to_the_base_dir_in_sorted_order() {
    let dir = tree(&[
        "docs/b.md",
        "docs/a.md",
        "docs/deep/c.md",
        "docs/deep/c.txt",
        "other/d.md",
    ]);
    assert_eq!(
        resolve(&dir, &["docs/**/*.md"]),
        ["docs/a.md", "docs/b.md", "docs/deep/c.md"]
    );
    // `*` does not cross directory separators.
    assert_eq!(resolve(&dir, &["docs/*.md"]), ["docs/a.md", "docs/b.md"]);
    // A glob selects files explicitly, so it is not limited to Markdown extensions.
    assert_eq!(
        resolve(&dir, &["docs/deep/*"]),
        ["docs/deep/c.md", "docs/deep/c.txt"]
    );
    assert_eq!(resolve(&dir, &["*/{a,d}.md"]), ["docs/a.md", "other/d.md"]);
}

#[test]
fn globs_respect_gitignore() {
    let dir = tree(&["a.md", "gen/b.md"]);
    write(&dir, ".gitignore", "gen/\n");
    assert_eq!(resolve(&dir, &["**/*.md"]), ["a.md"]);
}

#[test]
fn absolute_globs_work() {
    let dir = tree(&["x/a.md", "x/b.txt"]);
    let pattern = format!("{}/x/*.md", dir.path().display());
    let inputs = InputResolver::new().resolve(&[pattern.as_str()]).unwrap();
    assert_eq!(names(dir.path(), &inputs), ["x/a.md"]);
}

#[test]
fn existing_path_with_glob_characters_is_taken_literally() {
    let dir = tree(&["[draft].md", "d.md"]);
    assert_eq!(resolve(&dir, &["[draft].md"]), ["[draft].md"]);
}

#[test]
fn argument_order_is_preserved_and_duplicates_are_dropped() {
    let dir = tree(&["a.md", "docs/b.md", "docs/c.md"]);
    assert_eq!(
        resolve(&dir, &["docs/c.md", "docs", "./a.md", "a.md", "docs/*.md"]),
        ["docs/c.md", "docs/b.md", "a.md"]
    );
}

#[test]
fn stdin_is_detected_and_not_writable() {
    let dir = tree(&["a.md"]);
    let inputs = InputResolver::new()
        .base_dir(dir.path())
        .resolve(&["a.md", "-"])
        .unwrap();
    assert_eq!(names(dir.path(), &inputs), ["a.md", "-"]);
    assert_eq!(inputs[0].kind(), InputKind::File);
    assert!(inputs[0].is_writable());
    assert_eq!(inputs[1].kind(), InputKind::Stdin);
    assert!(!inputs[1].is_writable());
    assert_eq!(inputs[1].path(), None);
    assert_eq!(inputs[1], Input::stdin());
}

#[test]
fn stdin_may_be_given_only_once() {
    let err = InputResolver::new().resolve(&["-", "-"]).unwrap_err();
    assert!(matches!(err, InputError::DuplicateStdin), "{err}");
}

#[test]
fn nonexistent_path_is_an_error() {
    let dir = tree(&["a.md"]);
    let err = InputResolver::new()
        .base_dir(dir.path())
        .resolve(&["a.md", "missing.md"])
        .unwrap_err();
    match err {
        InputError::NotFound { path } => assert!(path.ends_with("missing.md")),
        other => panic!("unexpected {other}"),
    }
}

#[test]
fn glob_matching_nothing_is_an_error() {
    let dir = tree(&["a.md"]);
    let err = InputResolver::new()
        .base_dir(dir.path())
        .resolve(&["docs/**/*.md"])
        .unwrap_err();
    assert!(
        matches!(&err, InputError::NoMatches { pattern } if pattern == "docs/**/*.md"),
        "{err}"
    );
    assert!(err.to_string().contains("docs/**/*.md"));
}

#[test]
fn invalid_glob_is_an_error() {
    let dir = tree(&["a.md"]);
    let err = InputResolver::new()
        .base_dir(dir.path())
        .resolve(&["docs/[z-a]*.md"])
        .unwrap_err();
    assert!(matches!(err, InputError::InvalidGlob { .. }), "{err}");
}

#[test]
fn directory_without_markdown_resolves_to_nothing() {
    let dir = tree(&["src/main.rs"]);
    assert_eq!(resolve(&dir, &["src"]), Vec::<String>::new());
}

#[test]
fn empty_arguments_resolve_to_nothing() {
    assert_eq!(
        InputResolver::new().resolve::<&str>(&[]).unwrap(),
        Vec::new()
    );
}

#[test]
fn default_base_dir_yields_paths_as_written() {
    let inputs = InputResolver::new().resolve(&["Cargo.toml"]).unwrap();
    assert_eq!(inputs[0].path(), Some(Path::new("Cargo.toml")));
}

#[test]
fn files_read_into_source_files_with_their_path() {
    let dir = tree(&[]);
    write(&dir, "a.md", "# A\n\ntext\n");
    let inputs = InputResolver::new()
        .base_dir(dir.path())
        .resolve(&["a.md"])
        .unwrap();
    let source = inputs[0].read_with(&mut Cursor::new("ignored")).unwrap();
    assert_eq!(source.path(), Some(dir.path().join("a.md").as_path()));
    assert_eq!(source.text(), "# A\n\ntext\n");
    assert_eq!(inputs[0].read().unwrap(), source);
}

#[test]
fn stdin_reads_from_the_injected_reader_without_a_path() {
    let source = Input::stdin()
        .read_with(&mut Cursor::new("# From stdin\n"))
        .unwrap();
    assert_eq!(source.path(), None);
    assert_eq!(source.text(), "# From stdin\n");
}

#[test]
fn read_errors_name_the_input() {
    let dir = tree(&[]);
    let missing = Input::file(dir.path().join("gone.md"));
    assert_eq!(missing.kind(), InputKind::File);
    let err = missing.read().unwrap_err();
    assert!(matches!(err, InputError::Read { .. }));
    assert!(err.to_string().contains("gone.md"), "{err}");

    let err = Input::stdin()
        .read_with(&mut Cursor::new(vec![0xff, 0xfe]))
        .unwrap_err();
    assert!(err.to_string().contains("<stdin>"), "{err}");
}
