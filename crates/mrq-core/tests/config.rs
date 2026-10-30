//! Layered configuration loading: defaults, user config, ancestor `.mrq.yaml`
//! files, an explicit `--config` file and CLI overrides, plus provenance in errors
//! and in `config show --origin`.

use std::fs;
use std::path::{Path, PathBuf};

use mrq_core::config::{
    Config, ConfigError, ConfigLoader, FlavorName, LintExtends, OrderedList, Origin, RuleSetting,
    Wrap, user_cache_dir, user_config_path,
};

/// A temp tree: `<root>/user/config.yaml`, `<root>/a/.mrq.yaml`, `<root>/a/b/.mrq.yaml`
/// and the start directory `<root>/a/b/c`.
struct Tree {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("user")).unwrap();
        fs::create_dir_all(root.join("a/b/c")).unwrap();
        Self { _dir: dir, root }
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::write(&path, text).unwrap();
        path
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn loader(&self) -> ConfigLoader {
        ConfigLoader::new(self.path("a/b/c")).user_config(Some(self.path("user/config.yaml")))
    }
}

#[test]
fn defaults_match_the_readme() {
    let config = Config::default();
    assert_eq!(config.flavor, FlavorName::Gfm);
    assert_eq!(config.ignore, Vec::<String>::new());
    assert_eq!(config.lint.extends, LintExtends::Default);
    assert!(config.lint.rules.is_empty());
    assert_eq!(config.fmt.wrap, Wrap::Preserve);
    assert_eq!(config.fmt.width, 80);
    assert_eq!(config.fmt.emphasis.as_str(), "*");
    assert_eq!(config.fmt.strong.as_str(), "**");
    assert_eq!(config.fmt.list_marker.as_str(), "-");
    assert_eq!(config.fmt.ordered_list, OrderedList::Ordered);
    assert_eq!(config.fmt.code_fence.as_str(), "```");
    assert!(!config.validate.external);
    assert!(!config.include.allow_remote);
    assert!(!config.render.diagrams.kroki.enabled);
    assert_eq!(config.render.diagrams.kroki.url, "https://kroki.io");
}

#[test]
fn no_files_yields_defaults() {
    let tree = Tree::new();
    let loaded = tree.loader().load().unwrap();
    assert_eq!(loaded.config(), &Config::default());
    assert_eq!(loaded.files(), Vec::<PathBuf>::new());
}

#[test]
fn readme_example_parses() {
    let tree = Tree::new();
    tree.write(
        "a/.mrq.yaml",
        r#"flavor: obsidian
ignore: ["vendor/**", "CHANGELOG.md"]
lint:
  extends: none
  rules:
    MD012: { maximum: 2 }
    no-multiple-blanks: { maximum: 3 }
    MD047: off
fmt:
  wrap: width
  width: 100
  emphasis: "_"
  strong: "__"
  list_marker: "*"
  ordered_list: one
  code_fence: "~~~"
  code_formatters:
    rust: ["rustfmt", "--emit=stdout"]
validate:
  external: true
  external_exclude: ["^https://localhost"]
  frontmatter:
    - files: "blog/**/*.md"
      schema: ./schemas/post.schema.json
include:
  allow_remote: true
render:
  style: github
  diagrams:
    kroki: { enabled: true, url: "https://kroki.example" }
    commands:
      bob: ["svgbob"]
"#,
    );
    let config = tree.loader().load().unwrap().config().clone();
    assert_eq!(config.flavor, FlavorName::Obsidian);
    assert_eq!(config.ignore, ["vendor/**", "CHANGELOG.md"]);
    assert_eq!(config.lint.extends, LintExtends::None);
    assert_eq!(config.lint.rules.len(), 3);
    assert_eq!(
        config.lint.rules.get("MD012"),
        Some(&RuleSetting::Options(
            serde_json::json!({ "maximum": 2 })
                .as_object()
                .unwrap()
                .clone()
        ))
    );
    assert_eq!(
        config.lint.rules.get("MD047"),
        Some(&RuleSetting::Enabled(false))
    );
    assert_eq!(config.fmt.wrap, Wrap::Width);
    assert_eq!(config.fmt.width, 100);
    assert_eq!(config.fmt.emphasis.as_str(), "_");
    assert_eq!(config.fmt.strong.as_str(), "__");
    assert_eq!(config.fmt.list_marker.as_str(), "*");
    assert_eq!(config.fmt.ordered_list, OrderedList::One);
    assert_eq!(config.fmt.code_fence.as_str(), "~~~");
    assert_eq!(
        config.fmt.code_formatters["rust"],
        ["rustfmt", "--emit=stdout"]
    );
    assert!(config.validate.external);
    assert_eq!(config.validate.external_exclude, ["^https://localhost"]);
    assert_eq!(config.validate.frontmatter[0].files, "blog/**/*.md");
    assert_eq!(
        config.validate.frontmatter[0].schema,
        Path::new("./schemas/post.schema.json")
    );
    assert!(config.include.allow_remote);
    assert_eq!(config.render.style.as_deref(), Some("github"));
    assert!(config.render.diagrams.kroki.enabled);
    assert_eq!(config.render.diagrams.kroki.url, "https://kroki.example");
    assert_eq!(config.render.diagrams.commands["bob"], ["svgbob"]);
}

