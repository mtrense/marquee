//! Drawing an [`App`]: where the panels go ([`layout`]) and what they show ([`draw`]).
//!
//! The screen is a sidebar on the left (the file list above the outline), the document
//! on the right and a status line at the bottom (the file name, or the app's status
//! message; the open prompt, with the status message on the right). The focused panel
//! has a thick border, and files with unsaved edits have a [`DIRTY_MARK`] after their
//! name in the file list and the status line.
//! The sidebar is left out below [`SIDEBAR_MIN_SCREEN`] columns. The document panel
//! draws only the lines on screen, behind a [`GUTTER`] that marks the selected block and
//! the blocks of query matches; search matches are highlighted in the text. Images and
//! diagrams are drawn over the rows their lines reserve, clipped to the panel; one that
//! cannot be drawn shows its text fallback on its first row.

use std::ops::Range;

use mrq_render::ansi::{self, ColorLevel, StyledSpan, TextStyle};
use mrq_render::theme::Color as ThemeColor;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph,
};

use crate::app::{App, FileView, Overlay, Panel};
use crate::event::KEYS;
use crate::find::Find;
use crate::graphics::{SignedPosition, SlicedImage};
use crate::prompt::Prompt;

use mrq_core::Slashed as _;

/// The narrowest terminal that gets a sidebar.
pub const SIDEBAR_MIN_SCREEN: u16 = 40;

/// The columns left of the document's lines, where a bar marks the selected block.
pub const GUTTER: u16 = 2;

/// The selection mark in the gutter.
const SELECTION_MARK: &str = "▌ ";

/// The gutter mark of the blocks of query matches.
const MATCH_MARK: &str = "│ ";

/// After the name of a file with unsaved edits.
pub const DIRTY_MARK: &str = "[+]";

/// A search match in the text.
const MATCH: Style = Style::new().fg(Color::Black).bg(Color::Yellow);

/// The search match `n`/`N` went to last.
const CURRENT_MATCH: Style = Style::new().fg(Color::Black).bg(Color::Cyan);

/// Where the panels of an app go; a panel that is not shown is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppLayout {
    pub files: Option<Rect>,
    pub outline: Option<Rect>,
    pub document: Rect,
    pub status: Rect,
}

/// The layout of `app` on `area`. The sidebar is a quarter of the width (16 to 30
/// columns); with both the file list and the outline in it, the file list gets the
/// rows its files need, up to half of them.
pub fn layout(app: &App, area: Rect) -> AppLayout {
    let [body, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let several = app.files().len() > 1;
    if area.width < SIDEBAR_MIN_SCREEN || !(several || app.outline_visible()) {
        return AppLayout {
            files: None,
            outline: None,
            document: body,
            status,
        };
    }
    let width = (area.width / 4).clamp(16, 30);
    let [sidebar, document] =
        Layout::horizontal([Constraint::Length(width), Constraint::Fill(1)]).areas(body);
    let (files, outline) = match (several, app.outline_visible()) {
        (true, true) => {
            let wanted = u16::try_from(app.files().len() + 2).unwrap_or(u16::MAX);
            let rows = wanted.min(sidebar.height / 2);
            let [files, outline] =
                Layout::vertical([Constraint::Length(rows), Constraint::Fill(1)]).areas(sidebar);
            (Some(files), Some(outline))
        }
        (true, false) => (Some(sidebar), None),
        _ => (None, Some(sidebar)),
    };
    AppLayout {
        files,
        outline,
        document,
        status,
    }
}

/// The content area of a bordered panel at `area`.
pub fn inner(area: Rect) -> Rect {
    Block::bordered().inner(area)
}

/// Draws `app` into `frame`.
pub fn draw(frame: &mut Frame, app: &App) {
    let layout = layout(app, frame.area());
    if let Some(area) = layout.files {
        draw_files(frame, app, area);
    }
    if let Some(area) = layout.outline {
        draw_outline(frame, app, area);
    }
    draw_document(frame, app, layout.document);
    draw_status(frame, app, layout.status);
    if app.overlay() == Some(Overlay::Help) {
        draw_help(frame, frame.area());
    }
}

/// A bordered panel, thick when focused.
fn panel(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().title(title);
    if focused {
        block
            .border_type(BorderType::Thick)
            .border_style(Style::new().fg(Color::Cyan))
    } else {
        block
    }
}

/// A list panel with the cursor at `cursor`, reversed when focused.
fn draw_list(
    frame: &mut Frame,
    items: Vec<ListItem>,
    title: &str,
    focused: bool,
    cursor: usize,
    area: Rect,
) {
    let highlight = if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let list = List::new(items)
        .block(panel(title.to_owned(), focused))
        .highlight_symbol("› ")
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_style(highlight);
    let mut state = ListState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_files(frame: &mut Frame, app: &App, area: Rect) {
    let items = app
        .files()
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let name = name(file);
            if i == app.current() {
                ListItem::new(name).style(Style::new().add_modifier(Modifier::BOLD))
            } else {
                ListItem::new(name)
            }
        })
        .collect();
    let focused = app.focus() == Panel::Files;
    draw_list(frame, items, "Files", focused, app.file_cursor(), area);
}

