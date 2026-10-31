//! Helpers shared by the TUI tests: documents with names, an app over them, key events
//! and drawing to a `TestBackend`.

#![allow(dead_code, reason = "each test binary uses a different subset")]

use std::path::PathBuf;

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_core::source::SourceFile;
use mrq_render::Theme;
use mrq_render::ansi::{AnsiOptions, AnsiRenderer};
use mrq_tui::{App, Event};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub const GUIDE: &str = "\
# Guide

An introduction that is long enough to wrap when the document panel gets narrow.

## Install

Run the installer.

## Usage

### Options

Pass `--help` for the options.
";

pub const NOTES: &str = "\
# Notes

- one
- two
";

/// `text` parsed as the file `name`.
pub fn doc(name: &str, text: &str) -> Document {
    DocumentOptions::new()
        .parse_source(SourceFile::new(Some(PathBuf::from(name)), text.to_owned()))
        .unwrap()
}

/// An app over `docs` with the default theme, sized `width`×`height`.
pub fn app(docs: Vec<Document>, width: u16, height: u16) -> App {
    let renderer = AnsiRenderer::new(&Theme::default(), AnsiOptions::default()).unwrap();
    let mut app = App::new(docs, renderer);
    app.handle(Event::Resize(width, height));
    app
}

/// An app over `guide.md` alone.
pub fn guide(width: u16, height: u16) -> App {
    app(vec![doc("guide.md", GUIDE)], width, height)
}

/// An app over `guide.md`, `notes.md` and `todo.md`.
pub fn several(width: u16, height: u16) -> App {
    app(
        vec![
            doc("guide.md", GUIDE),
            doc("notes.md", NOTES),
            doc("todo.md", "# Todo\n\n- [ ] write\n"),
        ],
        width,
        height,
    )
}

/// A key press without modifiers.
pub fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// A key press with `modifiers`.
pub fn key_with(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

/// A key release, which apps ignore.
pub fn release(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new_with_kind(
        code,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ))
}

/// Sends every event in `events` to `app`.
pub fn press(app: &mut App, events: impl IntoIterator<Item = Event>) {
    for event in events {
        app.handle(event);
    }
}

/// `app` drawn on a terminal of its size, as text.
pub fn draw(app: &App) -> String {
    let (width, height) = app.size();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| mrq_tui::ui::draw(frame, app))
        .unwrap();
    terminal.backend().to_string()
}
