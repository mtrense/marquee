//! Layout snapshots on a `TestBackend`: the panels for one and several files, the focused
//! panel (thick border), the outline toggle, the help overlay, and resizing.

mod common;

use common::{draw, guide, key, press, several};
use mrq_tui::Event;
use ratatui::crossterm::event::KeyCode;

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
    let mut app = several(100, 24);
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
