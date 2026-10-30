//! Inline suppression comments (`markdownlint-disable`, `mrq-disable`, …), mirroring
//! markdownlint's semantics.

use mrq_core::Document;
use mrq_core::config::{LintConfig, LintExtends};
use mrq_core::diag::Diagnostic;
use mrq_core::lint::{Linter, Registry, Rule, RuleMeta};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Reports every line containing `word` (`bad` by default) at the line's start.
fn word_lines(meta: RuleMeta, doc: &Document, word: &str) -> Vec<Diagnostic> {
    let source = doc.source();
    (0..source.line_count())
        .filter_map(|line| source.line_range(line))
        .filter(|range| doc.text()[range.start..range.end].contains(word))
        .map(|range| meta.diagnostic(doc, range, word.to_owned()))
        .collect()
}

struct Bad;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct BadOptions {
    word: String,
}

impl Default for BadOptions {
    fn default() -> Self {
        Self { word: "bad".into() }
    }
}

impl Rule for Bad {
    type Options = BadOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST001",
        aliases: &["no-bad"],
        tags: &["words"],
        description: "Bad word",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &BadOptions) -> Vec<Diagnostic> {
        word_lines(Self::META, doc, &options.word)
    }
}

struct Ugly;

impl Rule for Ugly {
    type Options = mrq_core::lint::NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST002",
        aliases: &["no-ugly"],
        tags: &["words", "looks"],
        description: "Ugly word",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &Self::Options) -> Vec<Diagnostic> {
        word_lines(Self::META, doc, "ugly")
    }
}

/// Off by default.
struct Odd;

impl Rule for Odd {
    type Options = mrq_core::lint::NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "TEST003",
        aliases: &["no-odd"],
        tags: &["looks"],
        description: "Odd word",
        default_enabled: false,
    };

    fn check(&self, doc: &Document, _: &Self::Options) -> Vec<Diagnostic> {
        word_lines(Self::META, doc, "odd")
    }
}

fn linter() -> Linter {
    let mut registry = Registry::new();
    registry.register(Bad);
    registry.register(Ugly);
    registry.register(Odd);
    let config = LintConfig {
        extends: LintExtends::Default,
        ..LintConfig::default()
    };
    Linter::new(&registry, &config).unwrap()
}

/// `(line, code)` of every finding, lines counted from 1 like markdownlint.
fn findings(text: &str) -> Vec<(usize, String)> {
    let doc = Document::parse(text);
    linter()
        .check(&doc)
        .into_iter()
        .map(|diag| {
            let line = doc
                .source()
                .offset_to_position(diag.range.start, mrq_core::source::PositionEncoding::Utf8)
                .unwrap()
                .line;
            (line + 1, diag.code)
        })
        .collect()
}

fn at(pairs: &[(usize, &str)]) -> Vec<(usize, String)> {
    pairs
        .iter()
        .map(|(line, code)| (*line, (*code).to_owned()))
        .collect()
}

#[test]
fn without_comments_every_finding_is_reported() {
    assert_eq!(
        findings("bad\n\nugly\n"),
        at(&[(1, "TEST001"), (3, "TEST002")])
    );
}

#[test]
fn disable_and_enable_switch_all_rules_from_the_comment_on() {
    let text =
        "bad\n\n<!-- markdownlint-disable -->\nbad ugly\n\n<!-- markdownlint-enable -->\nugly\n";
    assert_eq!(findings(text), at(&[(1, "TEST001"), (7, "TEST002")]));
}

#[test]
fn a_comment_affects_its_own_line() {
    let text = "bad <!-- markdownlint-disable -->\nbad\n\nbad <!-- markdownlint-enable -->\n";
    assert_eq!(findings(text), at(&[(4, "TEST001")]));
}

#[test]
fn comments_on_one_line_apply_in_order() {
    let text = "<!-- markdownlint-disable --><!-- markdownlint-enable TEST001 -->\n\nbad ugly\n";
    assert_eq!(findings(text), at(&[(3, "TEST001")]));
    let text = "<!-- markdownlint-enable TEST001 --><!-- markdownlint-disable -->\n\nbad ugly\n";
    assert_eq!(findings(text), []);
}

