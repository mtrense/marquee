//! Layout snapshots on a `TestBackend`: the panels for one and several files, the focused
//! panel (thick border), the outline toggle, the help overlay, resizing, the document
//! selection and scrolling; and the colours for each colour level.

mod common;

use common::{GUIDE, app, doc, draw, guide, key, paragraphs, press, several};
use mrq_render::ansi::ColorLevel;
use mrq_tui::Event;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;
use ratatui::style::Color;

#[test]
fn one_file() {
    insta::assert_snapshot!(draw(&guide(80, 16)));
}

#[test]
fn several_files() {
    insta::assert_snapshot!(draw(&several(100, 16)));
}

#[test]
fn file_list_focused() {
    let mut app = several(100, 16);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Down)]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn outline_focused() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Down)]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn outline_hidden() {
    let mut app = several(100, 16);
    press(&mut app, [key(KeyCode::Char('o'))]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn other_file_opened() {
    let mut app = several(100, 16);
    press(
        &mut app,
        [key(KeyCode::Tab), key(KeyCode::Down), key(KeyCode::Enter)],
    );
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn help_overlay() {
    let mut app = several(100, 26);
    press(&mut app, [key(KeyCode::Char('?'))]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn resized_narrower() {
    let mut app = guide(80, 16);
    app.handle(Event::Resize(56, 14));
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn too_narrow_for_the_sidebar() {
    insta::assert_snapshot!(draw(&several(36, 12)));
}

#[test]
fn selection_moved() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn selection_unfocused() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Tab)]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn scrolled_to_the_end() {
    let mut app = guide(80, 12);
    press(&mut app, [key(KeyCode::End)]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn scrolled_a_page_down() {
    let text = paragraphs(1..31);
    let mut app = app(vec![doc("long.md", &format!("# Long\n\n{text}"))], 60, 12);
    press(&mut app, [key(KeyCode::PageDown)]);
    insta::assert_snapshot!(draw(&app));
}

/// The colours of every cell of `app` drawn on a terminal of its size.
fn colors(app: &mrq_tui::App) -> Vec<Color> {
    let (width, height) = app.size();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| mrq_tui::ui::draw(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .flat_map(|cell| [cell.fg, cell.bg])
        .collect()
}

#[test]
fn colors_follow_the_color_level() {
    let text = format!("{GUIDE}\n```rust\nfn main() {{}}\n```\n");
    let rgb = |c: &Color| matches!(c, Color::Rgb(..));
    let indexed = |c: &Color| matches!(c, Color::Indexed(..));
    let shown = |level| app(vec![doc("code.md", &text)], 80, 30).with_color_level(level);

    let true_color = colors(&shown(ColorLevel::TrueColor));
    assert!(true_color.iter().any(rgb));
    let palette = colors(&shown(ColorLevel::Ansi256));
    assert!(!palette.iter().any(rgb));
    assert!(palette.iter().any(indexed));
    let none = colors(&shown(ColorLevel::None));
    assert!(!none.iter().any(|c| rgb(c) || indexed(c)));
}
