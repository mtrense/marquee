//! Drawing an [`App`]: where the panels go ([`layout`]) and what they show ([`draw`]).
//!
//! The screen is a sidebar on the left (the file list above the outline), the document
//! on the right and a status line at the bottom. The focused panel has a thick border.
//! The sidebar is left out below [`SIDEBAR_MIN_SCREEN`] columns.

use mrq_render::ansi::{self, StyledSpan, TextStyle};
use mrq_render::theme::Color as ThemeColor;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph,
};

use crate::app::{App, Overlay, Panel};
use crate::event::KEYS;

/// The narrowest terminal that gets a sidebar.
pub const SIDEBAR_MIN_SCREEN: u16 = 40;

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
            let name = file.document().name().display().to_string();
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
    // `outline` indents each heading under the closest lower-level one, one line each.
    let text = mrq_core::toc::outline(file.outline());
    let items = text
        .lines()
        .map(|line| ListItem::new(line.to_owned()))
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

fn draw_document(frame: &mut Frame, app: &App, area: Rect) {
    let file = app.file();
    let rows = usize::from(inner(area).height);
    let lines: Vec<Line> = file.lines().iter().take(rows).map(line).collect();
    let title = file.document().name().display().to_string();
    let block = panel(title, app.focus() == Panel::Document);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let name = app.file().document().name().display().to_string();
    let left = if app.files().len() > 1 {
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

/// A styled line of the ANSI layout as a ratatui line.
fn line(line: &ansi::Line) -> Line<'static> {
    let spans = match &line.graphic {
        // Images come with M10.7; until then, their text fallback.
        Some(graphic) => &graphic.fallback,
        None => &line.spans,
    };
    Line::from(spans.iter().map(span).collect::<Vec<_>>())
}

fn span(span: &StyledSpan) -> Span<'static> {
    Span::styled(span.text.clone(), style(span.style))
}

fn style(text: TextStyle) -> Style {
    let mut style = Style::new();
    if let Some(fg) = text.fg {
        style = style.fg(color(fg));
    }
    if let Some(bg) = text.bg {
        style = style.bg(color(bg));
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

const fn color(color: ThemeColor) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}
