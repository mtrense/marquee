//! Tables drawn with box-drawing characters, shrunk to the width by wrapping cells.

use mrq_core::ast::Alignment;

use super::wrap::{Piece, longest_word, natural_width, push_span, wrap};
use super::{StyledSpan, TextStyle};

/// A row's cells as inline pieces.
pub(super) struct Row {
    pub(super) header: bool,
    pub(super) cells: Vec<Vec<Piece>>,
}

/// The lines of a table at most `width` wide, each with the index of the row it shows
/// (`None` for borders). Columns get their natural widths when those fit; otherwise
/// the widest column gives up a column at a time (down to one), and cells wrap.
pub(super) fn table(
    rows: &[Row],
    alignments: &[Alignment],
    width: usize,
    border: TextStyle,
    header: TextStyle,
) -> Vec<(Option<usize>, Vec<StyledSpan>)> {
    let columns = rows
        .iter()
        .map(|row| row.cells.len())
        .max()
        .unwrap_or(0)
        .max(alignments.len())
        .max(1);
    let mut widths = vec![1; columns];
    let mut minimum = vec![1; columns];
    for row in rows {
        for (i, cell) in row.cells.iter().enumerate() {
            widths[i] = widths[i].max(natural_width(cell));
            minimum[i] = minimum[i].max(longest_word(cell));
        }
    }
    // `│ ` before each column, ` ` after it, and the closing `│`.
    let room = width.saturating_sub(3 * columns + 1).max(columns);
    // Shrink the widest column, first those still wider than their longest word.
    while widths.iter().sum::<usize>() > room {
        let pick = |floor: &dyn Fn(usize) -> usize| {
            (0..columns)
                .filter(|&i| widths[i] > floor(i))
                .max_by_key(|&i| (widths[i], std::cmp::Reverse(i)))
        };
        let Some(i) = pick(&|i| minimum[i]).or_else(|| pick(&|_| 1)) else {
            break;
        };
        widths[i] -= 1;
    }

    let rule = |left: char, middle: char, right: char| {
        let mut text = String::from(left);
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                text.push(middle);
            }
            text.extend(std::iter::repeat_n('─', w + 2));
        }
        text.push(right);
        (None, vec![StyledSpan::new(text, border)])
    };
    let mut lines = vec![rule('┌', '┬', '┐')];
    for (index, row) in rows.iter().enumerate() {
        let style = if row.header {
            header
        } else {
            TextStyle::default()
        };
        let cells: Vec<Vec<Vec<StyledSpan>>> = (0..columns)
            .map(|i| {
                let pieces: Vec<Piece> = row
                    .cells
                    .get(i)
                    .into_iter()
                    .flatten()
                    .map(|piece| match piece {
                        Piece::Text(span) => Piece::Text(StyledSpan {
                            style: style.or(span.style),
                            ..span.clone()
                        }),
                        Piece::Break => Piece::Break,
                    })
                    .collect();
                wrap(&pieces, widths[i])
            })
            .collect();
        let height = cells.iter().map(Vec::len).max().unwrap_or(1);
        for line in 0..height {
            let mut spans = Vec::new();
            for (i, cell) in cells.iter().enumerate() {
                push_span(
                    &mut spans,
                    StyledSpan::new(if i == 0 { "│ " } else { " │ " }, border),
                );
                let content = cell.get(line).cloned().unwrap_or_default();
                let used: usize = content.iter().map(StyledSpan::width).sum();
                let free = widths[i].saturating_sub(used);
                let (before, after) = match alignments.get(i) {
                    Some(Alignment::Right) => (free, 0),
                    Some(Alignment::Center) => (free / 2, free - free / 2),
                    _ => (0, free),
                };
                push_span(
                    &mut spans,
                    StyledSpan::new(" ".repeat(before), TextStyle::default()),
                );
                for span in content {
                    push_span(&mut spans, span);
                }
                push_span(
                    &mut spans,
                    StyledSpan::new(" ".repeat(after), TextStyle::default()),
                );
            }
            push_span(&mut spans, StyledSpan::new(" │", border));
            lines.push((Some(index), spans));
        }
        if row.header {
            lines.push(rule('├', '┼', '┤'));
        }
    }
    lines.push(rule('└', '┴', '┘'));
    lines
}
