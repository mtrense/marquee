//! `sarif`: a SARIF 2.1.0 log (<https://docs.oasis-open.org/sarif/sarif/v2.1.0/>).
//!
//! The log is built from a few hand-written serde structs covering the subset mrq
//! emits (tool and rules, results with locations, related locations and fixes) rather
//! than from `serde-sarif`: those structs are a few dozen lines, add no dependencies
//! (serde-sarif pulls in typed-builder, strum and their proc-macro trees), and the
//! output is checked against the official 2.1.0 JSON schema in the tests.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;

use super::location::{display_path, find_source, resolve};
use super::{Diagnostic, Severity};
use crate::source::{ByteRange, SourceFile};

const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// Identity of the analysis tool reported in `runs[].tool.driver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
    pub information_uri: Option<String>,
}

impl Default for ToolInfo {
    /// `mrq` at the version of this crate.
    fn default() -> Self {
        Self {
            name: "mrq".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            information_uri: Some(env!("CARGO_PKG_REPOSITORY").into()),
        }
    }
}

#[derive(Serialize)]
struct Log<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: [Run<'a>; 1],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Run<'a> {
    tool: Tool<'a>,
    column_kind: &'static str,
    results: Vec<SarifResult<'a>>,
}

#[derive(Serialize)]
struct Tool<'a> {
    driver: Driver<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Driver<'a> {
    name: &'a str,
    version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    information_uri: Option<&'a str>,
    rules: Vec<Rule<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Rule<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    help_uri: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult<'a> {
    rule_id: &'a str,
    rule_index: usize,
    level: &'static str,
    message: Message,
    locations: [Location; 1],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_locations: Vec<Location>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fixes: Vec<SarifFix>,
}

#[derive(Serialize)]
struct Message {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)] // names follow the SARIF schema
struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<usize>,
    physical_location: PhysicalLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<Message>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhysicalLocation {
    artifact_location: ArtifactLocation,
    region: Region,
}

#[derive(Serialize, Clone)]
struct ArtifactLocation {
    uri: String,
}

/// Lines/columns when the source is known, byte offsets otherwise.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Region {
    #[serde(skip_serializing_if = "Option::is_none")]
    start_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_length: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifFix {
    description: Message,
    artifact_changes: [ArtifactChange; 1],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactChange {
    artifact_location: ArtifactLocation,
    replacements: Vec<Replacement>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Replacement {
    deleted_region: Region,
    inserted_content: Message,
}

const fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info | Severity::Hint => "note",
    }
}

/// A URI reference for `path`: percent-encoded relative reference, or a `file://` URI
/// for absolute paths.
fn uri(path: &Path) -> String {
    let text = display_path(path);
    let absolute = path.is_absolute() || text.starts_with('/');
    let mut uri = String::new();
    if absolute {
        uri.push_str("file://");
        if !text.starts_with('/') {
            uri.push('/');
        }
    }
    for byte in text.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
            || (absolute && byte == b':');
        if keep {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

fn region(source: Option<&SourceFile>, range: ByteRange) -> Region {
    match resolve(source, range) {
        Some((start, end)) => Region {
            start_line: Some(start.line),
            start_column: Some(start.column),
            end_line: Some(end.line),
            end_column: Some(end.column),
            byte_offset: None,
            byte_length: None,
        },
        None => Region {
            start_line: None,
            start_column: None,
            end_line: None,
            end_column: None,
            byte_offset: Some(range.start),
            byte_length: Some(range.len()),
        },
    }
}

fn result<'a>(diag: &'a Diagnostic, rule_index: usize, sources: &[SourceFile]) -> SarifResult<'a> {
    let source = find_source(sources, &diag.file);
    let artifact = ArtifactLocation {
        uri: uri(&diag.file),
    };
    let location = |id, range, message: Option<&str>| Location {
        id,
        physical_location: PhysicalLocation {
            artifact_location: artifact.clone(),
            region: region(source, range),
        },
        message: message.map(|text| Message { text: text.into() }),
    };
    let mut text = diag.message.clone();
    if let Some(help) = &diag.help {
        let _ = write!(text, "\n\nhelp: {help}");
    }
    SarifResult {
        rule_id: &diag.code,
        rule_index,
        level: level(diag.severity),
        message: Message { text },
        locations: [location(None, diag.range, None)],
        related_locations: diag
            .labels
            .iter()
            .enumerate()
            .map(|(id, label)| location(Some(id), label.range, Some(&label.message)))
            .collect(),
        fixes: diag
            .fix
            .iter()
            .map(|fix| SarifFix {
                description: Message {
                    text: fix.message.clone(),
                },
                artifact_changes: [ArtifactChange {
                    artifact_location: artifact.clone(),
                    replacements: fix
                        .edits
                        .iter()
                        .map(|edit| Replacement {
                            deleted_region: region(source, edit.range),
                            inserted_content: Message {
                                text: edit.replacement.clone(),
                            },
                        })
                        .collect(),
                }],
            })
            .collect(),
    }
}

