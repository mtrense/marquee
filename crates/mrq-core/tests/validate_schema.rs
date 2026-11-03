//! Acceptance tests for frontmatter JSON Schema validation (`validate.frontmatter`).
#![cfg(feature = "frontmatter-schema")]

use std::fs;
use std::path::PathBuf;

use mrq_core::Slashed as _;
use mrq_core::config::ConfigLoader;
use mrq_core::diag::Severity;
use mrq_core::document::{Document, DocumentOptions, Workspace};
use mrq_core::source::SourceFile;
use mrq_core::validate::{Checker, FrontmatterSchemas, SchemaError, codes};
use serde_json::{Value, json};

/// `text` parsed as `path` (gfm).
fn doc_at(path: &str, text: &str) -> Document {
    let source = SourceFile::new(Some(PathBuf::from(path)), text);
    DocumentOptions::new().parse_source(source).unwrap()
}

/// Schemas with `schema` applied to `site/blog/**/*.md`.
fn blog(schema: &Value) -> FrontmatterSchemas {
    let mut schemas = FrontmatterSchemas::new();
    schemas.add("site", "blog/**/*.md", schema).unwrap();
    schemas
}

/// (code, message, source of the primary span) of every diagnostic for `doc`.
fn check(schemas: &FrontmatterSchemas, doc: &Document) -> Vec<(String, String, String)> {
    let ws = Workspace::new("site");
    Checker::new(&ws)
        .with_frontmatter_schemas(schemas)
        .check(doc)
        .into_iter()
        .map(|diag| {
            assert_eq!(diag.severity, Severity::Error);
            let source = doc.text()[diag.range.start..diag.range.end].to_owned();
            (diag.code, diag.message, source)
        })
        .collect()
}

fn found(message: &str, source: &str) -> (String, String, String) {
    (
        codes::FRONTMATTER_SCHEMA.to_owned(),
        message.to_owned(),
        source.to_owned(),
    )
}

#[test]
fn wrong_type_points_at_the_value() {
    let schemas = blog(&json!({
        "properties": { "title": { "type": "string" } }
    }));
    let doc = doc_at("site/blog/a.md", "---\ntitle: 42\n---\n# A\n");
    assert_eq!(
        check(&schemas, &doc),
        [found("`title`: expected string, found integer", "42")]
    );
}

#[test]
fn enums_and_consts_list_the_expected_values() {
    let schemas = blog(&json!({
        "properties": {
            "status": { "enum": ["draft", "published"] },
            "layout": { "const": "post" },
            "tags": { "type": "array", "items": { "type": ["string", "null"] } }
        }
    }));
    let doc = doc_at(
        "site/blog/a.md",
        "---\nstatus: wip\nlayout: page\ntags: [a, 1]\n---\n",
    );
    assert_eq!(
        check(&schemas, &doc),
        [
            found(
                "`status`: expected one of \"draft\", \"published\", found \"wip\"",
                "wip"
            ),
            found("`layout`: expected \"post\", found \"page\"", "page"),
            found("`tags.1`: expected null or string, found integer", "1"),
        ]
    );
}

#[test]
fn missing_required_keys_point_at_the_parent() {
    let schemas = blog(&json!({
        "required": ["title", "date"],
        "properties": {
            "author": { "type": "object", "required": ["name"] }
        }
    }));
    let doc = doc_at(
        "site/blog/a.md",
        "---\ndate: 2024-01-01\nauthor:\n  mail: a@b.org\n---\n",
    );
    assert_eq!(
        check(&schemas, &doc),
        [
            found("missing required key `title`", "---"),
            found("`author`: missing required key `name`", "author"),
        ]
    );
}