#[test]
fn names_are_ids_aliases_or_tags_ignoring_case() {
    let only_ugly = at(&[(3, "TEST002")]);
    for names in ["TEST001", "test001", "no-bad", "NO-BAD"] {
        let text = format!("<!-- markdownlint-disable {names} -->\n\nbad ugly\n");
        assert_eq!(findings(&text), only_ugly, "{names}");
    }
    let text = "<!-- markdownlint-disable  no-bad\tTEST002 -->\n\nbad ugly\n";
    assert_eq!(findings(text), []);
    let text = "<!-- markdownlint-disable WORDS -->\n\nbad ugly\n";
    assert_eq!(findings(text), []);
}

#[test]
fn unknown_names_are_ignored() {
    let text = "<!-- markdownlint-disable MD999 nope -->\n\nbad\n";
    assert_eq!(findings(text), at(&[(3, "TEST001")]));
}

#[test]
fn disable_line_suppresses_the_comments_line_only() {
    let text = "bad <!-- markdownlint-disable-line -->\nbad\n\nbad ugly <!-- markdownlint-disable-line no-bad -->\n";
    assert_eq!(findings(text), at(&[(2, "TEST001"), (4, "TEST002")]));
}

#[test]
fn disable_next_line_suppresses_the_following_line_only() {
    let text = "bad <!-- markdownlint-disable-next-line -->\nbad ugly\nbad\n\n<!-- markdownlint-disable-next-line TEST002 -->\nbad ugly\n";
    assert_eq!(
        findings(text),
        at(&[(1, "TEST001"), (3, "TEST001"), (6, "TEST001")])
    );
}

#[test]
fn disable_line_wins_over_a_later_enable_on_the_line() {
    let text = "<!-- markdownlint-disable-line --> bad <!-- markdownlint-enable -->\n";
    assert_eq!(findings(text), []);
}

#[test]
fn disable_file_applies_to_the_whole_file_wherever_it_is() {
    let text = "bad ugly\n\n<!-- markdownlint-disable-file TEST001 -->\n";
    assert_eq!(findings(text), at(&[(1, "TEST002")]));
    // Enable after a file-wide disable re-enables from there on.
    let text = "bad\n\n<!-- markdownlint-enable -->\nbad\n\n<!-- markdownlint-disable-file -->\n";
    assert_eq!(findings(text), at(&[(4, "TEST001")]));
}

#[test]
fn enable_file_and_enable_switch_on_rules_that_config_left_off() {
    assert_eq!(findings("odd\n"), []);
    let text = "odd\n\n<!-- markdownlint-enable-file NO-ODD -->\n";
    assert_eq!(findings(text), at(&[(1, "TEST003")]));
    let text = "odd\n\n<!-- markdownlint-enable TEST003 -->\nodd\n";
    assert_eq!(findings(text), at(&[(4, "TEST003")]));
}

#[test]
fn restore_returns_to_the_captured_state() {
    let text = "<!-- markdownlint-disable TEST001 -->\n<!-- markdownlint-capture -->\n<!-- markdownlint-disable -->\nbad ugly\n<!-- markdownlint-restore -->\nbad ugly\n";
    assert_eq!(findings(text), at(&[(6, "TEST002")]));
}

#[test]
fn restore_without_capture_returns_to_the_initial_state() {
    let text = "<!-- markdownlint-disable -->\nbad\n<!-- markdownlint-restore -->\nbad\n";
    assert_eq!(findings(text), at(&[(4, "TEST001")]));
    // Like markdownlint, the initial capture is the configured state, before file-wide
    // comments.
    let text =
        "<!-- markdownlint-disable-file TEST002 -->\n<!-- markdownlint-restore -->\nbad ugly\n";
    assert_eq!(findings(text), at(&[(3, "TEST001"), (3, "TEST002")]));
}

#[test]
fn capture_holds_one_state_not_a_stack() {
    let text = "<!-- markdownlint-disable TEST001 -->\n<!-- markdownlint-capture -->\n<!-- markdownlint-disable TEST002 -->\n<!-- markdownlint-capture -->\n<!-- markdownlint-enable -->\n<!-- markdownlint-restore -->\n<!-- markdownlint-restore -->\nbad ugly\n";
    assert_eq!(findings(text), []);
}

