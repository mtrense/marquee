//! The lint rule framework: rules with typed options, the registry, rule selection
//! from `lint` config, and the proof rules MD012 and MD047.

use mrq_core::Document;
use mrq_core::config::{LintConfig, LintExtends, RuleSetting, RuleSettings};
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::lint::{Linter, NoOptions, Registry, Rule, RuleMeta};
use mrq_core::source::{ByteRange, TextEdit};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Reports every line longer than `max` characters; off by default.
struct LongLine;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct LongLineOptions {
    max: usize,
}

impl Default for LongLineOptions {
    fn default() -> Self {
        Self { max: 10 }
    }
}

impl Rule for LongLine {
    type Options = LongLineOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST001",
        aliases: &["long-line"],
        tags: &["test"],
        description: "Line is too long",
        default_enabled: false,
    };

    fn check(&self, doc: &Document, options: &LongLineOptions) -> Vec<Diagnostic> {
        let source = doc.source();
        (0..source.line_count())
            .filter_map(|line| source.line_range(line))
            .filter(|range| range.len() > options.max)
            .map(|range| Self::META.diagnostic(doc, range, "line too long"))
            .collect()
    }
}

/// Reports the first byte of every document; on by default, tagged `test`.
struct Always;

impl Rule for Always {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST002",
        aliases: &["always"],
        tags: &["test", "other"],
        description: "Always reports",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        vec![Self::META.diagnostic(doc, ByteRange::new(0, 0), "always")]
    }
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(LongLine);
    registry.register(Always);
    registry
}

fn config(extends: LintExtends, rules: &[(&str, serde_json::Value)]) -> LintConfig {
    let rules: serde_json::Map<String, serde_json::Value> = rules
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    LintConfig {
        extends,
        rules: serde_json::from_value(serde_json::Value::Object(rules)).unwrap(),
    }
}

fn codes(linter: &Linter, text: &str) -> Vec<String> {
    let doc = Document::parse(text);
    linter
        .check(&doc)
        .into_iter()
        .map(|diag| diag.code)
        .collect()
}

fn ids(linter: &Linter) -> Vec<&'static str> {
    linter.rules().map(|meta| meta.id).collect()
}

#[test]
fn default_extends_enables_default_rules_only() {
    let linter = Linter::new(&registry(), &LintConfig::default()).unwrap();
    assert_eq!(ids(&linter), ["TEST002"]);
}

#[test]
fn extends_none_starts_empty() {
    let linter = Linter::new(&registry(), &config(LintExtends::None, &[])).unwrap();
    assert_eq!(ids(&linter), Vec::<&str>::new());
}

#[test]
fn true_options_and_false_toggle_rules() {
    let reg = registry();
    let on = config(
        LintExtends::Default,
        &[("TEST001", true.into()), ("TEST002", false.into())],
    );
    assert_eq!(ids(&Linter::new(&reg, &on).unwrap()), ["TEST001"]);
    let options = config(
        LintExtends::None,
        &[("TEST001", serde_json::json!({ "max": 3 }))],
    );
    let linter = Linter::new(&reg, &options).unwrap();
    assert_eq!(codes(&linter, "abc\nabcd\n"), ["TEST001"]);
}

#[test]
fn on_and_off_strings_toggle_rules() {
    let reg = registry();
    let cfg = config(
        LintExtends::Default,
        &[("TEST001", "on".into()), ("TEST002", "off".into())],
    );
    assert_eq!(ids(&Linter::new(&reg, &cfg).unwrap()), ["TEST001"]);
}

#[test]
fn aliases_and_ids_ignore_case() {
    let reg = registry();
    let cfg = config(
        LintExtends::None,
        &[("Long-Line", true.into()), ("test002", true.into())],
    );
    assert_eq!(
        ids(&Linter::new(&reg, &cfg).unwrap()),
        ["TEST001", "TEST002"]
    );
}

#[test]
fn tags_toggle_groups_and_rules_win_over_tags() {
    let reg = registry();
    let cfg = config(LintExtends::None, &[("test", true.into())]);
    assert_eq!(
        ids(&Linter::new(&reg, &cfg).unwrap()),
        ["TEST001", "TEST002"]
    );
    let cfg = config(
        LintExtends::Default,
        &[("other", false.into()), ("always", true.into())],
    );
    assert_eq!(ids(&Linter::new(&reg, &cfg).unwrap()), ["TEST002"]);
}

#[test]
fn rules_unknown_to_the_registry_are_errors() {
    let empty = Registry::new();
    let cfg = config(LintExtends::None, &[("MD047", true.into())]);
    let err = Linter::new(&empty, &cfg).unwrap_err();
    assert!(err.to_string().contains("MD047"), "{err}");
}

#[test]
fn diagnostics_are_warnings_sorted_by_position() {
    let reg = registry();
    let cfg = config(LintExtends::Default, &[("TEST001", true.into())]);
    let linter = Linter::new(&reg, &cfg).unwrap();
    let doc = Document::parse("short\nthis line is long\n");
    let diags = linter.check(&doc);
    assert_eq!(
        diags
            .iter()
            .map(|d| (d.code.as_str(), d.range.start))
            .collect::<Vec<_>>(),
        [("TEST002", 0), ("TEST001", 6)]
    );
    assert!(diags.iter().all(|d| d.severity == Severity::Warning));
    assert_eq!(diags[1].message, "line too long");
}