fn draw_outline(frame: &mut Frame, app: &App, area: Rect) {
    let file = app.file();
    let items = file
        .outline_text()
        .iter()
        .map(|line| ListItem::new(line.as_str()))
        .collect();
    let focused = app.focus() == Panel::Outline;
    draw_list(
        frame,
        items,
        "Outline",
        focused,
        file.outline_cursor(),
        area,
    );
}

/// The lines on screen, each behind the gutter, which has the selection mark on the
/// selected block's lines (cyan while the document is focused, else dark grey) and the
/// match mark on the other lines of query matches; search matches are highlighted.
fn draw_document(frame: &mut Frame, app: &App, area: Rect) {
    let file = app.file();
    let view = file.view();
    let focused = app.focus() == Panel::Document;
    let selected = view.selected().map_or(0..0, |block| block.lines.clone());
    let mark = Span::styled(
        SELECTION_MARK,
        Style::new().fg(if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }),
    );
    let matched = Span::styled(MATCH_MARK, Style::new().fg(Color::Yellow));
    let blank = Span::raw(" ".repeat(usize::from(GUTTER)));
    let find = app.find();
    let visible = view.visible();
    // The graphics with rows on screen, and how each is drawn: `None` shows its fallback.
    let graphics: Vec<_> = file
        .graphics_on_screen()
        .iter()
        .map(|&(at, key)| {
            let graphic = file.lines()[at].graphic.as_ref();
            let protocol = graphic.and_then(|graphic| app.images().protocol(key, graphic));
            (at, protocol)
        })
        .collect();
    let failed = |i: usize| graphics.iter().any(|(at, p)| *at == i && p.is_none());
    let lines: Vec<Line> = file.lines()[visible.clone()]
        .iter()
        .zip(visible)
        .map(|(styled, i)| {
            let marked = find
                .is_some_and(|find| view.block_at(i).is_some_and(|block| find.is_marked(block)));
            let gutter = if selected.contains(&i) {
                &mark
            } else if marked {
                &matched
            } else {
                &blank
            };
            let highlights = find.map_or_else(Vec::new, |find| highlights(find, i));
            let mut line = line(styled, app.color_level(), &highlights);
            line.spans.insert(0, gutter.clone());
            if failed(i) {
                line.spans.extend(fallback(styled, app.color_level()));
            }
            line
        })
        .collect();
    let title = file.document().name().slashed();
    frame.render_widget(Paragraph::new(lines).block(panel(title, focused)), area);
    let text = inner(area);
    let text = Rect {
        x: text.x + GUTTER.min(text.width),
        width: text.width.saturating_sub(GUTTER),
        ..text
    };
    for (at, protocol) in &graphics {
        let Some(protocol) = protocol else { continue };
        let prefix: usize = file.lines()[*at].spans.iter().map(StyledSpan::width).sum();
        let x = i16::try_from(prefix).unwrap_or(i16::MAX);
        if prefix >= usize::from(text.width) {
            continue;
        }
        // Negative for an image that starts above the screen.
        let y = i16::try_from(at.cast_signed() - view.top().cast_signed()).unwrap_or(i16::MIN);
        let position = SignedPosition::from((x, y));
        frame.render_widget(SlicedImage::new(protocol, position), text);
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(prompt) = app.prompt() {
        return draw_prompt(frame, prompt, app.status(), area);
    }
    let name = name(app.file());
    let left = if let Some(message) = app.status() {
        format!(" {message}")
    } else if app.files().len() > 1 {
        format!(" {name} ({}/{})", app.current() + 1, app.files().len())
    } else {
        format!(" {name}")
    };
    let style = Style::new().add_modifier(Modifier::REVERSED);
    frame.render_widget(Paragraph::new(left).style(style), area);
    frame.render_widget(
        Paragraph::new("? help  q quit ")
            .style(style)
            .right_aligned(),
        area,
    );
}

/// The file's name, with `[+]` after it while it has unsaved edits.
fn name(file: &FileView) -> String {
    let name = file.document().name().slashed();
    if file.is_dirty() {
        format!("{name} {DIRTY_MARK}")
    } else {
        name
    }
}

