//! The JSON projection (`mrq_core::ast::json`): exact output for small documents, the
//! fields of every node kind, positions, snapshots of every flavor fixture validated
//! against the checked-in schema, and serialize → deserialize → serialize round trips.

use std::fs;
use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::ast::NodeType;
use mrq_core::ast::json::{self, JsonAst, JsonKind, JsonNode};
use mrq_core::document::DocumentOptions;
use serde_json::{Value, json};

/// Environment variable that makes [`schema_file_is_up_to_date`] rewrite the file.
const UPDATE_SCHEMA: &str = "MRQ_UPDATE_SCHEMA";

fn parse(flavor: &str, src: &str) -> Document {
    DocumentOptions::new().flavor(flavor).parse(src).unwrap()
}

fn value(doc: &Document) -> Value {
    serde_json::to_value(doc.to_json_ast()).unwrap()
}

/// Every node of `tree` (pre-order), root included.
fn nodes(tree: &Value) -> Vec<&Value> {
    let mut out = vec![tree];
    if let Some(children) = tree["children"].as_array() {
        out.extend(children.iter().flat_map(nodes));
    }
    out
}

/// The nodes of JSON type `ty`, without `id`, `position` and `children`.
fn of_type(tree: &Value, ty: &str) -> Vec<Value> {
    nodes(tree)
        .into_iter()
        .filter(|node| node["type"] == ty)
        .map(|node| {
            let mut node = node.clone();
            let map = node.as_object_mut().unwrap();
            for key in ["id", "position", "children"] {
                map.remove(key);
            }
            node
        })
        .collect()
}

fn point(line: usize, column: usize, offset: usize) -> Value {
    json!({ "line": line, "column": column, "offset": offset })
}

fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/mrq-ast-1.schema.json")
}

fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(&fs::read_to_string(schema_path()).unwrap()).unwrap();
    jsonschema::validator_for(&schema).expect("schema compiles")
}