#[test]
fn layers_user_then_far_then_near_then_cli() {
    let tree = Tree::new();
    tree.write(
        "user/config.yaml",
        "flavor: mkdocs\nfmt:\n  width: 60\n  wrap: never\n  emphasis: _\n",
    );
    tree.write(
        "a/.mrq.yaml",
        "flavor: obsidian\nfmt:\n  width: 70\n  wrap: sentence\n",
    );
    tree.write("a/b/.mrq.yaml", "flavor: commonmark\nfmt:\n  width: 90\n");

    let loaded = tree.loader().load().unwrap();
    let config = loaded.config();
    // Key by key: each value comes from the highest layer that sets it.
    assert_eq!(config.flavor, FlavorName::Commonmark);
    assert_eq!(config.fmt.width, 90);
    assert_eq!(config.fmt.wrap, Wrap::Sentence);
    assert_eq!(config.fmt.emphasis.as_str(), "_");
    assert_eq!(config.fmt.list_marker.as_str(), "-");
    assert_eq!(
        loaded.files(),
        [
            tree.path("user/config.yaml"),
            tree.path("a/.mrq.yaml"),
            tree.path("a/b/.mrq.yaml")
        ]
    );

    let loaded = tree.loader().flavor(Some(FlavorName::Gfm)).load().unwrap();
    assert_eq!(loaded.config().flavor, FlavorName::Gfm);
    assert_eq!(loaded.config().fmt.width, 90);
}

#[test]
fn yml_extension_is_discovered_but_yaml_wins_in_the_same_directory() {
    let tree = Tree::new();
    tree.write("a/.mrq.yml", "flavor: obsidian\n");
    assert_eq!(
        tree.loader().load().unwrap().config().flavor,
        FlavorName::Obsidian
    );
    tree.write("a/.mrq.yaml", "flavor: mkdocs\n");
    let loaded = tree.loader().load().unwrap();
    assert_eq!(loaded.config().flavor, FlavorName::Mkdocs);
    assert_eq!(loaded.files(), [tree.path("a/.mrq.yaml")]);
}

#[test]
fn explicit_config_replaces_discovery_but_keeps_user_config() {
    let tree = Tree::new();
    tree.write("user/config.yaml", "fmt:\n  width: 60\n");
    tree.write("a/b/.mrq.yaml", "flavor: commonmark\nfmt:\n  wrap: never\n");
    let explicit = tree.write("explicit.yaml", "flavor: mkdocs\n");

    let loaded = tree
        .loader()
        .explicit(Some(explicit.clone()))
        .load()
        .unwrap();
    assert_eq!(loaded.config().flavor, FlavorName::Mkdocs);
    assert_eq!(loaded.config().fmt.width, 60);
    assert_eq!(loaded.config().fmt.wrap, Wrap::Preserve);
    assert_eq!(loaded.files(), [tree.path("user/config.yaml"), explicit]);
}

#[test]
fn missing_explicit_config_is_an_io_error() {
    let tree = Tree::new();
    let err = tree
        .loader()
        .explicit(Some(tree.path("nope.yaml")))
        .load()
        .unwrap_err();
    assert!(matches!(err, ConfigError::Io { .. }), "{err:?}");
    assert!(err.to_string().contains("nope.yaml"), "{err}");
}

#[test]
fn missing_user_config_is_ignored() {
    let tree = Tree::new();
    let loaded = ConfigLoader::new(tree.path("a/b/c"))
        .user_config(Some(tree.path("does/not/exist.yaml")))
        .load()
        .unwrap();
    assert_eq!(loaded.config(), &Config::default());
}

#[test]
fn empty_and_comment_only_files_are_allowed() {
    let tree = Tree::new();
    tree.write("a/.mrq.yaml", "");
    tree.write("a/b/.mrq.yaml", "# nothing here\n");
    assert_eq!(tree.loader().load().unwrap().config(), &Config::default());
}

