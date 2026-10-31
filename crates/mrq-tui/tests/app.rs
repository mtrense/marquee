//! App state without a terminal: focus switching, the outline toggle, the help overlay,
//! the file list, quitting, resizing, and the event loop on a `TestBackend`.

mod common;

use std::convert::Infallible;

use common::{app, doc, draw, guide, key, key_with, press, release, several};
use mrq_tui::{Event, Overlay, Panel};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn starts_on_the_first_file_with_the_document_focused() {
    let app = several(100, 20);
    assert_eq!(app.focus(), Panel::Document);
    assert_eq!(app.current(), 0);
    assert_eq!(app.file_cursor(), 0);
    assert!(app.outline_visible());
    assert_eq!(app.overlay(), None);
    assert!(!app.should_quit());
    assert_eq!(app.file().document().name().to_str(), Some("guide.md"));
}

#[test]
fn the_file_list_is_shown_only_for_several_files() {
    assert_eq!(guide(100, 20).panels(), [Panel::Outline, Panel::Document]);
    assert_eq!(
        several(100, 20).panels(),
        [Panel::Files, Panel::Outline, Panel::Document]
    );
}

#[test]
fn tab_cycles_through_the_visible_panels() {
    let mut app = several(100, 20);
    let mut seen = Vec::new();
    for _ in 0..4 {
        app.handle(key(KeyCode::Tab));
        seen.push(app.focus());
    }
    assert_eq!(
        seen,
        [Panel::Files, Panel::Outline, Panel::Document, Panel::Files]
    );
    app.handle(key(KeyCode::BackTab));
    assert_eq!(app.focus(), Panel::Document);
    app.handle(key_with(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.focus(), Panel::Outline);

    let mut app = guide(100, 20);
    press(&mut app, [key(KeyCode::Tab)]);
    assert_eq!(app.focus(), Panel::Outline);
    press(&mut app, [key(KeyCode::Tab)]);
    assert_eq!(app.focus(), Panel::Document);
}

#[test]
fn o_toggles_the_outline_and_moves_focus_off_it() {
    let mut app = guide(100, 20);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Char('o'))]);
    assert!(!app.outline_visible());
    assert_eq!(app.focus(), Panel::Document);
    assert_eq!(app.panels(), [Panel::Document]);
    // Tab has nowhere else to go.
    press(&mut app, [key(KeyCode::Tab)]);
    assert_eq!(app.focus(), Panel::Document);
    press(&mut app, [key(KeyCode::Char('o'))]);
    assert!(app.outline_visible());
    assert_eq!(app.focus(), Panel::Document);
}

#[test]
fn help_overlay_takes_the_keys_until_closed() {
    let mut app = several(100, 20);
    press(&mut app, [key(KeyCode::Char('?'))]);
    assert_eq!(app.overlay(), Some(Overlay::Help));
    // Panel keys do nothing, and q closes the overlay instead of quitting.
    press(
        &mut app,
        [
            key(KeyCode::Tab),
            key(KeyCode::Char('o')),
            key(KeyCode::Char('q')),
        ],
    );
    assert_eq!(app.focus(), Panel::Document);
    assert!(app.outline_visible());
    assert_eq!(app.overlay(), None);
    assert!(!app.should_quit());
    for close in [key(KeyCode::Char('?')), key(KeyCode::Esc)] {
        press(&mut app, [key(KeyCode::Char('?')), close]);
        assert_eq!(app.overlay(), None);
    }
}

#[test]
fn q_and_ctrl_c_quit() {
    let mut app = guide(80, 20);
    press(&mut app, [key(KeyCode::Char('q'))]);
    assert!(app.should_quit());

    let mut app = guide(80, 20);
    press(
        &mut app,
        [
            key(KeyCode::Char('?')),
            key_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ],
    );
    assert!(app.should_quit());
}

#[test]
fn key_releases_are_ignored() {
    let mut app = guide(80, 20);
    press(
        &mut app,
        [release(KeyCode::Tab), release(KeyCode::Char('q'))],
    );
    assert_eq!(app.focus(), Panel::Document);
    assert!(!app.should_quit());
}