/// Fails with every schema violation of `instance`.
fn assert_valid(validator: &jsonschema::Validator, instance: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|err| format!("{}: {err}", err.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{what}:\n{}", errors.join("\n"));
}

// --- Top level and positions ----------------------------------------------------------

#[test]
fn small_document_projects_exactly() {
    let doc = parse("gfm", "# Hi\n");
    let pos = |start: (usize, usize, usize), end: (usize, usize, usize)| {
        json!({
            "start": point(start.0, start.1, start.2),
            "end": point(end.0, end.1, end.2),
        })
    };
    assert_eq!(
        value(&doc),
        json!({
            "mrq_ast": 1,
            "flavor": "gfm",
            "frontmatter": null,
            "type": "root",
            "id": 0,
            "position": pos((1, 1, 0), (2, 1, 5)),
            "children": [{
                "type": "heading",
                "depth": 1,
                "setext": false,
                "attrs": null,
                "id": 1,
                "position": pos((1, 1, 0), (1, 5, 4)),
                "children": [{
                    "type": "text",
                    "value": "Hi",
                    "id": 2,
                    "position": pos((1, 3, 2), (1, 5, 4)),
                }],
            }],
        })
    );
}

#[test]
fn version_comes_first_and_matches_the_constant() {
    let doc = parse("gfm", "x\n");
    let text = doc.to_json_ast().to_json_string();
    assert!(text.starts_with("{\n  \"mrq_ast\": 1,\n"), "{text}");
    assert!(text.ends_with("}\n"));
    assert_eq!(json::VERSION, 1);
    assert_eq!(doc.to_json_ast().mrq_ast, json::VERSION);
}

#[test]
fn frontmatter_value_is_on_the_root_and_its_block_is_a_node() {
    let doc = parse("obsidian", "---\ntitle: T\ntags: [a]\n---\n# Hi\n");
    let tree = value(&doc);
    assert_eq!(tree["flavor"], "obsidian");
    assert_eq!(tree["frontmatter"], json!({ "title": "T", "tags": ["a"] }));
    assert_eq!(
        of_type(&tree, "frontmatter"),
        [json!({ "type": "frontmatter", "format": "yaml", "value": "title: T\ntags: [a]\n" })]
    );
    let toml = value(&parse("gfm", "+++\na = 1\n+++\n"));
    assert_eq!(toml["frontmatter"], json!({ "a": 1 }));
    assert_eq!(of_type(&toml, "frontmatter")[0]["format"], "toml");
    // Malformed frontmatter keeps its node but has no value.
    let bad = value(&parse("gfm", "---\n[a\n---\nx\n"));
    assert_eq!(bad["frontmatter"], Value::Null);
    assert_eq!(of_type(&bad, "frontmatter").len(), 1);
}

#[test]
fn positions_use_utf16_columns_and_byte_offsets() {
    // `é` is 2 bytes / 1 UTF-16 unit, `😀` 4 bytes / 2 units.
    let tree = value(&parse("gfm", "é😀 *x*\n"));
    let emphasis = nodes(&tree)
        .into_iter()
        .find(|node| node["type"] == "emphasis")
        .unwrap()
        .clone();
    assert_eq!(emphasis["position"]["start"], point(1, 5, 7));
    assert_eq!(emphasis["position"]["end"], point(1, 8, 10));
    // A `\r` belongs to its line; the next line starts after the `\n`.
    let crlf = value(&parse("gfm", "a\r\n\r\nb\r\n"));
    let paragraphs: Vec<_> = nodes(&crlf)
        .into_iter()
        .filter(|node| node["type"] == "paragraph")
        .collect();
    assert_eq!(paragraphs[1]["position"]["start"], point(3, 1, 5));
    assert_eq!(crlf["position"]["end"], point(4, 1, 8));
}

#[test]
fn ids_are_node_indices_and_children_follow_the_tree() {
    for (flavor, path) in fixtures() {
        let src = fs::read_to_string(&path).unwrap();
        let doc = parse(flavor, &src);
        let tree = value(&doc);
        let ast = doc.ast();
        let projected = nodes(&tree);
        let ids: Vec<_> = ast.descendants(ast.root()).collect();
        assert_eq!(projected.len(), ids.len(), "{}", path.display());
        for (node, id) in projected.into_iter().zip(ids) {
            let ty = ast.kind(id).node_type();
            assert_eq!(node["id"], id.index(), "{}", path.display());
            assert_eq!(node["type"], json::type_name(ty), "{}", path.display());
            assert_eq!(
                node["children"].as_array().map(Vec::len),
                (json::is_parent(ty) || !ast.children(id).is_empty())
                    .then(|| ast.children(id).len()),
                "{} {ty}",
                path.display()
            );
        }
    }
}

#[test]
fn type_names_are_unique_camel_case() {
    let mut seen = std::collections::HashSet::new();
    for &ty in NodeType::ALL {
        let name = json::type_name(ty);
        assert!(seen.insert(name), "duplicate JSON name {name}");
        assert!(name.chars().all(|c| c.is_ascii_alphabetic()), "{name}");
        assert!(name.starts_with(|c: char| c.is_ascii_lowercase()), "{name}");
    }
    assert_eq!(json::type_name(NodeType::Document), "root");
    assert_eq!(json::type_name(NodeType::BlockQuote), "blockquote");
    assert_eq!(json::type_name(NodeType::Code), "inlineCode");
    assert_eq!(json::type_name(NodeType::CodeBlock), "code");
    assert_eq!(json::type_name(NodeType::WikiLink), "wikiLink");
}

#[test]
fn project_node_projects_a_subtree() {
    let doc = parse("gfm", "a\n\n> *b*\n");
    let ast = doc.ast();
    let quote = ast.children(ast.root())[1];
    let node = json::project_node(&doc, quote);
    assert_eq!(node.kind, JsonKind::Blockquote);
    assert_eq!(node.id, quote.index());
    assert_eq!(node.position.start.line, 3);
    let children = node.children.as_ref().unwrap();
    assert_eq!(children[0].kind, JsonKind::Paragraph);
}

// --- Fields of each kind ------------------------------------------------------------------

#[test]
fn gfm_block_fields() {
    let src = "\
Title
=====

- [ ] open
- [x] done

3) three

```rust ignore extra
fn main() {}
```

    indented

| a | b | c |
|:--|:-:|---|
| 1 | 2 | 3 |

[^n]: Note.

<div>
hi
</div>

***
";
    let tree = value(&parse("gfm", src));
    assert_eq!(of_type(&tree, "heading")[0]["setext"], true);
    assert_eq!(
        of_type(&tree, "list"),
        [
            json!({ "type": "list", "ordered": false, "start": null, "spread": false, "marker": "-", "delimiter": null }),
            json!({ "type": "list", "ordered": true, "start": 3, "spread": false, "marker": null, "delimiter": ")" }),
        ]
    );
    assert_eq!(
        of_type(&tree, "listItem"),
        [
            json!({ "type": "listItem", "checked": false, "taskMarker": null }),
            json!({ "type": "listItem", "checked": true, "taskMarker": null }),
            json!({ "type": "listItem", "checked": null, "taskMarker": null }),
        ]
    );
    assert_eq!(
        of_type(&tree, "code"),
        [
            json!({
                "type": "code", "lang": "rust", "meta": "ignore extra",
                "fence": { "char": "`", "length": 3 }, "value": "fn main() {}\n",
            }),
            json!({ "type": "code", "lang": null, "meta": null, "fence": null, "value": "indented\n" }),
        ]
    );
    assert_eq!(
        of_type(&tree, "table"),
        [json!({ "type": "table", "align": ["left", "center", null] })]
    );
    assert_eq!(
        of_type(&tree, "tableRow"),
        [
            json!({ "type": "tableRow", "header": true }),
            json!({ "type": "tableRow", "header": false }),
        ]
    );
    assert_eq!(
        of_type(&tree, "footnoteDefinition"),
        [json!({ "type": "footnoteDefinition", "label": "n" })]
    );
    assert_eq!(
        of_type(&tree, "html"),
        [json!({ "type": "html", "value": "<div>\nhi\n</div>\n" })]
    );
    assert_eq!(of_type(&tree, "thematicBreak").len(), 1);
}

