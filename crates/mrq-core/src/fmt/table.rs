//! The table printer: pipe tables with every column padded to its widest cell, measured
//! in display columns (`unicode-width`), so CJK text and emoji line up in a monospace
//! font.

use unicode_width::UnicodeWidthStr;

use super::Printer;
use crate::ast::{Alignment, NodeId};

/// The narrowest column: `:-:` needs three characters.
const MIN_WIDTH: usize = 3;

impl Printer<'_> {
    /// A pipe table, one row per line: the header, the delimiter row with each column's
    /// alignment marker, then the body. Every row has one cell per column (a missing cell
    /// is printed empty, one beyond the header's columns is dropped, as GFM does), and
    /// cells are padded on the side their alignment leaves open (but not for
    /// the diff).
    pub(super) fn table(&self, id: NodeId, alignments: &[Alignment]) -> Vec<String> {
        let columns = alignments.len();
        let rows: Vec<Vec<String>> = self
            .ast
            .children(id)
            .iter()
            .map(|&row| {
                let mut cells: Vec<String> = self
                    .ast
                    .children(row)
                    .iter()
                    .take(columns)
                    .map(|&cell| self.inlines(cell, true))
                    .collect();
                cells.resize(columns, String::new());
                cells
            })
            .collect();
        // Unpadded, cells are as printed and delimiter cells the narrowest.
        let mut widths = vec![MIN_WIDTH; columns];
        for row in rows.iter().filter(|_| !self.for_diff) {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.width());
            }
        }
        let mut lines = Vec::with_capacity(rows.len() + 1);
        for (index, row) in rows.iter().enumerate() {
            let cells =
                row.iter()
                    .zip(alignments)
                    .zip(&widths)
                    .map(|((cell, &alignment), &width)| {
                        if self.for_diff {
                            cell.clone()
                        } else {
                            pad(cell, alignment, width)
                        }
                    });
            lines.push(join(cells));
            if index == 0 {
                let delimiters = alignments
                    .iter()
                    .zip(&widths)
                    .map(|(&alignment, &width)| delimiter(alignment, width));
                lines.push(join(delimiters));
            }
        }
        lines
    }
}

/// `| a | b |`.
fn join(cells: impl Iterator<Item = String>) -> String {
    let mut line = String::from("|");
    for cell in cells {
        line.push(' ');
        line.push_str(&cell);
        line.push_str(" |");
    }
    line
}

/// `cell` padded with spaces to `width` columns: on the right unless it is right-aligned,
/// on both sides (one more on the right) when centered.
fn pad(cell: &str, alignment: Alignment, width: usize) -> String {
    let padding = width.saturating_sub(cell.width());
    let (left, right) = match alignment {
        Alignment::Right => (padding, 0),
        Alignment::Center => (padding / 2, padding - padding / 2),
        Alignment::None | Alignment::Left => (0, padding),
    };
    format!("{}{cell}{}", " ".repeat(left), " ".repeat(right))
}

/// The delimiter cell of a column `width` wide: dashes, with a `:` on the side(s) the
/// column is aligned to.
fn delimiter(alignment: Alignment, width: usize) -> String {
    let (left, right) = match alignment {
        Alignment::None => ("", ""),
        Alignment::Left => (":", ""),
        Alignment::Center => (":", ":"),
        Alignment::Right => ("", ":"),
    };
    let dashes = width - left.len() - right.len();
    format!("{left}{}{right}", "-".repeat(dashes))
}
