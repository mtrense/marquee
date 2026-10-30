//! MD053 `link-image-reference-definitions`.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{LinkForm, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::md052::undefined_references;
use super::{Definitions, cleared_lines, edit, ellipsify, message, normalize_label};

/// Link and image reference definitions should be needed.
///
/// A link reference definition or footnote definition (`^label`) that no reference
/// link, image or footnote uses is reported, and so is every later definition of a
/// label already defined; labels in `ignored_definitions` (normalized, `//` by default
/// for `[//]: # (comment)`) are fine. The finding covers the definition's first line;
/// the fix deletes that line when the definition fits on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md053;

/// Options of MD053 `link-image-reference-definitions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md053Options {
    /// Ignored definitions.
    pub ignored_definitions: Vec<String>,
}

impl Default for Md053Options {
    fn default() -> Self {
        Self {
            ignored_definitions: vec!["//".to_owned()],
        }
    }
}

/// markdownlint's `linkReferenceDefinitionRe`: a definition's label at the start of a
/// line.
static DEFINITION_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ {0,3}\[([^\]]*[^\\])\]:").expect("valid regex"));

impl Rule for Md053 {
    type Options = Md053Options;

    const META: RuleMeta = RuleMeta {
        id: "MD053",
        aliases: &["link-image-reference-definitions"],
        tags: &["images", "links"],
        description: "Link and image reference definitions should be needed",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md053Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let source = doc.source();
        let ignored: HashSet<&str> = options
            .ignored_definitions
            .iter()
            .map(String::as_str)
            .collect();
        let mut used: HashSet<String> = undefined_references(doc)
            .into_iter()
            .map(|reference| reference.label)
            .collect();
        for id in ast.descendants(ast.root()) {
            match ast.kind(id) {
                NodeKind::Link(link) | NodeKind::Image(link) => {
                    if let LinkForm::Reference { label, .. } = &link.form {
                        used.insert(normalize_label(label));
                    }
                }
                NodeKind::FootnoteReference(footnote) => {
                    used.insert(normalize_label(&format!("^{}", footnote.label)));
                }
                _ => {}
            }
        }
        let definitions = Definitions::of(doc);
        let unused = definitions
            .first
            .iter()
            .filter(|(label, _, _)| !used.contains(label))
            .map(|(label, line, _)| ("Unused", label, *line));
        let duplicates = definitions
            .duplicates
            .iter()
            .map(|(label, line)| ("Duplicate", label, *line));
        let lines = cleared_lines(doc);
        let mut diagnostics = Vec::new();
        for (kind, label, line) in unused.chain(duplicates) {
            if ignored.contains(label.as_str()) {
                continue;
            }
            let Some(range) = source.line_range(line) else {
                continue;
            };
            let content = lines.get(line).map_or("", String::as_str);
            let detail = format!("{kind} link or image reference definition: \"{label}\"");
            let shown = ellipsify(content, false, false);
            let diagnostic = Self::META.diagnostic(
                doc,
                range,
                message(&Self::META, Some(&detail), Some(&shown)),
            );
            let single_line = !DEFINITION_START.replace(content, "").trim().is_empty();
            diagnostics.push(if single_line {
                diagnostic.with_fix(Fix::new(
                    "Delete the definition",
                    vec![edit(line_with_ending(doc, line), "")],
                ))
            } else {
                diagnostic
            });
        }
        diagnostics
    }
}

/// Line `line` with its line ending, or with the line ending before it for a last line
/// without one.
fn line_with_ending(doc: &Document, line: usize) -> ByteRange {
    let source = doc.source();
    let range = source
        .line_range(line)
        .unwrap_or(ByteRange::empty(doc.text().len()));
    if let Some(next) = source.line_range(line + 1) {
        return ByteRange::new(range.start, next.start);
    }
    let start = line
        .checked_sub(1)
        .and_then(|previous| source.line_range(previous))
        .map_or(range.start, |previous| previous.end);
    ByteRange::new(start, range.end)
}
