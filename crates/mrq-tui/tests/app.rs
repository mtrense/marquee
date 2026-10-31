//! App state without a terminal: focus switching, the outline toggle, the help overlay,
//! the file list, the document selection and scrolling with the outline following it,
//! quitting, resizing, and the event loop on a `TestBackend`.

mod common;

use std::convert::Infallible;

use common::{app, doc, draw, guide, key, key_with, press, release, selected, several};
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

/// The text of the first line of the selected block of the shown file.
#[test]
fn arrow_keys_move_the_selection_and_the_outline_follows() {
    let mut app = guide(100, 10);
    assert_eq!(selected(&app), "# Guide");
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    assert_eq!(selected(&app), "## Install");
    assert_eq!(app.file().outline_cursor(), 1);
    press(&mut app, [key(KeyCode::Down)]);
    assert_eq!(selected(&app), "Run the installer.");
    assert_eq!(app.file().outline_cursor(), 1);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    assert_eq!(selected(&app), "### Options");
    assert_eq!(app.file().outline_cursor(), 3);
    // The document scrolled to keep the selection in view.
    let view = app.file().view();
    assert!(view.top() > 0);
    assert!(
        view.visible()
            .contains(&view.selected().unwrap().lines.start)
    );
    press(
        &mut app,
        [key(KeyCode::Up), key(KeyCode::Up), key(KeyCode::Up)],
    );
    assert_eq!(selected(&app), "## Install");
    assert_eq!(app.file().outline_cursor(), 1);
}

#[test]
fn the_outline_cursor_moves_alone_until_the_document_moves() {
    let mut app = guide(100, 10);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Down)]);
    assert_eq!(app.file().outline_cursor(), 1);
    assert_eq!(selected(&app), "# Guide");
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Down)]);
    assert_eq!(app.file().outline_cursor(), 0);
}

#[test]
fn page_keys_scroll_the_document() {
    let mut app = guide(100, 10);
    let height = usize::from(app.document_height());
    assert_eq!(app.file().view().height(), height);
    press(&mut app, [key(KeyCode::PageDown)]);
    assert_eq!(app.file().view().top(), height);
    press(&mut app, [key(KeyCode::End)]);
    let view = app.file().view();
    assert_eq!(view.top(), app.file().lines().len() - height);
    assert_eq!(selected(&app), "Pass --help for the options.");
    assert_eq!(app.file().outline_cursor(), 3);
    press(&mut app, [key(KeyCode::PageUp), key(KeyCode::Home)]);
    assert_eq!(app.file().view().top(), 0);
    assert_eq!(selected(&app), "# Guide");
    assert_eq!(app.file().outline_cursor(), 0);
}

#[test]
fn page_keys_move_the_cursor_of_a_focused_list() {
    let mut app = guide(100, 10);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::End)]);
    assert_eq!(app.file().outline_cursor(), 3);
    press(&mut app, [key(KeyCode::Home)]);
    assert_eq!(app.file().outline_cursor(), 0);
    press(&mut app, [key(KeyCode::PageDown)]);
    assert_eq!(app.file().outline_cursor(), 3);
    press(&mut app, [key(KeyCode::PageUp)]);
    assert_eq!(app.file().outline_cursor(), 0);
    assert_eq!(app.file().view().top(), 0);
}

#[test]
fn resizing_keeps_the_selection() {
    let mut app = guide(100, 10);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    let node = app.file().view().selected().unwrap().node;
    for (width, height) in [(44, 10), (100, 6), (30, 30)] {
        app.handle(Event::Resize(width, height));
        let view = app.file().view();
        assert_eq!(view.selected().unwrap().node, node);
        assert_eq!(view.height(), usize::from(app.document_height()));
        assert!(
            view.visible()
                .contains(&view.selected().unwrap().lines.start)
        );
    }
}

#[test]
fn each_file_keeps_its_own_selection() {
    let mut app = several(100, 16);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    let open = |app: &mut mrq_tui::App, steps: KeyCode| {
        press(
            app,
            [
                key(KeyCode::Tab),
                key(steps),
                key(KeyCode::Enter),
                key(KeyCode::BackTab),
            ],
        );
    };
    open(&mut app, KeyCode::Down);
    assert_eq!(app.current(), 1);
    assert_eq!(selected(&app), "# Notes");
    open(&mut app, KeyCode::Up);
    assert_eq!(app.current(), 0);
    assert_eq!(selected(&app), "## Install");
}
