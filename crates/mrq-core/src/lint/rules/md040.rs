//! MD040 `fenced-code-language`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{ellipsify, line_of, message};

/// Fenced code blocks should have a language specified.
///
/// A fenced code block without an info string is reported; with `allowed_languages`,
/// so is one whose language (the info string's first word, as written) is not in the
/// list, and with `language_only`, one whose info string has more than the language.
/// No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md040;

/// Options of MD040 `fenced-code-language`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md040Options {
    /// List of languages.
    pub allowed_languages: Vec<String>,
    /// Require language only.
    pub language_only: bool,
}

impl Rule for Md040 {
    type Options = Md040Options;

    const META: RuleMeta = RuleMeta {
        id: "MD040",
        aliases: &["fenced-code-language"],
        tags: &["code", "language"],
        description: "Fenced code blocks should have a language specified",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md040Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::CodeBlock(code) = ast.kind(id) else {
                continue;
            };
            let Some(fence) = code.fence else {
                continue;
            };
            let start = ast.span(id).start;
            let end = doc
                .source()
                .line_range(line_of(doc, start))
                .map_or(start, |line| line.end);
            let range = ByteRange::new(start, end);
            // The opening fence line from the fence on, as written.
            let line = &text[start..end];
            let info = line[fence.len.min(line.len())..].trim_start();
            let language = info.split_whitespace().next().unwrap_or_default();
            let mut report = |message: String| {
                diagnostics.push(Self::META.diagnostic(doc, range, message));
            };
            if language.is_empty() {
                let shown = ellipsify(line, false, false);
                report(message(&Self::META, None, Some(&shown)));
            } else if !options.allowed_languages.is_empty()
                && !options
                    .allowed_languages
                    .iter()
                    .any(|allowed| allowed == language)
            {
                let detail = format!("\"{language}\" is not allowed");
                report(message(&Self::META, Some(&detail), None));
            }
            if options.language_only && !info[language.len()..].trim().is_empty() {
                let detail = format!("Info string contains more than language: \"{line}\"");
                report(message(&Self::META, Some(&detail), None));
            }
        }
        diagnostics
    }
}
