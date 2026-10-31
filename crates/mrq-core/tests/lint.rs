//! The lint rule framework: rules with typed options, the registry, rule selection
//! from `lint` config and `--rule`, applying fixes, and the proof rules MD012 and MD047.

use mrq_core::Document;
use mrq_core::config::{LintConfig, LintExtends, RuleSetting, RuleSettings};
use mrq_core::diag::{Diagnostic, Fix, Severity};
use mrq_core::document::DocumentOptions;
use mrq_core::lint::{Linter, MAX_FIX_PASSES, NoOptions, Registry, Rule, RuleMeta, apply_fixes};
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

/// A replacement of `range` with `replacement`, as a fixable finding of `meta`.
fn fixable(meta: RuleMeta, doc: &Document, range: ByteRange, replacement: &str) -> Diagnostic {
    let edit = TextEdit {
        range,
        replacement: replacement.to_owned(),
    };
    meta.diagnostic(doc, range, "fixable")
        .with_fix(Fix::new("fix", vec![edit]))
}

/// Squashes every `aa` into `a`, one pair per match, so `aaaa` needs two passes.
struct Squash;

impl Rule for Squash {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST003",
        aliases: &["squash"],
        tags: &["fixing"],
        description: "Doubled a",
        default_enabled: false,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        doc.text()
            .match_indices("aa")
            .map(|(at, _)| fixable(Self::META, doc, ByteRange::new(at, at + 2), "a"))
            .collect()
    }
}

/// Reports a missing final newline twice, with identical fixes.
struct Twice;

impl Rule for Twice {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST004",
        aliases: &["twice"],
        tags: &["fixing"],
        description: "No final newline",
        default_enabled: false,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let end = doc.text().len();
        if doc.text().ends_with('\n') {
            return Vec::new();
        }
        let at = ByteRange::new(end, end);
        vec![
            fixable(Self::META, doc, at, "\n"),
            fixable(Self::META, doc, at, "\n"),
        ]
    }
}

/// Swaps `x` and `y` forever: never reaches a fixpoint.
struct Flip;

impl Rule for Flip {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST005",
        aliases: &["flip"],
        tags: &[],
        description: "x or y",
        default_enabled: false,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        doc.text()
            .match_indices(['x', 'y'])
            .map(|(at, c)| {
                let other = if c == "x" { "y" } else { "x" };
                fixable(Self::META, doc, ByteRange::new(at, at + 1), other)
            })
            .collect()
    }
}

