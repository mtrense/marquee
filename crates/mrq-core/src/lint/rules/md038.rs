//! MD038 `no-space-in-code`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, message};

/// Spaces inside code span elements.
///
/// Whitespace at the start or end of a code span's content is reported, beyond the
/// single space (or line ending) of padding `CommonMark` strips from both ends, and
/// beyond the one space needed next to a backtick in unpadded content. The fix deletes
/// it, and the padding too when both ends have extra whitespace.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md038;

impl Rule for Md038 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD038",
        aliases: &["no-space-in-code"],
        tags: &["whitespace", "code"],
        description: "Spaces inside code span elements",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Code(_)) {
                continue;
            }
            let span = ast.span(id);
            let code = &text[span.start..span.end];
            let fence = code.len() - code.trim_start_matches('`').len();
            if code.len() < fence * 2 {
                continue;
            }
            let content = ByteRange::new(span.start + fence, span.end - fence);
            for (range, start) in extra_spaces(&text[content.start..content.end]) {
                let range = ByteRange::new(content.start + range.start, content.start + range.end);
                let fix = Fix::new("Delete the spaces", vec![edit(range, "")]);
                let shown = ellipsify(code, start, !start);
                diagnostics.push(
                    Self::META
                        .diagnostic(doc, range, message(&Self::META, None, Some(&shown)))
                        .with_fix(fix),
                );
            }
        }
        diagnostics
    }
}

/// The whitespace to delete in a code span's `content` (offsets into it), each with
/// whether it is at the start, following micromark's tokens: one character of padding
/// at each end if both ends are a space or line ending and something else is between,
/// then data split at line endings.
fn extra_spaces(content: &str) -> Vec<(ByteRange, bool)> {
    let padding = |c: char| matches!(c, ' ' | '\r' | '\n');
    let padded = content.starts_with(padding)
        && content.ends_with(padding)
        && content.contains(|c| !padding(c));
    let ending_len = |ending: &str| match ending {
        _ if ending.starts_with("\r\n") => 2,
        _ => 1,
    };
    let (pad_start, pad_end) = if padded {
        let end = if content.ends_with("\r\n") { 2 } else { 1 };
        (ending_len(content), end)
    } else {
        (0, 0)
    };
    let data = ByteRange::new(pad_start, content.len() - pad_end);
    // The first and last data line (micromark's first and last `codeTextData`).
    let lines: Vec<ByteRange> = content[data.start..data.end]
        .split_inclusive(['\r', '\n'])
        .scan(data.start, |offset, line| {
            let start = *offset;
            *offset += line.len();
            let line = line.trim_end_matches(['\r', '\n']);
            Some(ByteRange::new(start, start + line.len()))
        })
        .filter(|line| !line.is_empty())
        .collect();
    let (Some(&first), Some(&last)) = (lines.first(), lines.last()) else {
        return Vec::new();
    };
    let first_text = &content[first.start..first.end];
    let last_text = &content[last.start..last.end];
    let lead = first_text.len() - first_text.trim_start().len();
    let trail = last_text.len() - last_text.trim_end().len();
    // Only whitespace before something else counts (`^\s+\S`, `\S\s+$`).
    let lead = if lead == first_text.len() { 0 } else { lead };
    let trail = if trail == last_text.len() { 0 } else { trail };
    let lead_backtick = lead > 0 && first_text[lead..].starts_with('`');
    let trail_backtick = trail > 0 && last_text[..last_text.len() - trail].ends_with('`');
    let start_count = lead - usize::from(lead_backtick && !padded);
    let end_count = trail - usize::from(trail_backtick && !padded);
    let remove_padding =
        start_count > 0 && end_count > 0 && padded && !lead_backtick && !trail_backtick;
    let mut spaces = Vec::new();
    if start_count > 0 {
        let from = if remove_padding { 0 } else { first.start };
        let len = start_count + if remove_padding { pad_start } else { 0 };
        spaces.push((ByteRange::new(from, from + len), true));
    }
    if end_count > 0 {
        let to = if remove_padding {
            content.len()
        } else {
            last.end
        };
        let len = end_count + if remove_padding { pad_end } else { 0 };
        spaces.push((ByteRange::new(to - len, to), false));
    }
    spaces
}