/// Writes a pretty-printed SARIF 2.1.0 log with one run. Every distinct diagnostic
/// code becomes a rule (with the first `url` seen as its `helpUri`); labels become
/// related locations, fixes become SARIF fixes. Columns are UTF-16 code units.
pub fn emit_sarif(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    tool: &ToolInfo,
    out: &mut impl Write,
) -> io::Result<()> {
    let mut rules: Vec<Rule> = Vec::new();
    let mut results = Vec::with_capacity(diagnostics.len());
    for diag in diagnostics {
        let index = if let Some(index) = rules.iter().position(|rule| rule.id == diag.code) {
            let rule = &mut rules[index];
            rule.help_uri = rule.help_uri.or(diag.url.as_deref());
            index
        } else {
            rules.push(Rule {
                id: &diag.code,
                help_uri: diag.url.as_deref(),
            });
            rules.len() - 1
        };
        results.push(result(diag, index, sources));
    }
    let log = Log {
        schema: SCHEMA,
        version: "2.1.0",
        runs: [Run {
            tool: Tool {
                driver: Driver {
                    name: &tool.name,
                    version: &tool.version,
                    information_uri: tool.information_uri.as_deref(),
                    rules,
                },
            },
            column_kind: "utf16CodeUnits",
            results,
        }],
    };
    serde_json::to_writer_pretty(&mut *out, &log)?;
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn relative_paths_are_percent_encoded() {
        assert_eq!(uri(Path::new("docs/a b%.md")), "docs/a%20b%25.md");
        assert_eq!(uri(Path::new("c:d/ü.md")), "c%3Ad/%C3%BC.md");
    }

    #[test]
    fn absolute_paths_become_file_uris() {
        assert_eq!(uri(Path::new("/srv/a b.md")), "file:///srv/a%20b.md");
    }

    #[test]
    fn maps_severities_to_levels() {
        assert_eq!(level(Severity::Error), "error");
        assert_eq!(level(Severity::Warning), "warning");
        assert_eq!(level(Severity::Info), "note");
        assert_eq!(level(Severity::Hint), "note");
    }

    #[test]
    fn default_tool_is_mrq() {
        let tool = ToolInfo::default();
        assert_eq!(tool.name, "mrq");
        assert_eq!(tool.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn repeated_codes_share_a_rule_and_pick_up_a_later_url() {
        let range = ByteRange::new(0, 1);
        let diags = [
            Diagnostic::new("A", Severity::Error, "a.md", range, "1"),
            Diagnostic::new("B", Severity::Error, "a.md", range, "2"),
            Diagnostic::new("A", Severity::Error, "a.md", range, "3").with_url("https://x.test/A"),
        ];
        let sources = [SourceFile::new(Some(PathBuf::from("a.md")), "ab")];
        let mut out = Vec::new();
        emit_sarif(&diags, &sources, &ToolInfo::default(), &mut out).unwrap();
        let log: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let run = &log["runs"][0];
        assert_eq!(
            run["tool"]["driver"]["rules"],
            serde_json::json!([{"id": "A", "helpUri": "https://x.test/A"}, {"id": "B"}])
        );
        let indices: Vec<_> = run["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["ruleIndex"].as_u64().unwrap())
            .collect();
        assert_eq!(indices, [0, 1, 0]);
    }
}