#[test]
fn builtin_registry_has_the_proof_rules() {
    let registry = Registry::builtin();
    let md047 = registry
        .get("single-trailing-newline")
        .expect("MD047 by alias");
    assert_eq!(md047.id, "MD047");
    assert!(md047.default_enabled);
    assert_eq!(registry.get("md012").expect("MD012").id, "MD012");
    let ids: Vec<&str> = registry.rules().map(|meta| meta.id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "sorted: {ids:?}");
}

fn builtin(rules: &[(&str, serde_json::Value)]) -> Linter {
    Linter::new(&Registry::builtin(), &config(LintExtends::None, rules)).unwrap()
}

fn fixed(linter: &Linter, text: &str) -> String {
    let doc = Document::parse(text);
    let edits: Vec<TextEdit> = linter
        .check(&doc)
        .into_iter()
        .flat_map(|diag| diag.fix.expect("fix").edits)
        .collect();
    doc.source().apply_edits(&edits).unwrap().text().to_owned()
}

#[test]
fn md047_requires_a_single_trailing_newline() {
    let linter = builtin(&[("MD047", true.into())]);
    assert_eq!(linter.check(&Document::parse("# A\n")), []);
    assert_eq!(linter.check(&Document::parse("")), []);
    let diags = linter.check(&Document::parse("# A\ntext"));
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].range, ByteRange::new(7, 8));
    assert_eq!(fixed(&linter, "# A\ntext"), "# A\ntext\n");
    assert_eq!(fixed(&linter, "a\r\nb"), "a\r\nb\r\n");
}

#[test]
fn md012_reports_blank_lines_beyond_the_maximum_outside_code() {
    let linter = builtin(&[("MD012", true.into())]);
    let text = "a\n\n\n\nb\n\n```\n\n\n```\n";
    let diags = linter.check(&Document::parse(text));
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert_eq!(fixed(&linter, text), "a\n\nb\n\n```\n\n\n```\n");

    let two = builtin(&[("no-multiple-blanks", serde_json::json!({ "maximum": 2 }))]);
    assert_eq!(fixed(&two, text), "a\n\n\nb\n\n```\n\n\n```\n");
}

#[test]
fn md012_ignores_frontmatter() {
    let linter = builtin(&[("MD012", true.into())]);
    let text = "---\na: 1\n\n\nb: 2\n---\n# A\n";
    assert_eq!(linter.check(&Document::parse(text)), []);
}

#[test]
fn rule_settings_keep_keys_as_written() {
    let settings: RuleSettings =
        serde_json::from_value(serde_json::json!({ "md047": false, "MD012": { "maximum": 3 } }))
            .unwrap();
    assert_eq!(settings.get("md047"), Some(&RuleSetting::Enabled(false)));
    assert!(matches!(
        settings.get("MD012"),
        Some(RuleSetting::Options(_))
    ));
}

#[test]
fn rule_settings_validate_against_a_registry() {
    let validate = |json: serde_json::Value| {
        serde_json::from_value::<RuleSettings>(json)
            .unwrap()
            .validate(&Registry::builtin())
            .map_err(|err| err.to_string())
    };
    assert_eq!(
        validate(serde_json::json!({ "md012": { "maximum": 2 }, "whitespace": "off" })),
        Ok(())
    );
    let err = validate(serde_json::json!({ "MD999": true })).unwrap_err();
    assert!(err.contains("unknown lint rule `MD999`"), "{err}");
    let err = validate(serde_json::json!({ "MD012": { "maximun": 2 } })).unwrap_err();
    assert!(err.contains("maximun"), "{err}");
    assert!(validate(serde_json::json!({ "MD012": { "maximum": "x" } })).is_err());
    assert!(validate(serde_json::json!({ "whitespace": { "a": 1 } })).is_err());
    let err =
        serde_json::from_value::<RuleSettings>(serde_json::json!({ "MD012": 3 })).unwrap_err();
    assert!(err.to_string().contains("expected"), "{err}");
}

#[test]
fn config_schema_includes_every_rules_options() {
    let schema = mrq_core::config::schema_json();
    let schema: serde_json::Value = serde_json::from_str(&schema).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let rules = |rules: serde_json::Value| serde_json::json!({ "lint": { "rules": rules } });
    for meta in Registry::builtin().rules() {
        for key in std::iter::once(meta.id).chain(meta.aliases.iter().copied()) {
            for value in [
                serde_json::json!(true),
                serde_json::json!("off"),
                serde_json::json!({}),
            ] {
                let instance = rules(serde_json::json!({ key: value }));
                assert!(validator.is_valid(&instance), "{instance}");
            }
            let instance = rules(serde_json::json!({ key: { "no_such_option": 1 } }));
            assert!(!validator.is_valid(&instance), "{instance}");
        }
    }
    assert!(validator.is_valid(&rules(serde_json::json!({ "MD012": { "maximum": 2 } }))));
    assert!(!validator.is_valid(&rules(serde_json::json!({ "MD012": { "maximum": "x" } }))));
    assert!(!validator.is_valid(&rules(serde_json::json!({ "MD999": true }))));
    assert!(validator.is_valid(&rules(serde_json::json!({ "whitespace": false }))));
}