#[test]
fn gfm_inline_fields() {
    let src = "\
*a* **b** ~~c~~ `d` <span>x</span>\\
[e](u \"t\") [f][r] [r][]
[r] <https://a.b> www.c.org ![alt &amp; x](i.png) [^n] <!-- note -->

[r]: v
[^n]: N.
";
    let tree = value(&parse("gfm", src));
    for ty in ["emphasis", "strong", "delete", "break", "softBreak"] {
        assert_eq!(of_type(&tree, ty), [json!({ "type": ty })], "{ty}");
    }
    assert_eq!(
        of_type(&tree, "inlineCode"),
        [json!({ "type": "inlineCode", "value": "d" })]
    );
    assert_eq!(
        of_type(&tree, "inlineHtml")[0],
        json!({ "type": "inlineHtml", "value": "<span>" })
    );
    let link =
        |url: &str, title: Option<&str>, form: &str, label: Option<&str>, style: Option<&str>| {
            json!({
                "type": "link", "url": url, "title": title, "form": form,
                "label": label, "referenceType": style,
            })
        };
    assert_eq!(
        of_type(&tree, "link"),
        [
            link("u", Some("t"), "inline", None, None),
            link("v", None, "reference", Some("r"), Some("full")),
            link("v", None, "reference", Some("r"), Some("collapsed")),
            link("v", None, "reference", Some("r"), Some("shortcut")),
            link("https://a.b", None, "autolink", None, None),
            link("http://www.c.org", None, "bare", None, None),
        ]
    );
    assert_eq!(
        of_type(&tree, "image"),
        [json!({
            "type": "image", "url": "i.png", "title": null, "alt": "alt & x", "form": "inline",
            "label": null, "referenceType": null,
        })]
    );
    assert_eq!(
        of_type(&tree, "footnoteReference"),
        [json!({ "type": "footnoteReference", "label": "n" })]
    );
    assert_eq!(
        of_type(&tree, "comment"),
        [json!({ "type": "comment", "syntax": "html", "value": " note " })]
    );
}

#[test]
fn obsidian_fields() {
    let src = "\
> [!tip]- Title
> body

[[a#h|x]] ![[i.png|100]] #t/u ==hi== %%c%% $x$

$$
y
$$
";
    let tree = value(&parse("obsidian", src));
    assert_eq!(
        of_type(&tree, "callout"),
        [json!({ "type": "callout", "kind": "tip", "title": "Title", "fold": "closed" })]
    );
    assert_eq!(
        of_type(&tree, "wikiLink"),
        [json!({ "type": "wikiLink", "target": "a", "anchor": "h", "alias": "x" })]
    );
    assert_eq!(
        of_type(&tree, "embed"),
        [json!({ "type": "embed", "target": "i.png", "anchor": null, "alias": "100" })]
    );
    assert_eq!(
        of_type(&tree, "tag"),
        [json!({ "type": "tag", "name": "t/u" })]
    );
    assert_eq!(
        of_type(&tree, "highlight"),
        [json!({ "type": "highlight" })]
    );
    assert_eq!(
        of_type(&tree, "comment"),
        [json!({ "type": "comment", "syntax": "percent", "value": "c" })]
    );
    assert_eq!(
        of_type(&tree, "math"),
        [
            json!({ "type": "math", "display": false, "value": "x" }),
            json!({ "type": "math", "display": true, "value": "\ny\n" }),
        ]
    );
}

#[test]
fn mkdocs_fields() {
    let src = "\
## Section {: #sec .wide data-x=\"1 2\" }

??? note inline \"Title\"
    Body

Term
:   Details

*[HTML]: Hyper Text
";
    let tree = value(&parse("mkdocs", src));
    let heading = &of_type(&tree, "heading")[0];
    assert_eq!(heading["depth"], 2);
    let attrs = &heading["attrs"];
    assert_eq!(attrs["id"], "sec");
    assert_eq!(attrs["classes"], json!(["wide"]));
    assert_eq!(
        attrs["attributes"],
        json!([{ "name": "data-x", "value": "1 2" }])
    );
    assert_eq!(attrs["position"]["start"], point(1, 12, 11));
    assert_eq!(attrs["position"]["end"], point(1, 40, 39));
    assert_eq!(
        of_type(&tree, "admonition"),
        [json!({
            "type": "admonition", "kind": "note", "classes": ["inline"], "title": "Title",
            "fold": "closed",
        })]
    );
    for ty in [
        "definitionList",
        "definitionItem",
        "definitionTerm",
        "definitionDetails",
    ] {
        assert_eq!(of_type(&tree, ty), [json!({ "type": ty })], "{ty}");
    }
    assert_eq!(
        of_type(&tree, "abbreviationDefinition"),
        [json!({ "type": "abbreviationDefinition", "abbr": "HTML", "expansion": "Hyper Text" })]
    );
}

// --- Fixtures, schema and round trips -------------------------------------------------

/// `(flavor, path)` of every fixture: `parse` (gfm), `obsidian` and `mkdocs`.
fn fixtures() -> Vec<(&'static str, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out = Vec::new();
    for (dir, flavor) in [
        ("parse", "gfm"),
        ("obsidian", "obsidian"),
        ("mkdocs", "mkdocs"),
    ] {
        let mut paths: Vec<_> = fs::read_dir(root.join(dir))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        paths.sort();
        assert!(!paths.is_empty(), "no fixtures in {dir}");
        out.extend(paths.into_iter().map(|path| (flavor, path)));
    }
    out
}

#[test]
fn fixture_snapshots_validate_and_round_trip() {
    let validator = validator();
    for (flavor, path) in fixtures() {
        let src = fs::read_to_string(&path).unwrap();
        let projection = parse(flavor, &src).to_json_ast();
        let text = projection.to_json_string();
        let what = path.display().to_string();
        assert_valid(&validator, &serde_json::from_str(&text).unwrap(), &what);
        let back: JsonAst = serde_json::from_str(&text).unwrap();
        assert_eq!(back, projection, "{what}");
        assert_eq!(back.to_json_string(), text, "{what}");
        let dir = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy();
        let stem = path.file_stem().unwrap().to_string_lossy();
        insta::assert_snapshot!(format!("{dir}__{stem}"), text);
    }
}

#[test]
fn schema_file_is_up_to_date() {
    let generated = json::schema_json();
    let path = schema_path();
    if std::env::var_os(UPDATE_SCHEMA).is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &generated).unwrap();
    }
    let checked_in = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        checked_in == generated,
        "{} is stale; regenerate it with `just schema`",
        path.display()
    );
}