#[test]
fn the_file_list_moves_its_cursor_and_opens_files() {
    let mut app = several(100, 20);
    press(&mut app, [key(KeyCode::Tab)]);
    assert_eq!(app.focus(), Panel::Files);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    assert_eq!(app.file_cursor(), 2);
    // The cursor stops at the ends.
    press(&mut app, [key(KeyCode::Down)]);
    assert_eq!(app.file_cursor(), 2);
    press(&mut app, [key(KeyCode::Up)]);
    assert_eq!(app.file_cursor(), 1);
    assert_eq!(app.current(), 0);
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.current(), 1);
    assert_eq!(app.file().document().name().to_str(), Some("notes.md"));
    assert_eq!(app.file().outline()[0].text, "Notes");
    // Arrow keys only move the focused panel's cursor.
    press(&mut app, [key(KeyCode::BackTab), key(KeyCode::Down)]);
    assert_eq!(app.file_cursor(), 1);
}

#[test]
fn the_outline_moves_its_cursor_when_focused() {
    let mut app = guide(100, 20);
    press(&mut app, [key(KeyCode::Down)]);
    assert_eq!(app.file().outline_cursor(), 0);
    press(
        &mut app,
        [key(KeyCode::Tab), key(KeyCode::Down), key(KeyCode::Down)],
    );
    assert_eq!(app.file().outline_cursor(), 2);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    assert_eq!(app.file().outline_cursor(), 3);
    press(&mut app, [key(KeyCode::Up)]);
    assert_eq!(app.file().outline_cursor(), 2);
}

#[test]
fn resizing_lays_the_document_out_again() {
    let mut app = guide(100, 20);
    let wide = app.file().lines().len();
    app.handle(Event::Resize(40, 20));
    assert_eq!(app.size(), (40, 20));
    let narrow = app.file().lines().len();
    assert!(narrow > wide, "{narrow} lines at 40 columns, {wide} at 100");
    let width = usize::from(app.document_width());
    assert!(app.file().lines().iter().all(|line| line.width() <= width));
    // Hiding the outline widens the document.
    press(&mut app, [key(KeyCode::Char('o'))]);
    assert!(usize::from(app.document_width()) > width);
}

#[test]
fn a_tiny_terminal_does_not_panic() {
    let mut app = several(0, 0);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Char('o'))]);
    assert_eq!(app.document_width(), 0);
    // TestBackend cannot show an empty buffer, so these are drawn.
    for (width, height) in [(1, 1), (5, 2), (12, 3)] {
        let mut app = several(width, height);
        press(&mut app, [key(KeyCode::Char('?'))]);
        let _ = draw(&app);
        press(&mut app, [key(KeyCode::Esc), key(KeyCode::Tab)]);
        let _ = draw(&app);
    }
}

#[test]
fn an_empty_document_has_no_outline() {
    let mut app = app(vec![doc("empty.md", "")], 60, 10);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Down)]);
    assert_eq!(app.file().outline(), []);
    assert_eq!(app.file().outline_cursor(), 0);
    let _ = draw(&app);
}

#[test]
fn the_event_loop_draws_until_quit() {
    let mut app = several(100, 20);
    let mut terminal = Terminal::new(TestBackend::new(90, 16)).unwrap();
    let events = [
        key(KeyCode::Tab),
        key(KeyCode::Char('q')),
        key(KeyCode::Tab),
    ];
    mrq_tui::run_on(
        &mut terminal,
        &mut app,
        events.into_iter().map(Ok::<_, Infallible>),
    )
    .unwrap();
    assert!(app.should_quit());
    // The loop stopped at q, and took the terminal's size.
    assert_eq!(app.focus(), Panel::Files);
    assert_eq!(app.size(), (90, 16));
    assert!(terminal.backend().to_string().contains("guide.md"));
}

/// An event source error.
#[derive(Debug, PartialEq)]
struct Broken;

impl From<Infallible> for Broken {
    fn from(never: Infallible) -> Self {
        match never {}
    }
}

#[test]
fn the_event_loop_ends_with_its_events_and_passes_errors_on() {
    let mut app = guide(80, 20);
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    mrq_tui::run_on(
        &mut terminal,
        &mut app,
        std::iter::empty::<Result<_, Infallible>>(),
    )
    .unwrap();
    assert!(!app.should_quit());

    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let result = mrq_tui::run_on(&mut terminal, &mut app, [Err(Broken)]);
    assert_eq!(result, Err(Broken));
}
