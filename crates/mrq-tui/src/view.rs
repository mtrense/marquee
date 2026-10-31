//! The document panel's state: the blocks of a laid-out document that can be selected,
//! the selection, and the scroll position.
//!
//! A [`Block`] is a run of lines that shows one leaf block of the AST: a heading (with
//! its rule), a paragraph, a code block, a whole table, … Containers (lists, quotes,
//! callouts) are not blocks of their own, so the selection moves through a list item by
//! item and through a quote paragraph by paragraph; blank lines belong to no block.
//!
//! The selection is the one place later features act on: following a link in it,
//! highlighting matches, editing its section. It is kept by [`NodeId`] across layouts
//! ([`DocumentView::following`]) and can be set by node or by source offset, which is
//! how it finds its way back after an edit re-parses the document.

use std::collections::HashMap;
use std::ops::Range;

use mrq_core::ast::{Ast, NodeId, NodeKind, Span};
use mrq_core::toc::TocEntry;
use mrq_render::ansi::{HeadingInfo, Line};

/// A selectable block: the lines that show one block of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The block node in the document's AST.
    pub node: NodeId,
    /// The block's span in the source.
    pub span: Span,
    /// Set for a heading.
    pub heading: Option<HeadingInfo>,
    /// Its lines in the layout.
    pub lines: Range<usize>,
    /// The index of the outline entry of the section the block is in: the last outline
    /// heading at or before it. `None` before the first heading.
    pub section: Option<usize>,
}

/// The blocks of a layout, which one is selected and which lines are on screen; see the
/// [module docs](self).
///
/// Moving the selection scrolls just enough to show the selected block, and scrolling
/// moves the selection to a block on screen once it scrolls out. Every step costs at
/// most a binary search, however long the document.
#[derive(Debug, Clone, Default)]
pub struct DocumentView {
    blocks: Vec<Block>,
    line_count: usize,
    /// `None` only without blocks.
    selected: Option<usize>,
    /// The first line on screen.
    top: usize,
    /// The rows on screen.
    height: usize,
}

