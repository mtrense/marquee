//! MD043 `required-headings`.

use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{expected_actual, heading_line, heading_text, message};

/// Required heading structure.
///
/// With `headings`, the document's headings (as `## Text`, any style) must match the
/// list in order: an entry is a heading to compare (ignoring case unless `match_case`),
/// `%`-prefixed for a regular expression on the text, `*` for zero or more headings,
/// `+` for one or more, `?` for exactly one, and `^` (or `^^`, …) marks a loop that
/// repeats while the next heading does not match. The first mismatch is reported on
/// the heading's line, missing headings on the last line. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md043;

/// Options of MD043 `required-headings`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md043Options {
    /// List of headings.
    pub headings: Option<Vec<String>>,
    /// Match case of headings.
    pub match_case: bool,
}

/// markdownlint's `headingsItemsRe`: what a `headings` entry may look like.
static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[*+?]|\^+|[#%]{1,6}\s+\S.*)$").expect("valid regex"));

/// A heading as `#…` and text, with `%` for patterns.
static PREFIXED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([#%]{1,6})\s+(\S.*)$").expect("valid regex"));

/// How often loops may repeat before markdownlint gives up.
const MAX_LOOPS: usize = 1000;

impl Rule for Md043 {
    type Options = Md043Options;

    const META: RuleMeta = RuleMeta {
        id: "MD043",
        aliases: &["required-headings"],
        tags: &["headings"],
        description: "Required heading structure",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md043Options) -> Vec<Diagnostic> {
        let Some(headings) = &options.headings else {
            return Vec::new();
        };
        let required: Vec<&str> = headings
            .iter()
            .map(|heading| {
                if ITEM.is_match(heading) {
                    heading.as_str()
                } else {
                    "[Invalid syntax]"
                }
            })
            .collect();
        let is_loop = |item: &str| !item.is_empty() && item.bytes().all(|b| b == b'^');
        let matches =
            |actual: &str, expected: &str| heading_matches(actual, expected, options.match_case);
        let mut i = 0;
        // markdownlint's `getExpected`: the next entry, past loop markers if `skip_loops`.
        let next = |i: &mut usize, skip_loops: bool| loop {
            let expected = required.get(*i).copied().unwrap_or("[No heading]");
            *i += 1;
            if !(skip_loops && is_loop(expected)) {
                break expected;
            }
        };
        let ast = doc.ast();
        let mut loops_left = MAX_LOOPS;
        let mut any_fallback = false;
        let mut report = None;
        for id in ast.descendants(ast.root()) {
            let NodeKind::Heading(heading) = ast.kind(id) else {
                continue;
            };
            let actual = format!(
                "{} {}",
                "#".repeat(usize::from(heading.level)),
                heading_text(doc, id)
            );
            let range = heading_line(doc, id);
            let mut expected = next(&mut i, false);
            if is_loop(expected) {
                let current = i;
                let next_expected = next(&mut i, true);
                if matches(&actual, next_expected) {
                    expected = next_expected;
                } else {
                    // Loop back to the previous marker like this one, if any.
                    let too_many_loops = loops_left == 0;
                    loops_left = loops_left.saturating_sub(1);
                    let marker = (0..current.saturating_sub(1))
                        .rev()
                        .find(|&j| required[j] == expected);
                    match marker {
                        Some(j) => {
                            i = j + 1;
                            expected = next(&mut i, false);
                        }
                        None => expected = next_expected,
                    }
                    if too_many_loops {
                        let limit = format!("Limit: {MAX_LOOPS}");
                        report = Some((
                            range,
                            message(&Self::META, Some("Too many loops"), Some(&limit)),
                        ));
                        break;
                    }
                }
            }
            match expected {
                "*" => {
                    let next_expected = next(&mut i, true);
                    if !matches(&actual, next_expected) {
                        any_fallback = true;
                        i -= 1;
                    }
                }
                "+" => any_fallback = true,
                "?" => {}
                _ if matches(&actual, expected) => any_fallback = false,
                _ if any_fallback => i -= 1,
                _ => {
                    let detail = expected_actual(expected, &actual);
                    report = Some((range, message(&Self::META, Some(&detail), None)));
                    break;
                }
            }
        }
        if report.is_none()
            && let Some(missing) = required
                .iter()
                .skip(i)
                .find(|&&item| item != "*" && !is_loop(item))
        {
            let end = ByteRange::empty(doc.text().len());
            let detail = expected_actual(missing, "[End of file]");
            report = Some((end, message(&Self::META, Some(&detail), None)));
        }
        report
            .map(|(range, message)| Self::META.diagnostic(doc, range, message))
            .into_iter()
            .collect()
    }
}

/// markdownlint's `headingIsExpected`: text equality, or for `%` entries the same level
/// and a regular expression match on the text. An invalid expression matches nothing.
fn heading_matches(actual: &str, expected: &str, match_case: bool) -> bool {
    if expected.starts_with('%') {
        let (Some(actual), Some(expected)) =
            (PREFIXED.captures(actual), PREFIXED.captures(expected))
        else {
            return false;
        };
        let flags = if match_case { "" } else { "(?i)" };
        return actual[1].len() == expected[1].len()
            && Regex::new(&format!("{flags}{}", &expected[2]))
                .is_ok_and(|pattern| pattern.is_match(&actual[2]));
    }
    if match_case {
        actual == expected
    } else {
        actual.to_lowercase() == expected.to_lowercase()
    }
}