#[test]
fn mrq_prefix_works_like_markdownlint() {
    let text = "<!-- mrq-disable no-bad -->\nbad ugly\n<!-- mrq-enable -->\nbad <!-- mrq-disable-line -->\n<!-- mrq-disable-next-line -->\nugly\nodd\n<!-- mrq-enable-file TEST003 -->\n";
    assert_eq!(findings(text), at(&[(2, "TEST002"), (7, "TEST003")]));
    let text = "bad\n<!-- mrq-capture -->\n<!-- mrq-disable -->\n<!-- mrq-restore -->\nbad\n<!-- mrq-disable-file -->\n";
    assert_eq!(findings(text), []);
}

#[test]
fn keywords_ignore_case_and_need_a_word_boundary() {
    let text = "<!--MARKDOWNLINT-Disable-->\nbad\n";
    assert_eq!(findings(text), []);
    let text = "<!-- markdownlint-disabled -->\nbad\n<!-- mrq-disablement TEST001 -->\nbad\n<!-- disable -->\nbad\n";
    assert_eq!(
        findings(text),
        at(&[(2, "TEST001"), (4, "TEST001"), (6, "TEST001")])
    );
}

/// Divergence: markdownlint needs `-->` on the line of `<!--` (except for
/// `configure-file`); mrq takes any comment and places it on its first line.
#[test]
fn comments_may_span_lines() {
    let text = "bad <!-- markdownlint-disable-line\n  TEST001 -->\nbad\n";
    assert_eq!(findings(text), at(&[(3, "TEST001")]));
}

#[test]
fn comments_in_code_are_ignored() {
    let text = "```\n<!-- markdownlint-disable -->\n```\n\nbad\n\n`<!-- mrq-disable-line -->` bad\n\n    <!-- mrq-disable-file -->\n\nbad\n";
    assert_eq!(
        findings(text),
        at(&[(5, "TEST001"), (7, "TEST001"), (11, "TEST001")])
    );
}

#[test]
fn comments_inside_html_blocks_count() {
    let text = "<div>\n<!-- markdownlint-disable -->\n</div>\n\nbad\n";
    assert_eq!(findings(text), []);
}

#[test]
fn comments_in_containers_count() {
    let text = "> <!-- markdownlint-disable-next-line -->\n> bad\n\n- item <!-- mrq-disable-line --> bad\n";
    assert_eq!(findings(text), []);
}

#[test]
fn configure_file_changes_settings_for_the_whole_file() {
    let text = "bad ugly\n\n<!-- markdownlint-configure-file { \"no-bad\": false } -->\n";
    assert_eq!(findings(text), at(&[(1, "TEST002")]));
    let text = "bad ugly worse <!-- mrq-configure-file {\"TEST001\": {\"word\": \"worse\"}, \"TEST002\": false} -->\n";
    let diags = linter().check(&Document::parse(text));
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].message, "worse");
    // Spanning lines, then switched off and on again by later comments.
    let text = "<!-- markdownlint-configure-file {\n  \"default\": false,\n  \"TEST003\": true\n} -->\nbad odd\n<!-- markdownlint-disable -->\nodd\n<!-- markdownlint-enable no-bad -->\nbad\n";
    assert_eq!(
        findings(text),
        at(&[(5, "TEST003"), (8, "TEST001"), (9, "TEST001")])
    );
}

#[test]
fn invalid_configure_file_settings_are_ignored() {
    for json in [
        "{ nope",
        "[1]",
        "{\"MD999\": false}",
        "{\"TEST001\": {\"nope\": 1}}",
    ] {
        let text = format!("bad\n\n<!-- markdownlint-configure-file {json} -->\n");
        assert_eq!(findings(&text), at(&[(1, "TEST001")]), "{json}");
    }
    let text =
        "bad ugly\n\n<!-- markdownlint-configure-file {\"MD999\": false, \"TEST002\": false} -->\n";
    assert_eq!(findings(text), at(&[(1, "TEST001")]));
}

#[test]
fn rules_reports_config_not_comments() {
    let ids: Vec<_> = linter().rules().map(|meta| meta.id).collect();
    assert_eq!(ids, ["TEST001", "TEST002"]);
}