#[test]
fn only_documents_matching_the_glob_are_checked() {
    let schemas = blog(&json!({ "required": ["title"] }));
    let source = SourceFile::new(None, "# stdin\n");
    let stdin = DocumentOptions::new().parse_source(source).unwrap();
    for doc in [
        doc_at("site/a.md", "# A\n"),
        doc_at("other/blog/a.md", "# A\n"),
        doc_at("site/blog/a.txt", "# A\n"),
        stdin,
    ] {
        assert_eq!(check(&schemas, &doc), [], "{}", doc.name().display());
    }
    let nested = doc_at("site/blog/2024/./x/a.md", "# A\n");
    assert_eq!(check(&schemas, &nested).len(), 1);
}

#[test]
fn an_empty_base_is_the_current_directory() {
    let mut schemas = FrontmatterSchemas::new();
    schemas
        .add("", "site/*/a.md", &json!({ "required": ["title"] }))
        .unwrap();
    assert_eq!(check(&schemas, &doc_at("site/blog/a.md", "# A\n")).len(), 1);
}

#[test]
fn documents_without_frontmatter_are_an_empty_mapping() {
    let schemas = blog(&json!({ "required": ["title"] }));
    let doc = doc_at("site/blog/a.md", "# A\n");
    assert_eq!(
        check(&schemas, &doc),
        [found("missing required key `title`", "")]
    );
}

#[test]
fn malformed_frontmatter_is_left_to_the_parser() {
    let schemas = blog(&json!({ "required": ["title"] }));
    let doc = doc_at("site/blog/a.md", "---\ntitle: [\n---\n");
    assert_eq!(check(&schemas, &doc), []);
}

#[test]
fn toml_frontmatter_spans() {
    let schemas = blog(&json!({
        "required": ["title"],
        "properties": { "draft": { "type": "boolean" } }
    }));
    let doc = doc_at("site/blog/a.md", "+++\ndraft = \"yes\"\n+++\n");
    assert_eq!(
        check(&schemas, &doc),
        [
            found("missing required key `title`", "+++"),
            found("`draft`: expected boolean, found string", "\"yes\""),
        ]
    );
}

#[test]
fn drafts_7_and_2020_12() {
    // `items` as an array is a tuple in draft 7; 2020-12 uses `prefixItems`.
    let draft7 = blog(&json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "properties": { "pair": { "items": [{ "type": "string" }, { "type": "integer" }] } }
    }));
    let draft2020 = blog(&json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "properties": { "pair": { "prefixItems": [{ "type": "string" }, { "type": "integer" }] } }
    }));
    let doc = doc_at("site/blog/a.md", "---\npair: [a, b]\n---\n");
    for schemas in [draft7, draft2020] {
        assert_eq!(
            check(&schemas, &doc),
            [found("`pair.1`: expected integer, found string", "b")]
        );
    }
}

#[test]
fn every_matching_schema_applies() {
    let mut schemas = blog(&json!({ "required": ["title"] }));
    schemas
        .add("site", "**/a.md", &json!({ "required": ["date"] }))
        .unwrap();
    let doc = doc_at("site/blog/a.md", "---\nx: 1\n---\n");
    assert_eq!(
        check(&schemas, &doc),
        [
            found("missing required key `title`", "---"),
            found("missing required key `date`", "---"),
        ]
    );
}

#[test]
fn invalid_schemas_and_globs_are_errors() {
    let mut schemas = FrontmatterSchemas::new();
    let err = schemas
        .add("site", "*.md", &json!({ "type": "strin" }))
        .unwrap_err();
    assert!(matches!(err, SchemaError::Invalid { .. }), "{err}");
    let err = schemas.add("site", "a[", &json!({})).unwrap_err();
    assert!(matches!(err, SchemaError::Glob { .. }), "{err}");
    // No network: a remote `$ref` cannot be resolved.
    let err = schemas
        .add(
            "site",
            "*.md",
            &json!({ "$ref": "https://example.com/s.json" }),
        )
        .unwrap_err();
    assert!(matches!(err, SchemaError::Invalid { .. }), "{err}");
    assert!(schemas.is_empty());
}