/// The prompt with the cursor in it, and `status` on the right of its text, cut off
/// where it does not fit.
fn draw_prompt(frame: &mut Frame, prompt: &Prompt, status: Option<&str>, area: Rect) {
    let text = format!("{}{}", prompt.label(), prompt.text());
    let width = u16::try_from(text.chars().count() + 1).unwrap_or(u16::MAX);
    let [left, right] =
        Layout::horizontal([Constraint::Length(width), Constraint::Fill(1)]).areas(area);
    frame.render_widget(Paragraph::new(text), left);
    if let Some(status) = status {
        let style = Style::new().add_modifier(Modifier::REVERSED);
        let status = format!(" {status} ");
        let paragraph = if status.chars().count() > usize::from(right.width) {
            Paragraph::new(status)
        } else {
            Paragraph::new(status).right_aligned()
        };
        frame.render_widget(paragraph.style(style), right);
    }
    let before = prompt.label().chars().count() + prompt.cursor_column();
    let x = area.x + u16::try_from(before).unwrap_or(u16::MAX);
    frame.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
}

/// The key bindings in a box in the middle of `area`.
fn draw_help(frame: &mut Frame, area: Rect) {
    let key_width = KEYS
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);
    let lines: Vec<Line> = KEYS
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::styled(
                    format!(" {key:<key_width$}  "),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(*action),
            ])
        })
        .collect();
    let width = lines.iter().map(Line::width).max().unwrap_or(0) + 3;
    let height = lines.len() + 2;
    let popup = area.centered(
        Constraint::Length(u16::try_from(width).unwrap_or(u16::MAX)),
        Constraint::Length(u16::try_from(height).unwrap_or(u16::MAX)),
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(panel("Help".to_owned(), true)),
        popup,
    );
}

/// The spans of a line of the ANSI layout that are drawn: what a search looks through.
/// A graphic is drawn over the line, after its spans (the prefix of its container).
pub(crate) fn drawn(line: &ansi::Line) -> &[StyledSpan] {
    &line.spans
}

/// The search matches of `find` on line `line`, with their styles.
fn highlights(find: &Find, line: usize) -> Vec<(Range<usize>, Style)> {
    let current = find.current().map(|index| &find.hits()[index]);
    find.hits_on(line)
        .iter()
        .map(|hit| {
            let style = if Some(hit) == current {
                CURRENT_MATCH
            } else {
                MATCH
            };
            (hit.columns.clone(), style)
        })
        .collect()
}

/// The text fallback of the graphic of `line`, with colours for `level`.
fn fallback(line: &ansi::Line, level: ColorLevel) -> impl Iterator<Item = Span<'_>> {
    line.graphic
        .iter()
        .flat_map(|graphic| &graphic.fallback)
        .map(move |span| Span::styled(span.text.as_str(), style(span.style, level)))
}

/// A styled line of the ANSI layout as a ratatui line, with colours for `level` and
/// `highlights` over it: byte ranges of its drawn text, in order and without overlaps.
fn line<'a>(
    line: &'a ansi::Line,
    level: ColorLevel,
    highlights: &[(Range<usize>, Style)],
) -> Line<'a> {
    let mut spans = Vec::new();
    let mut start = 0;
    for span in drawn(line) {
        let text = span.text.as_str();
        let base = style(span.style, level);
        let end = start + text.len();
        // The part of the span not drawn yet starts at `at`.
        let mut at = start;
        for (range, highlight) in highlights {
            if range.end <= at || range.start >= end {
                continue;
            }
            let (from, to) = (range.start.max(at), range.end.min(end));
            if from > at {
                spans.push(Span::styled(&text[at - start..from - start], base));
            }
            spans.push(Span::styled(
                &text[from - start..to - start],
                base.patch(*highlight),
            ));
            at = to;
        }
        if at < end {
            spans.push(Span::styled(&text[at - start..], base));
        }
        start = end;
    }
    Line::from(spans)
}

/// The ratatui style of `text`, with its colours for `level`. Without colours, bold,
/// italic and the other attributes stay: the panels around them use them as well.
fn style(text: TextStyle, level: ColorLevel) -> Style {
    let mut style = Style::new();
    if let Some(fg) = text.fg.and_then(|c| color(c, level)) {
        style = style.fg(fg);
    }
    if let Some(bg) = text.bg.and_then(|c| color(c, level)) {
        style = style.bg(bg);
    }
    for (on, modifier) in [
        (text.bold, Modifier::BOLD),
        (text.dim, Modifier::DIM),
        (text.italic, Modifier::ITALIC),
        (text.underline, Modifier::UNDERLINED),
        (text.strikethrough, Modifier::CROSSED_OUT),
        (text.reverse, Modifier::REVERSED),
    ] {
        if on {
            style = style.add_modifier(modifier);
        }
    }
    style
}

fn color(color: ThemeColor, level: ColorLevel) -> Option<Color> {
    match level {
        ColorLevel::TrueColor => Some(Color::Rgb(color.r, color.g, color.b)),
        ColorLevel::Ansi256 => Some(Color::Indexed(ansi::palette(color))),
        ColorLevel::None => None,
    }
}