#[test]
fn unknown_key_names_file_and_line() {
    let tree = Tree::new();
    tree.write("a/.mrq.yaml", "flavor: gfm\n");
    let near = tree.write(
        "a/b/.mrq.yaml",
        "flavor: gfm\nfmt:\n  width: 80\n  wrapp: never\n",
    );
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(location.path, near);
    assert_eq!(location.line, 4);
    assert_eq!(location.column, 3);
    let message = err.to_string();
    assert!(
        message.contains(&format!("{}:4:3", near.display())),
        "{message}"
    );
    assert!(
        message.contains("unknown key `wrapp` in `fmt`; expected one of `wrap`"),
        "{message}"
    );
}

#[test]
fn unknown_top_level_key_in_user_config_names_it() {
    let tree = Tree::new();
    let user = tree.write("user/config.yaml", "\n\nflavour: gfm\n");
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(location.path, user);
    assert_eq!(location.line, 3);
    assert!(err.to_string().contains("flavour"), "{err}");
}

#[test]
fn unknown_key_is_attributed_to_the_file_that_has_it() {
    let tree = Tree::new();
    let far = tree.write("a/.mrq.yaml", "lint:\n  bogus: 1\n");
    tree.write("a/b/.mrq.yaml", "lint:\n  extends: none\n");
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(location.path, far);
    assert_eq!(location.line, 2);
}

#[test]
fn invalid_value_names_file_and_line() {
    let tree = Tree::new();
    let near = tree.write("a/b/.mrq.yaml", "fmt:\n  wrap: sometimes\n");
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(location.path, near);
    assert_eq!(location.line, 2);
    assert!(err.to_string().contains("sometimes"), "{err}");
}

#[test]
fn invalid_value_in_a_list_names_the_item_line() {
    let tree = Tree::new();
    let near = tree.write(
        "a/b/.mrq.yaml",
        "validate:\n  frontmatter:\n    - files: a\n      schema: b\n    - files: c\n",
    );
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(location.path, near);
    assert_eq!(location.line, 5);
    assert!(
        err.to_string()
            .contains("missing key `schema` in `validate.frontmatter.1`"),
        "{err}"
    );
}

#[test]
fn yaml_syntax_error_names_file_and_line() {
    let tree = Tree::new();
    let near = tree.write("a/b/.mrq.yaml", "fmt:\n  width: 80\n wrap: [never\n");
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(location.path, near);
    assert!(location.line >= 2, "{location:?}");
}

#[test]
fn top_level_must_be_a_mapping() {
    let tree = Tree::new();
    let near = tree.write("a/b/.mrq.yaml", "- a\n- b\n");
    let err = tree.loader().load().unwrap_err();
    assert_eq!(err.location().expect("location").path, near);
}

#[test]
fn origins_name_file_and_line_or_default_or_cli() {
    let tree = Tree::new();
    let user = tree.write("user/config.yaml", "fmt:\n  width: 60\n");
    let near = tree.write(
        "a/b/.mrq.yaml",
        "flavor: obsidian\nignore:\n  - vendor/**\nfmt:\n  wrap: never\n",
    );
    let loaded = tree.loader().load().unwrap();
    assert_eq!(
        loaded.origin(&["flavor"]),
        Some(Origin::File {
            path: near.clone(),
            line: 1
        })
    );
    assert_eq!(
        loaded.origin(&["fmt", "wrap"]),
        Some(Origin::File {
            path: near.clone(),
            line: 5
        })
    );
    assert_eq!(
        loaded.origin(&["fmt", "width"]),
        Some(Origin::File {
            path: user,
            line: 2
        })
    );
    assert_eq!(
        loaded.origin(&["ignore", "0"]),
        Some(Origin::File {
            path: near,
            line: 3
        })
    );
    assert_eq!(loaded.origin(&["fmt", "emphasis"]), Some(Origin::Default));
    assert_eq!(loaded.origin(&["nope"]), None);

    let loaded = tree.loader().flavor(Some(FlavorName::Gfm)).load().unwrap();
    assert_eq!(loaded.origin(&["flavor"]), Some(Origin::Cli));
}

