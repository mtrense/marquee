//! The md backend: `fmt` output, optionally down-converted to another flavor. The
//! fixtures of `tests/fixtures/md/<flavor>/` are converted to each target (snapshots),
//! and the converted files, re-parsed with the target flavor in a workspace of their
//! own, pass `validate` without errors and are stable under `fmt`.

use std::fs;
use std::path::Path;

use mrq_core::config::FmtConfig;
use mrq_core::diag::Severity;
use mrq_core::document::DocumentOptions;
use mrq_core::flavor::{Flavor, Registry};
use mrq_core::validate::Checker;
use mrq_core::{Document, Workspace, fmt};
use mrq_render::md::MdRenderer;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/md");

/// Files the fixtures embed, which the converted workspace needs too.
const ASSETS: &[&str] = &["cat.png"];

/// Converts every Markdown file of the fixture directory `dir` (flavor `from`) to `to`,
/// snapshots each, writes them to a temporary workspace and validates it there.
fn convert_fixtures(dir: &str, from: &str, to: &str) {
    let source = Path::new(FIXTURES).join(dir);
    let target = Registry::builtin().get(to).unwrap();
    let renderer = MdRenderer::new(FmtConfig::default()).with_target(target);
    let out = tempfile::tempdir().unwrap();
    let mut names: Vec<String> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "md"))
        .collect();
    names.sort();
    for name in &names {
        let doc = DocumentOptions::new()
            .flavor(from)
            .load(source.join(name))
            .unwrap();
        let output = renderer.render_with_diagnostics(&doc);
        assert!(
            output.diagnostics.is_empty(),
            "{name}: {:?}",
            output.diagnostics
        );
        let snapshot = format!("{dir}_to_{to}__{}", name.replace(' ', "_"));
        insta::assert_snapshot!(snapshot, output.markdown);
        fs::write(out.path().join(name), &output.markdown).unwrap();
    }
    for asset in ASSETS {
        fs::write(out.path().join(asset), b"").unwrap();
    }

    let options = DocumentOptions::new().flavor(to);
    let mut workspace = Workspace::with_options(out.path(), options.clone());
    for name in &names {
        let path = out.path().join(name);
        let converted = fs::read_to_string(&path).unwrap();
        let doc = workspace.load(&path).unwrap();
        assert_eq!(doc.flavor().name(), to, "{name}");
        // Converted output is canonical for the target: `fmt` leaves it as it is.
        assert_eq!(fmt::format(doc, &FmtConfig::default()), converted, "{name}");
    }
    let diagnostics = Checker::new(&workspace).check_workspace();
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{dir} as {to}: {errors:#?}");
}

#[test]
fn obsidian_converts_to_commonmark_that_validates() {
    convert_fixtures("obsidian", "obsidian", "commonmark");
}

#[test]
fn obsidian_converts_to_gfm_that_validates() {
    convert_fixtures("obsidian", "obsidian", "gfm");
}

#[test]
fn mkdocs_converts_to_gfm_that_validates() {
    convert_fixtures("mkdocs", "mkdocs", "gfm");
}

#[test]
fn without_a_target_the_output_is_fmt_output() {
    let src = "Title\n=====\n\n* [[Note]] #tag\n\n> [!tip] Hint\n> body\n";
    let doc = DocumentOptions::new()
        .flavor("obsidian")
        .parse(src)
        .unwrap();
    let renderer = MdRenderer::new(FmtConfig::default());
    assert_eq!(
        renderer.render(&doc),
        fmt::format(&doc, &FmtConfig::default())
    );
    let same = MdRenderer::new(FmtConfig::default())
        .with_target(Registry::builtin().get("obsidian").unwrap());
    assert_eq!(same.render(&doc), renderer.render(&doc));
}

#[test]
fn the_fmt_config_applies() {
    let config = FmtConfig {
        list_marker: mrq_core::config::ListMarker::Star,
        ..FmtConfig::default()
    };
    let doc = Document::parse("- a\n- b\n");
    assert_eq!(MdRenderer::new(config).render(&doc), "* a\n* b\n");
}

#[test]
fn the_target_is_reported() {
    let renderer = MdRenderer::new(FmtConfig::default());
    assert!(renderer.target().is_none());
    let renderer = renderer.with_target(Registry::builtin().get("commonmark").unwrap());
    assert_eq!(renderer.target().map(Flavor::name), Some("commonmark"));
}
