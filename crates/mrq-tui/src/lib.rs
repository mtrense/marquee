//! Terminal UI for `mrq` (`mrq show`): a viewer for one or more documents with an
//! outline sidebar and, for several files, a file list.
//!
//! The app is split so that everything but the terminal itself is testable:
//!
//! - [`App`] is the state. [`App::handle`] takes an [`Event`] (a key press or a resize)
//!   and updates it; keys are first mapped to an [`Action`] ([`event::action`]), so
//!   later key bindings only add actions.
//! - [`ui::draw`] renders an [`App`] into a ratatui [`Frame`](ratatui::Frame), so tests
//!   draw to a `TestBackend` and snapshot the text.
//! - [`run_on`] is the event loop over any backend and any source of events; [`run`]
//!   drives it on the real terminal with crossterm, restoring the terminal when it ends,
//!   fails or panics.
//!
//! ```
//! use mrq_core::Document;
//! use mrq_render::Theme;
//! use mrq_render::ansi::{AnsiOptions, AnsiRenderer};
//! use mrq_tui::{App, Event, Panel};
//! use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
//!
//! let renderer = AnsiRenderer::new(&Theme::default(), AnsiOptions::default()).unwrap();
//! let mut app = App::new(vec![Document::parse("# Title\n\nText.\n")], renderer);
//! app.handle(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
//! assert_eq!(app.focus(), Panel::Outline);
//! ```

use std::io;
use std::iter;

use ratatui::Terminal;
use ratatui::backend::Backend;

mod app;
pub mod event;
pub mod ui;

pub use app::{App, FileView, Overlay, Panel};
pub use event::{Action, Event};

/// Runs `app` on the terminal until it quits: raw mode on the alternate screen, events
/// from crossterm. The terminal is restored afterwards, also on an error or a panic.
pub fn run(app: &mut App) -> io::Result<()> {
    // Also installs a panic hook that restores the terminal before the panic message.
    let mut terminal = ratatui::try_init()?;
    let _restore = Restore;
    let events = iter::repeat_with(ratatui::crossterm::event::read)
        .filter_map(|read| read.map(|event| Event::from_crossterm(&event)).transpose());
    run_on(&mut terminal, app, events)
}

/// The event loop: syncs `app` with the terminal size, draws it, and hands it the next
/// event, until it quits or `events` ends. An error from the backend or from `events`
/// stops the loop and is returned.
pub fn run_on<B, E>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    events: impl IntoIterator<Item = Result<Event, E>>,
) -> Result<(), E>
where
    B: Backend,
    E: From<B::Error>,
{
    let mut events = events.into_iter();
    loop {
        let size = terminal.size()?;
        app.resize(size.width, size.height);
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.should_quit() {
            return Ok(());
        }
        let Some(event) = events.next() else {
            return Ok(());
        };
        app.handle(event?);
    }
}

/// Restores the terminal when dropped.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
