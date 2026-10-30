//! MD010 `no-hard-tabs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, js_len, lines, message, span_lines};

/// Hard tabs.
///
/// Every run of tab characters is reported, also in code unless `code_blocks` is off
/// (then code blocks and code spans are skipped) or the fenced block's language is in
/// `ignore_code_languages`. The fix replaces each tab with `spaces_per_tab` spaces.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md010;

/// Options of MD010 `no-hard-tabs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md010Options {
    /// Include code blocks.
    pub code_blocks: bool,
    /// Fenced code languages to ignore (ignoring case). Like markdownlint, a non-empty
    /// list also skips fenced blocks without a language.
    pub ignore_code_languages: Vec<String>,
    /// Number of spaces for each hard tab.
    pub spaces_per_tab: usize,
}

impl Default for Md010Options {
    fn default() -> Self {
        Self {
            code_blocks: true,
            ignore_code_languages: Vec::new(),
            spaces_per_tab: 1,
        }
    }
}

impl Rule for Md010 {
    type Options = Md010Options;

    const META: RuleMeta = RuleMeta {
        id: "MD010",
        aliases: &["no-hard-tabs"],
        tags: &["whitespace", "hard_tab"],
        description: "Hard tabs",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md010Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let ignored: Vec<String> = options
            .ignore_code_languages
            .iter()
            .map(|lang| lang.to_lowercase())
            .collect();
        // Whole lines (fenced and indented code) and byte ranges (code spans) to skip.
        let mut code_lines = vec![false; doc.source().line_count()];
        let mut code_spans = Vec::new();
        for id in ast.descendants(ast.root()) {
            let span = ast.span(id);
            match ast.kind(id) {
                NodeKind::CodeBlock(block) => {
                    let skip = match &block.fence {
                        Some(_) if !options.code_blocks => true,
                        Some(_) => {
                            !ignored.is_empty()
                                && block
                                    .lang()
                                    .is_none_or(|lang| ignored.contains(&lang.to_lowercase()))
                        }
                        None => !options.code_blocks,
                    };
                    if skip {
                        let (first, last) = span_lines(doc, span);
                        let (first, last) = match block.fence {
                            Some(_) => (first + 1, last.saturating_sub(1)),
                            None => (first, last),
                        };
                        for line in code_lines.iter_mut().take(last + 1).skip(first) {
                            *line = true;
                        }
                    }
                }
                NodeKind::Code(_) if !options.code_blocks => code_spans.push(span),
                _ => {}
            }
        }
        let mut diagnostics = Vec::new();
        for (line, range) in lines(doc) {
            if code_lines[line] {
                continue;
            }
            let content = &text[range.start..range.end];
            let mut search = 0;
            while let Some(found) = content[search..].find('\t') {
                let start = search + found;
                let len = content[start..].len() - content[start..].trim_start_matches('\t').len();
                search = start + len;
                let tabs = ByteRange::new(range.start + start, range.start + start + len);
                // markdownlint compares inclusive positions, so a tab right after a
                // code span counts as inside it.
                if code_spans
                    .iter()
                    .any(|code| code.start < tabs.end && tabs.start <= code.end)
                {
                    continue;
                }
                let detail = format!("Column: {}", js_len(&content[..start]) + 1);
                let fix = Fix::new(
                    "Replace the hard tabs with spaces",
                    vec![edit(tabs, " ".repeat(len * options.spaces_per_tab))],
                );
                diagnostics.push(
                    Self::META
                        .diagnostic(doc, tabs, message(&Self::META, Some(&detail), None))
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}
