//! The document view without an app: the selectable blocks of a laid-out document, moving
//! the selection, scrolling, the outline entry of the selection, and selecting by node or
//! source offset.

mod common;

use std::fmt::Write as _;
use std::ops::Range;

use common::{GUIDE, doc, paragraphs};
use mrq_core::Document;
use mrq_render::Theme;
use mrq_render::ansi::{AnsiOptions, AnsiRenderer, Line};
use mrq_tui::DocumentView;

const MIXED: &str = "\
# Mixed

Intro paragraph.

- one
- two

| a | b |
| - | - |
| 1 | 2 |

> quoted
>
> twice

```rust
fn main() {}
```

## End

Last.
";

/// `text` laid out at `width` columns, with a view `height` rows high.
fn laid_out(text: &str, width: usize, height: usize) -> (Document, Vec<Line>, DocumentView) {
    let document = doc("test.md", text);
    let renderer = AnsiRenderer::new(&Theme::default(), AnsiOptions::default())
        .unwrap()
        .with_width(width);
    let lines = renderer.render(&document).lines;
    let mut view = DocumentView::new(&lines, document.ast(), &document.toc());
    view.set_height(height);
    (document, lines, view)
}

/// The text of block `index`, its lines joined by newlines.
fn text(lines: &[Line], view: &DocumentView, index: usize) -> String {
    lines[view.blocks()[index].lines.clone()]
        .iter()
        .map(|line| line.text().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text of the selected block.
fn selected(lines: &[Line], view: &DocumentView) -> String {
    text(lines, view, view.selected_index().unwrap())
}

#[test]
fn blocks_are_the_leaf_blocks_with_tables_whole() {
    let (document, lines, view) = laid_out(MIXED, 40, 10);
    let texts: Vec<String> = (0..view.blocks().len())
        .map(|i| text(&lines, &view, i))
        .collect();
    assert_eq!(texts.len(), 10, "{texts:#?}");
    assert!(
        texts[0].starts_with("# Mixed\n─"),
        "the heading has its rule"
    );
    assert_eq!(texts[1], "Intro paragraph.");
    assert!(texts[2].ends_with("one") && texts[3].ends_with("two"));
    // The borders and both rows.
    assert_eq!(texts[4].lines().count(), 5, "{}", texts[4]);
    assert!(texts[5].ends_with("quoted") && texts[6].ends_with("twice"));
    assert!(texts[7].contains("fn main() {}"));
    assert!(texts[8].starts_with("## End"));
    assert_eq!(texts[9], "Last.");
    // No blank line belongs to a block.
    for block in view.blocks() {
        assert!(
            lines[block.lines.clone()]
                .iter()
                .all(|l| !l.text().trim().is_empty())
        );
    }
    // Spans and headings come from the AST.
    let blocks = view.blocks();
    assert_eq!(
        &document.text()[Range::from(blocks[1].span)],
        "Intro paragraph."
    );
    assert!(document.text()[Range::from(blocks[4].span)].starts_with("| a | b |"));
    assert_eq!(blocks[0].heading.as_ref().map(|h| h.level), Some(1));
    assert_eq!(blocks[8].heading.as_ref().map(|h| h.level), Some(2));
    assert!(blocks[1].heading.is_none());
    // The section is the outline entry of the last heading at or before the block.
    let sections: Vec<_> = blocks.iter().map(|b| b.section).collect();
    let mut expected = vec![Some(0); 8];
    expected.extend([Some(1), Some(1)]);
    assert_eq!(sections, expected);
}

#[test]
fn starts_at_the_top_with_the_first_block_selected() {
    let (_, lines, view) = laid_out(MIXED, 40, 10);
    assert_eq!(view.top(), 0);
    assert_eq!(view.selected_index(), Some(0));
    assert!(selected(&lines, &view).starts_with("# Mixed"));
    assert_eq!(view.section(), Some(0));
    assert_eq!(view.visible(), 0..10);
    assert_eq!(view.line_count(), lines.len());
}

#[test]
fn arrow_keys_move_between_blocks_and_scroll_them_into_view() {
    let (_, lines, mut view) = laid_out(MIXED, 40, 6);
    view.down();
    assert_eq!(selected(&lines, &view), "Intro paragraph.");
    assert_eq!(view.top(), 0);
    for _ in 0..3 {
        view.down();
    }
    // The table does not fit below the list, so the view scrolls to its last line.
    assert_eq!(view.selected_index(), Some(4));
    let table = view.blocks()[4].lines.clone();
    assert_eq!(view.top(), table.end - 6);
    for _ in 0..5 {
        view.down();
    }
    assert_eq!(selected(&lines, &view), "Last.");
    assert_eq!(view.section(), Some(1));
    // At the last block, Down scrolls to the end and stops there.
    for _ in 0..10 {
        view.down();
    }
    assert_eq!(view.selected_index(), Some(9));
    assert_eq!(view.top(), lines.len() - 6);
    // Up comes back, revealing each block at the top.
    view.up();
    assert!(selected(&lines, &view).starts_with("## End"));
    for _ in 0..8 {
        view.up();
    }
    assert_eq!(view.selected_index(), Some(0));
    assert_eq!(view.top(), 0);
    view.up();
    assert_eq!((view.selected_index(), view.top()), (Some(0), 0));
}

#[test]
fn a_block_taller_than_the_view_scrolls_line_by_line() {
    let code = (0..20).fold(String::new(), |mut code, i| {
        writeln!(code, "line {i}").unwrap();
        code
    });
    let text = format!("Before.\n\n```\n{code}```\n\nAfter.\n");
    let (_, lines, mut view) = laid_out(&text, 40, 5);
    view.down();
    let code = view.blocks()[1].lines.clone();
    assert_eq!(view.selected_index(), Some(1));
    assert_eq!(view.top(), code.start, "the block's top is shown first");
    view.down();
    assert_eq!(view.selected_index(), Some(1), "still inside the block");
    assert_eq!(view.top(), code.start + 1);
    while view.selected_index() == Some(1) {
        view.down();
    }
    assert_eq!(selected(&lines, &view), "After.");
    assert!(view.visible().contains(&(code.end - 1)) || view.top() >= code.end - 1);
    // Up into the tall block shows its end first, then scrolls up through it.
    view.up();
    assert_eq!(view.selected_index(), Some(1));
    assert_eq!(view.visible().end, code.end);
    let top = view.top();
    view.up();
    assert_eq!((view.selected_index(), view.top()), (Some(1), top - 1));
}

#[test]
fn paging_scrolls_by_the_height_and_the_selection_follows() {
    let text = paragraphs(0..40);
    let (_, lines, mut view) = laid_out(&text, 40, 10);
    let max = lines.len() - 10;
    view.page_down();
    assert_eq!(view.top(), 10);
    // The first block that starts in view.
    assert_eq!(selected(&lines, &view), "Paragraph 5.");
    view.page_down();
    view.page_up();
    assert_eq!(view.top(), 10);
    assert_eq!(selected(&lines, &view), "Paragraph 5.");
    view.end();
    assert_eq!(view.top(), max);
    assert_eq!(selected(&lines, &view), "Paragraph 39.");
    view.page_down();
    assert_eq!(view.top(), max);
    view.home();
    assert_eq!((view.top(), view.selected_index()), (0, Some(0)));
    view.page_up();
    assert_eq!(view.top(), 0);
    // Paging at the end selects the last block, at the start the first.
    view.down();
    view.page_up();
    assert_eq!(view.selected_index(), Some(0));
    view.end();
    view.up();
    view.up();
    view.page_down();
    assert_eq!(selected(&lines, &view), "Paragraph 39.");
}

#[test]
fn a_selection_still_in_view_stays_when_paging() {
    let text = paragraphs(0..40);
    let (_, lines, mut view) = laid_out(&text, 40, 10);
    for _ in 0..7 {
        view.down();
    }
    // Paragraph 7 is on line 14, out of view after a page of 10 lines from line 6.
    assert_eq!(view.top(), 5);
    view.page_up();
    assert_eq!(view.top(), 0);
    assert_eq!(selected(&lines, &view), "Paragraph 0.");
    view.down();
    view.down();
    view.page_down();
    // Paragraph 2 (line 4) is gone; paragraph 5 is the first to start in view.
    assert_eq!(selected(&lines, &view), "Paragraph 5.");
    // Paragraph 36 (line 72) is still in view when the last page stops short.
    view.select(36);
    assert_eq!(view.top(), 63);
    view.page_down();
    assert_eq!(view.top(), lines.len() - 10);
    assert_eq!(selected(&lines, &view), "Paragraph 36.");
}

#[test]
fn the_selection_can_be_set_by_node_or_offset() {
    let (document, lines, mut view) = laid_out(MIXED, 40, 6);
    let end = document.toc()[1].node;
    assert!(view.select_node(end));
    assert!(selected(&lines, &view).starts_with("## End"));
    assert!(view.visible().contains(&view.blocks()[8].lines.start));
    assert!(!view.select_node(document.ast().root()));
    assert_eq!(view.selected_index(), Some(8));

    let offset = document.text().find("twice").unwrap();
    assert!(view.select_offset(offset + 2));
    assert!(selected(&lines, &view).ends_with("twice"));
    // Between blocks: the next one; past the end: the last one.
    let gap = document.text().find("\n\n- one").unwrap() + 1;
    assert!(view.select_offset(gap));
    assert!(selected(&lines, &view).ends_with("one"));
    assert!(view.select_offset(document.text().len() + 10));
    assert_eq!(selected(&lines, &view), "Last.");
    // Selecting moves the view only as far as needed.
    assert!(view.select(0));
    assert_eq!(view.top(), 0);
    assert!(!view.select(99));
}

#[test]
fn jump_selects_by_offset_and_scrolls_the_block_to_the_top() {
    let text = paragraphs(0..40);
    let (_, lines, mut view) = laid_out(&text, 40, 10);
    assert!(view.jump(text.find("Paragraph 20.").unwrap()));
    assert_eq!(selected(&lines, &view), "Paragraph 20.");
    assert_eq!(view.top(), view.selected().unwrap().lines.start);
    // Back up as well.
    assert!(view.jump(text.find("Paragraph 5.").unwrap()));
    assert_eq!(view.top(), view.selected().unwrap().lines.start);
    // Near the end only as far as the end allows.
    assert!(view.jump(text.find("Paragraph 39.").unwrap()));
    assert_eq!(selected(&lines, &view), "Paragraph 39.");
    assert_eq!(view.visible().end, view.line_count());
}

#[test]
fn the_height_keeps_the_selection_in_view() {
    let (_, _, mut view) = laid_out(MIXED, 40, 100);
    view.select(9);
    assert_eq!(view.top(), 0);
    view.set_height(3);
    let last = view.blocks()[9].lines.clone();
    assert!(view.visible().contains(&last.start), "{:?}", view.visible());
    view.set_height(100);
    assert_eq!(view.top(), 0, "no scrolling past the end");
}

#[test]
fn a_view_can_follow_another_layout() {
    let (document, _, mut wide) = laid_out(GUIDE, 80, 6);
    wide.select(4);
    let renderer = AnsiRenderer::new(&Theme::default(), AnsiOptions::default())
        .unwrap()
        .with_width(20);
    let lines = renderer.render(&document).lines;
    let narrow = DocumentView::new(&lines, document.ast(), &document.toc()).following(&wide);
    assert_eq!(
        narrow.selected().unwrap().node,
        wide.selected().unwrap().node
    );
    assert_eq!(narrow.height(), 6);
    // The selection stays where it was on screen.
    let row = |view: &DocumentView| view.selected().unwrap().lines.start - view.top();
    assert_eq!(row(&narrow), row(&wide));
}

#[test]
fn an_empty_document_scrolls_without_a_selection() {
    let (_, lines, mut view) = laid_out("", 40, 5);
    assert_eq!(lines, []);
    assert_eq!(view.selected_index(), None);
    assert_eq!(view.section(), None);
    for step in [
        DocumentView::down,
        DocumentView::up,
        DocumentView::page_down,
        DocumentView::page_up,
        DocumentView::home,
        DocumentView::end,
    ] {
        step(&mut view);
    }
    assert_eq!((view.top(), view.selected_index()), (0, None));
    assert!(!view.select_offset(0));
    // A zero height does not break anything either.
    let (_, _, mut view) = laid_out(MIXED, 40, 0);
    view.down();
    view.page_down();
    view.end();
    assert_eq!(view.selected_index(), Some(9));
}
