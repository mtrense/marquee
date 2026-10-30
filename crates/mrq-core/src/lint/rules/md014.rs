//! MD014 `commands-show-output`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, message, span_lines};

/// Dollar signs used before commands without showing output.
///
/// In a code block whose every non-blank line starts with `$` and whitespace, each
/// prompt is reported; the fix deletes it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md014;

impl Rule for Md014 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD014",
        aliases: &["commands-show-output"],
        tags: &["code"],
        description: "Dollar signs used before commands without showing output",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let source = doc.source();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::CodeBlock(block) = ast.kind(id) else {
                continue;
            };
            let (first, _) = span_lines(doc, ast.span(id));
            let first = first + usize::from(block.fence.is_some());
            // Each content line of the literal is the end of its source line, after
            // the container prefix and the block's indentation.
            let mut prompts = Vec::new();
            let mut all = true;
            for (index, code_line) in block.literal.lines().enumerate() {
                if code_line.is_empty() {
                    continue;
                }
                let Some(range) = source.line_range(first + index) else {
                    break;
                };
                let line = &text[range.start..range.end];
                let Some(value_start) = line.len().checked_sub(code_line.len()) else {
                    all = false;
                    break;
                };
                let value = &line[value_start..];
                let indent = value.len() - value.trim_start().len();
                let after = &value[indent..];
                let Some(command) = after.strip_prefix('$') else {
                    all = false;
                    break;
                };
                let space = command.len() - command.trim_start().len();
                if space == 0 {
                    all = false;
                    break;
                }
                let at = range.start + value_start + indent;
                prompts.push((ByteRange::new(at, at + 1 + space), value));
            }
            if !all {
                continue;
            }
            for (prompt, value) in prompts {
                let fix = Fix::new("Delete the prompt", vec![edit(prompt, "")]);
                let context = ellipsify(value, false, false);
                diagnostics.push(
                    Self::META
                        .diagnostic(doc, prompt, message(&Self::META, None, Some(&context)))
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}