/// [`registry`] plus the fixing rules.
fn fixing() -> Registry {
    let mut registry = registry();
    registry.register(Squash);
    registry.register(Twice);
    registry.register(Flip);
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

fn replace(doc: &Document, start: usize, end: usize, replacement: &str) -> Diagnostic {
    fixable(Squash::META, doc, ByteRange::new(start, end), replacement)
}

#[test]
fn apply_fixes_dedupes_identical_fixes() {
    let linter = Linter::new(
        &fixing(),
        &config(LintExtends::None, &[("twice", true.into())]),
    );
    let doc = Document::parse("a");
    let diagnostics = linter.unwrap().check(&doc);
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(apply_fixes(doc.text(), &diagnostics).unwrap(), "a\n");
}

#[test]
fn apply_fixes_skips_fixes_overlapping_an_earlier_one() {
    let doc = Document::parse("abcdef");
    let diagnostics = [
        replace(&doc, 2, 4, "Y"),
        replace(&doc, 0, 3, "X"),
        replace(&doc, 3, 3, "-"),
        replace(&doc, 4, 6, "Z"),
        Squash::META.diagnostic(&doc, ByteRange::new(0, 6), "no fix"),
    ];
    // 0..3 starts first; 2..4 overlaps it, the insertion at its end and 4..6 do not.
    assert_eq!(apply_fixes(doc.text(), &diagnostics).unwrap(), "X-dZ");
    assert_eq!(
        apply_fixes(doc.text(), &diagnostics[4..]).unwrap(),
        "abcdef"
    );
}

#[test]
fn apply_fixes_rejects_invalid_edits() {
    let doc = Document::parse("ab");
    assert!(apply_fixes(doc.text(), &[replace(&doc, 1, 9, "")]).is_err());
}

#[test]
fn fix_reparses_and_relints_until_nothing_changes() {
    let linter = Linter::new(
        &fixing(),
        &config(LintExtends::None, &[("squash", true.into())]),
    )
    .unwrap();
    let fixed = linter
        .fix(Document::parse("aaaa\n"), &DocumentOptions::new())
        .unwrap();
    assert_eq!(fixed.document.text(), "a\n");
    assert_eq!(fixed.passes, 2);
    assert_eq!(fixed.diagnostics, []);

    let clean = linter
        .fix(Document::parse("b\n"), &DocumentOptions::new())
        .unwrap();
    assert_eq!(clean.passes, 0);
    assert_eq!(clean.document.text(), "b\n");
}

#[test]
fn fix_stops_after_max_passes() {
    let linter = Linter::new(
        &fixing(),
        &config(LintExtends::None, &[("flip", true.into())]),
    )
    .unwrap();
    let fixed = linter
        .fix(Document::parse("x\n"), &DocumentOptions::new())
        .unwrap();
    assert_eq!(fixed.passes, MAX_FIX_PASSES);
    assert_eq!(MAX_FIX_PASSES, 10);
    // Ten flips: back to `x`, still reported.
    assert_eq!(fixed.document.text(), "x\n");
    assert_eq!(fixed.diagnostics.len(), 1);
}

#[test]
fn fix_reports_what_remains_in_the_fixed_text() {
    let cfg = config(
        LintExtends::None,
        &[("squash", true.into()), ("long-line", true.into())],
    );
    let linter = Linter::new(&fixing(), &cfg).unwrap();
    let options = DocumentOptions::new().flavor("obsidian");
    let doc = options
        .parse_source(mrq_core::source::SourceFile::new(
            Some("notes.md".into()),
            "bbbbbbbbbbbbaa\n",
        ))
        .unwrap();
    let fixed = linter.fix(doc, &options).unwrap();
    assert_eq!(fixed.document.text(), "bbbbbbbbbbbba\n");
    assert_eq!(fixed.document.name(), std::path::Path::new("notes.md"));
    assert_eq!(fixed.document.flavor().name(), "obsidian");
    let remaining: Vec<_> = fixed
        .diagnostics
        .iter()
        .map(|diag| (diag.code.as_str(), diag.range, diag.fix.is_some()))
        .collect();
    assert_eq!(remaining, [("TEST001", ByteRange::new(0, 13), false)]);
}

#[test]
fn restrict_to_runs_only_the_named_rules_even_if_config_disables_them() {
    let reg = registry();
    let cfg = config(
        LintExtends::Default,
        &[("TEST001", serde_json::json!({ "max": 3 }))],
    );
    let only = |names: &[&str]| Linter::new(&reg, &cfg).unwrap().restrict_to(names);
    assert_eq!(ids(&only(&["always"]).unwrap()), ["TEST002"]);
    assert_eq!(ids(&only(&["Test"]).unwrap()), ["TEST001", "TEST002"]);
    // A disabled rule is switched on, with its configured options.
    let off = config(
        LintExtends::None,
        &[
            ("TEST001", serde_json::json!({ "max": 3 })),
            ("test", false.into()),
        ],
    );
    let linter = Linter::new(&reg, &off)
        .unwrap()
        .restrict_to(["long-line"])
        .unwrap();
    assert_eq!(ids(&linter), ["TEST001"]);
    assert_eq!(codes(&linter, "abc\nabcd\n"), ["TEST001"]);

    let err = only(&["TEST001", "nope"]).unwrap_err();
    assert!(err.to_string().contains("`nope`"), "{err}");
}

#[test]
fn restrict_to_is_kept_by_inline_configuration() {
    let reg = registry();
    let linter = Linter::new(&reg, &LintConfig::default())
        .unwrap()
        .restrict_to(["TEST001"])
        .unwrap();
    // Enabling comments cannot add rules beyond the restriction ...
    let text = "<!-- mrq-enable TEST002 -->\n<!-- markdownlint-configure-file { \"TEST002\": true, \"TEST001\": { \"max\": 200 } } -->\n";
    let codes_found = codes(&linter, text);
    assert!(
        !codes_found.contains(&"TEST002".to_owned()),
        "{codes_found:?}"
    );
    // ... configure-file still sets the options, and disabling comments still apply.
    assert_eq!(codes_found, Vec::<String>::new());
    let disabled = "<!-- mrq-disable-line --> a line that is long\n";
    assert_eq!(codes(&linter, disabled), Vec::<String>::new());
}