impl DocumentView {
    /// The view of `lines`, a layout of the document with the AST `ast` and the outline
    /// `outline`, scrolled to the top with the first block selected and no rows yet
    /// ([`set_height`](Self::set_height)).
    pub fn new(lines: &[Line], ast: &Ast, outline: &[TocEntry]) -> Self {
        let entries: HashMap<NodeId, usize> = outline
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.node, index))
            .collect();
        let mut blocks: Vec<Block> = Vec::new();
        let mut section = None;
        for (i, line) in lines.iter().enumerate() {
            let Some(info) = &line.block else { continue };
            let Some(node) = selectable(ast, info.node) else {
                continue;
            };
            if let Some(last) = blocks.last_mut()
                && last.node == node
                && last.lines.end == i
            {
                last.lines.end = i + 1;
                continue;
            }
            if let Some(&index) = entries.get(&node) {
                section = Some(index);
            }
            blocks.push(Block {
                node,
                span: ast.span(node),
                heading: info.heading.clone(),
                lines: i..i + 1,
                section,
            });
        }
        Self {
            selected: (!blocks.is_empty()).then_some(0),
            blocks,
            line_count: lines.len(),
            top: 0,
            height: 0,
        }
    }

    /// This view with the selection, its row on screen and the height of `old`, a view
    /// of an earlier layout of the same document (another width). Without the old
    /// selected node, the selection stays on the first block.
    #[must_use]
    pub fn following(mut self, old: &Self) -> Self {
        self.height = old.height;
        if let Some(block) = old.selected()
            && let Some(index) = self.blocks.iter().position(|b| b.node == block.node)
        {
            let row = block.lines.start.saturating_sub(old.top);
            self.selected = Some(index);
            self.top = self.blocks[index].lines.start.saturating_sub(row);
        }
        self.clamp();
        self.reveal(false);
        self
    }

    /// This view with the block at the source offset `offset` selected as
    /// [`select_offset`](Self::select_offset) does, on the row of the screen that the
    /// selection of `old` was on (as far as the ends allow), and the height of `old`: the
    /// view of an edited document, whose nodes are not those of `old`.
    #[must_use]
    pub fn following_offset(mut self, old: &Self, offset: usize) -> Self {
        self.height = old.height;
        let row = old
            .selected()
            .map_or(0, |block| block.lines.start.saturating_sub(old.top));
        if self.select_offset(offset)
            && let Some(block) = self.selected()
        {
            self.top = block.lines.start.saturating_sub(row);
        }
        self.clamp();
        self.reveal(false);
        self
    }

    /// The blocks, in document order.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The index of the selected block; `None` only without blocks.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected(&self) -> Option<&Block> {
        self.selected.map(|i| &self.blocks[i])
    }

    /// The outline entry of the selected block's section; see [`Block::section`].
    pub fn section(&self) -> Option<usize> {
        self.selected()?.section
    }

    /// The first line on screen.
    pub fn top(&self) -> usize {
        self.top
    }

    /// The rows on screen.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The lines of the layout.
    pub fn line_count(&self) -> usize {
        self.line_count
    }

    /// The lines on screen.
    pub fn visible(&self) -> Range<usize> {
        self.top..(self.top + self.height).min(self.line_count)
    }

    /// Gives the view `height` rows, scrolling to keep the selection on screen.
    pub fn set_height(&mut self, height: usize) {
        self.height = height;
        self.clamp();
        self.reveal(false);
    }

    /// Selects block `index` and scrolls it into view; `false` if there is no such
    /// block.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.blocks.len() {
            return false;
        }
        let up = self.selected.is_some_and(|s| index < s);
        self.selected = Some(index);
        self.reveal(up);
        true
    }

    /// Selects the block of `node`; `false` if `node` is not a block of this view.
    pub fn select_node(&mut self, node: NodeId) -> bool {
        self.blocks
            .iter()
            .position(|b| b.node == node)
            .is_some_and(|index| self.select(index))
    }

    /// Selects the innermost block whose span contains the source offset `offset`, else
    /// the first one after it, else the last one; `false` without blocks.
    pub fn select_offset(&mut self, offset: usize) -> bool {
        let containing = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.span.start <= offset && offset < b.span.end)
            .min_by_key(|(_, b)| b.span.end - b.span.start)
            .map(|(i, _)| i);
        let index = containing
            .or_else(|| self.blocks.iter().position(|b| b.span.start >= offset))
            .or_else(|| self.blocks.len().checked_sub(1));
        index.is_some_and(|index| self.select(index))
    }

    /// Selects the block at the source offset `offset` as [`select_offset`] does, and
    /// scrolls it to the top of the screen, as far as the end of the document allows:
    /// where a followed link or an outline entry lands. `false` without blocks.
    ///
    /// [`select_offset`]: Self::select_offset
    pub fn jump(&mut self, offset: usize) -> bool {
        if !self.select_offset(offset) {
            return false;
        }
        if let Some(block) = self.selected() {
            self.top = block.lines.start;
        }
        self.clamp();
        true
    }

    /// Selects the block that line `line` of the layout is in, else the first one after
    /// it, else the last one, and scrolls as [`select`](Self::select) does, then on to
    /// the line if it is still off screen (in a block taller than the screen): where a
    /// search or query match lands. `false` without blocks.
    pub fn select_line(&mut self, line: usize) -> bool {
        let index = self
            .block_at(line)
            .or_else(|| self.blocks.iter().position(|b| b.lines.start >= line))
            .or_else(|| self.blocks.len().checked_sub(1));
        let Some(index) = index else {
            return false;
        };
        let on_screen = |view: &Self| (view.top..view.top + view.height.max(1)).contains(&line);
        if self.selected == Some(index) && on_screen(self) {
            return true;
        }
        self.select(index);
        if !on_screen(self) {
            self.top = line;
            self.clamp();
        }
        true
    }

    /// Selects the next block, or scrolls one line down while the selected block goes on
    /// below the screen (or the last block is selected).
    pub fn down(&mut self) {
        let Some(selected) = self.selected else {
            return self.scroll_to(self.top + 1);
        };
        let bottom = self.top + self.height.max(1);
        if self.blocks[selected].lines.end > bottom || selected + 1 == self.blocks.len() {
            self.scroll_to(self.top + 1);
        } else {
            self.select(selected + 1);
        }
    }

    /// Selects the previous block, or scrolls one line up while the selected block starts
    /// above the screen (or the first block is selected).
    pub fn up(&mut self) {
        let Some(selected) = self.selected else {
            return self.scroll_to(self.top.saturating_sub(1));
        };
        if self.blocks[selected].lines.start < self.top || selected == 0 {
            self.scroll_to(self.top.saturating_sub(1));
        } else {
            self.select(selected - 1);
        }
    }

    /// Scrolls a screen down; at the end, selects the last block.
    pub fn page_down(&mut self) {
        if self.top == self.max_top() {
            self.select(self.blocks.len().saturating_sub(1));
        } else {
            self.scroll_to(self.top + self.height.max(1));
        }
    }

    /// Scrolls a screen up; at the start, selects the first block.
    pub fn page_up(&mut self) {
        if self.top == 0 {
            self.select(0);
        } else {
            self.scroll_to(self.top.saturating_sub(self.height.max(1)));
        }
    }

    /// Scrolls to the start and selects the first block.
    pub fn home(&mut self) {
        self.top = 0;
        self.select(0);
    }

    /// Scrolls to the end and selects the last block.
    pub fn end(&mut self) {
        self.top = self.max_top();
        self.select(self.blocks.len().saturating_sub(1));
    }

    /// The block whose lines contain `line`, if any.
    pub fn block_at(&self, line: usize) -> Option<usize> {
        let index = self.blocks.partition_point(|b| b.lines.end <= line);
        self.blocks
            .get(index)
            .is_some_and(|b| b.lines.contains(&line))
            .then_some(index)
    }

    /// The last valid top line: the one that puts the last line at the bottom.
    fn max_top(&self) -> usize {
        self.line_count.saturating_sub(self.height.max(1))
    }

    fn clamp(&mut self) {
        self.top = self.top.min(self.max_top());
    }

    /// Scrolls to `top` (clamped), then moves a selection that went off screen to the
    /// first block that starts on screen, or else the one the top line is in.
    fn scroll_to(&mut self, top: usize) {
        self.top = top;
        self.clamp();
        let Some(selected) = self.selected else {
            return;
        };
        let screen = self.top..self.top + self.height.max(1);
        let lines = &self.blocks[selected].lines;
        if lines.start < screen.end && screen.start < lines.end {
            return;
        }
        let first = self
            .blocks
            .partition_point(|b| b.lines.start < screen.start);
        let index = match self.blocks.get(first) {
            Some(block) if block.lines.start < screen.end => first,
            _ => self.block_at(screen.start).unwrap_or(first),
        };
        self.selected = Some(index.min(self.blocks.len() - 1));
    }

    /// Scrolls as little as possible to show the selected block: all of it if it fits,
    /// else its first lines, or its last ones when the selection moved `up`.
    fn reveal(&mut self, up: bool) {
        let Some(selected) = self.selected else {
            return;
        };
        let lines = self.blocks[selected].lines.clone();
        let height = self.height.max(1);
        if lines.start < self.top {
            self.top = if up {
                lines.start.max(lines.end.saturating_sub(height))
            } else {
                lines.start
            };
        } else if lines.end > self.top + height {
            self.top = if up {
                lines.end - height
            } else {
                lines.start.min(lines.end - height)
            };
        }
        self.clamp();
    }
}

/// The block a line of `node` belongs to: the outermost table around it, else `node`
/// itself unless it is a container (whose own lines are prefixes and separators).
fn selectable(ast: &Ast, node: NodeId) -> Option<NodeId> {
    let table = std::iter::once(node)
        .chain(ast.ancestors(node))
        .filter(|&id| matches!(ast.kind(id), NodeKind::Table(_)))
        .last();
    if table.is_some() {
        return table;
    }
    let container = matches!(
        ast.kind(node),
        NodeKind::Document
            | NodeKind::BlockQuote
            | NodeKind::List(_)
            | NodeKind::ListItem(_)
            | NodeKind::FootnoteDefinition(_)
            | NodeKind::DefinitionList
            | NodeKind::DefinitionItem
            | NodeKind::DefinitionTerm
            | NodeKind::DefinitionDetails
            | NodeKind::Admonition(_)
            | NodeKind::Callout(_)
    );
    (!container).then_some(node)
}