#[test]
fn user_config_path_prefers_mrq_config_home_then_xdg_then_home() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    };
    assert_eq!(
        user_config_path(env(&[("MRQ_CONFIG_HOME", "/m"), ("XDG_CONFIG_HOME", "/x")])),
        Some(PathBuf::from("/m/config.yaml"))
    );
    assert_eq!(
        user_config_path(env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")])),
        Some(PathBuf::from("/x/mrq/config.yaml"))
    );
    assert_eq!(
        user_config_path(env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")])),
        Some(PathBuf::from("/h/.config/mrq/config.yaml"))
    );
    assert_eq!(user_config_path(env(&[])), None);
}

#[test]
fn user_cache_dir_prefers_mrq_cache_home_then_xdg_then_home() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    };
    assert_eq!(
        user_cache_dir(env(&[("MRQ_CACHE_HOME", "/m"), ("XDG_CACHE_HOME", "/x")])),
        Some(PathBuf::from("/m"))
    );
    assert_eq!(
        user_cache_dir(env(&[("XDG_CACHE_HOME", "/x"), ("HOME", "/h")])),
        Some(PathBuf::from("/x/mrq"))
    );
    assert_eq!(
        user_cache_dir(env(&[("XDG_CACHE_HOME", ""), ("USERPROFILE", "/u")])),
        Some(PathBuf::from("/u/.cache/mrq"))
    );
    assert_eq!(user_cache_dir(env(&[])), None);
}

#[test]
fn schema_snapshot() {
    insta::assert_snapshot!(mrq_core::config::schema_json());
}

