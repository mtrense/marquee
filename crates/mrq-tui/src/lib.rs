//! Terminal UI for `mrq` (`mrq show`): a viewer for one or more documents with an
//! outline sidebar and, for several files, a file list.
//!
//! The app is split so that everything but the terminal itself is testable:
//!
//! - [`App`] is the state. [`App::handle`] takes an [`Event`] (a key press or a resize)
//!   and updates it; keys are first mapped to an [`Action`] ([`event::action`]), so
//!   later key bindings only add actions.
//! - [`DocumentView`] is the document panel's part of it: the selectable blocks of the
//!   laid-out document, the selection and the scroll position.
//! - Navigation is part of [`App`] too: `Enter` follows the chosen link of the selected
//!   block (`←`/`→` choose among several) to a heading, another file or, through the
//!   injectable [`Opener`], to the operating system; `Backspace` goes back through
//!   [`App::history`]. What went wrong is an [`App::status`] message.
//! - `/` and `>` open a [`Prompt`](prompt::Prompt) in the status line for a search or
//!   a query; their hits are the [`App::find`] ([`find`]), and `n`/`N` go through them.
//! - Structural edits (promote, demote, move and delete the section of the selection)
//!   are [`TextEdit`](mrq_core::source::TextEdit)s on the source, planned in [`edit`]
//!   and kept per file on an [`UndoStack`](edit::UndoStack) for `u`/`Ctrl-r`; `w` saves,
//!   and `q` asks before dropping unsaved edits.
//! - `e` hands the shown file to the injectable [`Editor`] ([`SystemEditor`] runs
//!   `$VISUAL` or `$EDITOR` on the suspended terminal) and reloads it afterwards. Files
//!   changed on disk come in as [`Event::Changed`] from a [`Watch`]
//!   ([`watch::FileWatcher`]) and are reloaded, unless that would drop unsaved edits.
//! - [`ui::draw`] renders an [`App`] into a ratatui [`Frame`](ratatui::Frame), so tests
//!   draw to a `TestBackend` and snapshot the text. The document panel draws only its
//!   visible lines, and the images and diagrams on them with the app's ratatui-image
//!   picker ([`graphics`]; [`App::set_picker`]), whose rows the layout reserves.
//! - [`run_on`] is the event loop over any backend and any source of events; [`run`]
//!   drives it on the real terminal with crossterm and the file watcher's changes,
//!   restoring the terminal when it ends, fails or panics.
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

use std::io::{self, IsTerminal};
use std::iter;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event as crossterm;

mod app;
pub mod edit;
pub mod editor;
pub mod event;
pub mod find;
pub mod graphics;
pub mod open;
pub mod prompt;
pub mod ui;
pub mod view;
pub mod watch;

pub use app::{App, FileView, Location, Overlay, Panel};
pub use editor::{Editor, SystemEditor};
pub use event::{Action, Event};
pub use graphics::Graphics;
pub use open::{Opener, SystemOpener};
pub use view::{Block, DocumentView};
pub use watch::Watch;

/// How often [`run`] looks for changed files while waiting for a key.
const POLL: Duration = Duration::from_millis(50);

/// Runs `app` on the terminal until it quits: raw mode on the alternate screen, events
/// from crossterm, images drawn with the picker its [`Graphics`] get from the terminal,
/// and the open files watched for changes (if the file system can be watched; else the
/// status line says why not). The terminal is restored afterwards,
/// also on an error or a panic.
pub fn run(app: &mut App) -> io::Result<()> {
    // Also installs a panic hook that restores the terminal before the panic message.
    let mut terminal = ratatui::try_init()?;
    let _restore = Restore;
    // In raw mode on the alternate screen, before any key is read. The answer comes on
    // stdin, so not when the document came from there.
    let picker = app.graphics().picker(|| {
        if io::stdin().is_terminal() {
            graphics::Picker::from_query_stdio()
        } else {
            Err(graphics::Errors::NoStdinResponse)
        }
    });
    app.set_picker(picker);
    let mut changes = match watch::FileWatcher::new() {
        Ok((watcher, changes)) => {
            app.set_watcher(watcher);
            Some(changes)
        }
        Err(err) => {
            app.say(format!("files are not reloaded on changes: {err}"));
            None
        }
    };
    // Keys are read on this thread, between events only, so none are taken from the
    // editor while it runs.
    let events =
        iter::repeat_with(move || next_event(changes.as_mut())).filter_map(Result::transpose);
    run_on(&mut terminal, app, events)
}

/// The next event from crossterm or, between keys, from `changes`; `None` for a
/// crossterm event the app ignores.
fn next_event(changes: Option<&mut watch::Changes>) -> io::Result<Option<Event>> {
    let Some(changes) = changes else {
        return Ok(Event::from_crossterm(&crossterm::read()?));
    };
    loop {
        if let Some(path) = changes.next(Instant::now()) {
            return Ok(Some(Event::Changed(path)));
        }
        if crossterm::poll(POLL)? {
            return Ok(Event::from_crossterm(&crossterm::read()?));
        }
    }
}

/// The event loop: syncs `app` with the terminal size, draws it, and hands it the next
/// event, until it quits or `events` ends. After the app handed the terminal to the
/// editor, the screen is cleared so that it is drawn afresh. An error from the backend
/// or from `events` stops the loop and is returned.
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
        if app.take_repaint() {
            terminal.clear()?;
        }
    }
}

/// Restores the terminal when dropped.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