#[test]
fn schema_rejects_wrong_versions_and_unknown_types() {
    let validator = validator();
    let good = value(&parse("gfm", "# Hi *there*\n"));
    assert_valid(&validator, &good, "good");
    let mut wrong_version = good.clone();
    wrong_version["mrq_ast"] = json!(2);
    assert!(!validator.is_valid(&wrong_version));
    let mut unknown_type = good.clone();
    unknown_type["children"][0]["type"] = json!("nope");
    assert!(!validator.is_valid(&unknown_type));
    // Optional fields are `null`, never left out.
    for field in ["depth", "attrs", "id", "position"] {
        let mut missing = good.clone();
        missing["children"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(!validator.is_valid(&missing), "{field}");
    }
    let mut no_frontmatter = good;
    no_frontmatter
        .as_object_mut()
        .unwrap()
        .remove("frontmatter");
    assert!(!validator.is_valid(&no_frontmatter));
}

#[test]
fn deserialized_nodes_are_typed() {
    let projection = parse("obsidian", "[[a]]\n").to_json_ast();
    let text = serde_json::to_string(&projection).unwrap();
    let back: JsonAst = serde_json::from_str(&text).unwrap();
    assert_eq!(back.flavor, "obsidian");
    assert_eq!(back.root.kind, JsonKind::Root);
    let paragraph: &JsonNode = &back.root.children.as_ref().unwrap()[0];
    let link = &paragraph.children.as_ref().unwrap()[0];
    assert!(matches!(&link.kind, JsonKind::WikiLink(w) if w.target == "a"));
    assert!(link.children.is_none());
}
