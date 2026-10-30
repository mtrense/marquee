//! MD056 `table-column-count`.

use std::cmp::Ordering;

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{expected_actual, message, table_lines};

/// Table column count.
///
/// Every row of a table (the delimiter row included) must have as many cells as its
/// first row; cells are divided by unescaped pipes, also in code spans. Too few cells
/// are reported at the row's end, too many from the first extra cell on. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md056;

impl Rule for Md056 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD056",
        aliases: &["table-column-count"],
        tags: &["table"],
        description: "Table column count",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for table in ast.descendants(ast.root()) {
            if !matches!(ast.kind(table), NodeKind::Table(_)) {
                continue;
            }
            let mut expected = 0;
            for row in table_lines(doc, table) {
                let leading = usize::from(row.leading_pipe());
                let trailing = usize::from(row.trailing_pipe());
                let actual = (row.pipes.len() + 1).saturating_sub(leading + trailing);
                if expected == 0 {
                    expected = actual;
                }
                let end = row.content.end;
                let (detail, range) = match actual.cmp(&expected) {
                    Ordering::Less => {
                        let content = &text[row.content.start..end];
                        let last = end - content.chars().next_back().map_or(0, char::len_utf8);
                        (
                            "Too few cells, row will be missing data",
                            ByteRange::new(last, end),
                        )
                    }
                    Ordering::Greater => {
                        // A cell starts at the pipe before it.
                        let start = row.pipes[expected - 1 + leading];
                        (
                            "Too many cells, extra data will be missing",
                            ByteRange::new(start, end),
                        )
                    }
                    Ordering::Equal => continue,
                };
                let detail = format!("{}; {detail}", expected_actual(expected, actual));
                diagnostics.push(Self::META.diagnostic(
                    doc,
                    range,
                    message(&Self::META, Some(&detail), None),
                ));
            }
        }
        diagnostics
    }
}