#[test]
fn unexpected_keys_point_at_the_key() {
    let schemas = blog(&json!({
        "properties": { "title": { "type": "string" } },
        "additionalProperties": false
    }));
    let doc = doc_at("site/blog/a.md", "---\ntitle: A\ntitel: B\nextra: 1\n---\n");
    assert_eq!(
        check(&schemas, &doc),
        [
            found("unexpected key `titel`", "titel"),
            found("unexpected key `extra`", "extra"),
        ]
    );
}

/// A temp directory with `.mrq.yaml` (`config`) at its root and a `sub/dir` to start
/// config discovery from; returns the directory guard and its canonical path.
fn tree(config: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("sub/dir")).unwrap();
    fs::write(root.join(".mrq.yaml"), config).unwrap();
    for (path, text) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    (dir, root)
}

#[test]
fn config_paths_resolve_against_the_declaring_config_file() {
    let (_dir, root) = tree(
        "validate:\n  frontmatter:\n    - files: \"blog/**/*.md\"\n      schema: schemas/post.json\n    \
         - files: \"blog/**/*.md\"\n      schema: ./schemas/extra.yaml\n",
        &[
            ("schemas/post.json", r#"{ "required": ["title"] }"#),
            (
                "schemas/extra.yaml",
                "properties:\n  draft: { type: boolean }\n",
            ),
        ],
    );
    let loaded = ConfigLoader::new(root.join("sub/dir")).load().unwrap();
    let schemas = FrontmatterSchemas::from_config(&loaded).unwrap();

    let path = root.join("blog/a.md");
    let doc = doc_at(path.to_str().unwrap(), "---\ndraft: 1\n---\n");
    let ws = Workspace::new(&root);
    let diagnostics = Checker::new(&ws)
        .with_frontmatter_schemas(&schemas)
        .check(&doc);
    let summary: Vec<_> = diagnostics
        .iter()
        .map(|diag| (diag.message.as_str(), diag.help.clone().unwrap()))
        .collect();
    let help = |rel: &str| format!("schema `{}`", root.join(rel).slashed());
    assert_eq!(
        summary,
        [
            ("missing required key `title`", help("schemas/post.json")),
            (
                "`draft`: expected boolean, found integer",
                help("schemas/extra.yaml")
            ),
        ]
    );
    // Relative to the config file, not to the start directory.
    let elsewhere = doc_at(root.join("sub/dir/blog/a.md").to_str().unwrap(), "# A\n");
    assert_eq!(schemas.check(&elsewhere), []);
}

#[test]
fn unreadable_and_unparsable_schema_files() {
    let config = |schema: &str| {
        format!("validate:\n  frontmatter:\n    - files: \"*.md\"\n      schema: {schema}\n")
    };
    let (_dir, root) = tree(&config("missing.json"), &[]);
    let loaded = ConfigLoader::new(&root).load().unwrap();
    let err = FrontmatterSchemas::from_config(&loaded).unwrap_err();
    assert!(
        matches!(&err, SchemaError::Read { path, .. } if *path == root.join("missing.json")),
        "{err}"
    );

    let (_dir, root) = tree(&config("bad.json"), &[("bad.json", "{ nope")]);
    let loaded = ConfigLoader::new(&root).load().unwrap();
    let err = FrontmatterSchemas::from_config(&loaded).unwrap_err();
    assert!(matches!(&err, SchemaError::Parse { .. }), "{err}");

    let (_dir, root) = tree(&config("bad.json"), &[("bad.json", r#"{ "type": 1 }"#)]);
    let loaded = ConfigLoader::new(&root).load().unwrap();
    let err = FrontmatterSchemas::from_config(&loaded).unwrap_err();
    assert!(
        matches!(&err, SchemaError::Invalid { path, .. } if *path == Some(root.join("bad.json"))),
        "{err}"
    );
}