#[test]
fn schema_rejects_unknown_keys_and_accepts_the_readme_example() {
    let schema: serde_json::Value = serde_json::from_str(&mrq_core::config::schema_json()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let good = serde_json::json!({
        "flavor": "gfm",
        "lint": { "rules": { "MD012": { "maximum": 2 }, "MD047": "off" } },
        "fmt": { "wrap": "width", "width": 100, "emphasis": "_" },
        "render": { "diagrams": { "kroki": { "enabled": true } } }
    });
    assert!(validator.is_valid(&good));
    assert!(!validator.is_valid(&serde_json::json!({ "fmt": { "wrapp": "never" } })));
    assert!(!validator.is_valid(&serde_json::json!({ "flavor": "markdown-it" })));
}

#[test]
fn unknown_lint_rule_names_file_and_line() {
    let tree = Tree::new();
    let near = tree.write(
        "a/b/.mrq.yaml",
        "lint:\n  rules:\n    MD012: true\n    MD31: off\n",
    );
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(
        (location.path.as_path(), location.line),
        (near.as_path(), 4)
    );
    assert!(
        err.to_string().contains("unknown lint rule `MD31`"),
        "{err}"
    );
}

#[test]
fn invalid_lint_rule_options_name_file_and_line() {
    let tree = Tree::new();
    let near = tree.write(
        "a/b/.mrq.yaml",
        "lint:\n  rules:\n    MD012:\n      maximun: 2\n",
    );
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(location.path, near);
    assert!(location.line >= 3, "{location:?}");
    assert!(err.to_string().contains("maximun"), "{err}");
}

#[test]
fn lint_rule_options_merge_across_layers() {
    let tree = Tree::new();
    tree.write("user/config.yaml", "lint:\n  rules:\n    MD047: off\n");
    tree.write(
        "a/.mrq.yaml",
        "lint:\n  rules:\n    MD012: { maximum: 2 }\n",
    );
    let config = tree.loader().load().unwrap().config().clone();
    assert_eq!(
        config.lint.rules.get("MD047"),
        Some(&RuleSetting::Enabled(false))
    );
    assert!(matches!(
        config.lint.rules.get("MD012"),
        Some(RuleSetting::Options(_))
    ));
}

fn rule(config: &Config, key: &str) -> Option<RuleSetting> {
    config.lint.rules.get(key).cloned()
}

fn options(json: &serde_json::Value) -> RuleSetting {
    RuleSetting::Options(json.as_object().unwrap().clone())
}

#[test]
fn markdownlint_json_is_the_lint_fallback() {
    let tree = Tree::new();
    let file = tree.write(
        "a/.markdownlint.json",
        "{\n  \"$schema\": \"https://example.com/schema.json\",\n  \"default\": false,\n  \"MD012\": { \"maximum\": 2 },\n  \"line-length\": false,\n  \"single-trailing-newline\": true\n}\n",
    );
    let loaded = tree.loader().load().unwrap();
    let config = loaded.config();
    assert_eq!(config.lint.extends, LintExtends::None);
    assert_eq!(
        rule(config, "MD012"),
        Some(options(&serde_json::json!({ "maximum": 2 })))
    );
    assert_eq!(
        rule(config, "single-trailing-newline"),
        Some(RuleSetting::Enabled(true))
    );
    // Rules mrq does not know (yet, or markdownlint plugins) are skipped.
    assert_eq!(rule(config, "line-length"), None);
    assert_eq!(loaded.files(), std::slice::from_ref(&file));
    assert_eq!(
        loaded.origin(&["lint", "rules", "MD012", "maximum"]),
        Some(Origin::File {
            path: file,
            line: 4
        })
    );
}

#[test]
fn markdownlint_jsonc_allows_comments_and_trailing_commas() {
    let tree = Tree::new();
    tree.write(
        "a/b/.markdownlint.jsonc",
        "// base\n{\n  /* no blanks */ \"MD012\": { \"maximum\": 3, }, // trailing\n  \"MD047\": false,\n}\n",
    );
    let config = tree.loader().load().unwrap().config().clone();
    assert_eq!(config.lint.extends, LintExtends::Default);
    assert_eq!(
        rule(&config, "MD012"),
        Some(options(&serde_json::json!({ "maximum": 3 })))
    );
    assert_eq!(rule(&config, "MD047"), Some(RuleSetting::Enabled(false)));
}

#[test]
fn markdownlint_yaml_and_closest_file_wins() {
    let tree = Tree::new();
    tree.write("a/.markdownlint.json", "{ \"MD047\": false }");
    tree.write("a/b/.markdownlint.yaml", "MD012:\n  maximum: 4\n");
    tree.write("a/b/.markdownlint.yml", "MD047: false\n");
    let config = tree.loader().load().unwrap().config().clone();
    assert_eq!(
        rule(&config, "MD012"),
        Some(options(&serde_json::json!({ "maximum": 4 })))
    );
    assert_eq!(rule(&config, "MD047"), None);
}

#[test]
fn markdownlint_extends_merges_shallowly_and_the_extending_file_wins() {
    let tree = Tree::new();
    tree.write(
        "base.yaml",
        "default: false\nMD012: { maximum: 5 }\nMD047: true\n",
    );
    tree.write(
        "a/.markdownlint.json",
        "{ \"extends\": \"../base.yaml\", \"MD047\": false }",
    );
    let config = tree.loader().load().unwrap().config().clone();
    assert_eq!(config.lint.extends, LintExtends::None);
    assert_eq!(
        rule(&config, "MD012"),
        Some(options(&serde_json::json!({ "maximum": 5 })))
    );
    assert_eq!(rule(&config, "MD047"), Some(RuleSetting::Enabled(false)));
}

#[test]
fn markdownlint_extends_cycle_is_an_error() {
    let tree = Tree::new();
    tree.write("a/x.json", "{ \"extends\": \".markdownlint.json\" }");
    let file = tree.write("a/.markdownlint.json", "{ \"extends\": \"x.json\" }");
    let err = tree.loader().load().unwrap_err();
    assert!(err.to_string().contains("extends"), "{err}");
    assert!(
        err.location()
            .is_some_and(|l| l.path == file || l.path.ends_with("x.json"))
    );
}

#[test]
fn invalid_markdownlint_options_name_the_markdownlint_file() {
    let tree = Tree::new();
    let file = tree.write(
        "a/.markdownlint.json",
        "{\n  \"MD012\": { \"maximum\": \"two\" }\n}\n",
    );
    let err = tree.loader().load().unwrap_err();
    let location = err.location().expect("location");
    assert_eq!(
        (location.path.as_path(), location.line),
        (file.as_path(), 2)
    );
}

#[test]
fn markdownlint_is_ignored_when_an_mrq_config_sets_lint() {
    let tree = Tree::new();
    tree.write("a/.markdownlint.json", "{ \"MD012\": false }");
    tree.write("a/b/.mrq.yaml", "lint:\n  rules:\n    MD047: off\n");
    let loaded = tree.loader().load().unwrap();
    assert_eq!(rule(loaded.config(), "MD012"), None);
    assert_eq!(loaded.files(), [tree.path("a/b/.mrq.yaml")]);

    // An `.mrq.yaml` without `lint` keeps the fallback.
    tree.write("a/b/.mrq.yaml", "flavor: obsidian\n");
    let loaded = tree.loader().load().unwrap();
    assert_eq!(
        rule(loaded.config(), "MD012"),
        Some(RuleSetting::Enabled(false))
    );
    assert_eq!(loaded.config().flavor, FlavorName::Obsidian);
}

#[test]
fn markdownlint_sits_above_the_user_config() {
    let tree = Tree::new();
    tree.write(
        "user/config.yaml",
        "lint:\n  extends: none\n  rules:\n    MD012: off\n",
    );
    tree.write("a/.markdownlint.yaml", "MD012: true\n");
    let loaded = tree.loader().load().unwrap();
    assert_eq!(loaded.config().lint.extends, LintExtends::Default);
    assert_eq!(
        rule(loaded.config(), "MD012"),
        Some(RuleSetting::Enabled(true))
    );
}
