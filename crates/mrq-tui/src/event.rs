//! Input: the [`Event`]s the app reacts to, the [`Action`]s keys stand for, and the key
//! bindings ([`action`], listed for the help overlay in [`KEYS`]).

use ratatui::crossterm::event::{self as crossterm, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, Overlay};

/// Something that happened outside the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A key press, repeat or release; only presses and repeats do anything.
    Key(KeyEvent),
    /// The terminal is now this many columns and rows.
    Resize(u16, u16),
}

impl Event {
    /// The app's event for a crossterm event, `None` for those it ignores (mouse, focus,
    /// paste).
    pub fn from_crossterm(event: &crossterm::Event) -> Option<Self> {
        match event {
            crossterm::Event::Key(key) => Some(Self::Key(*key)),
            crossterm::Event::Resize(width, height) => Some(Self::Resize(*width, *height)),
            _ => None,
        }
    }
}

/// What a key does; see [`App::apply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Focus the next visible panel, in screen order, wrapping around.
    NextPanel,
    PreviousPanel,
    ToggleOutline,
    ToggleHelp,
    CloseOverlay,
    /// Move the selection in the document, or the cursor of the focused list, one step
    /// up; in a block taller than the screen, scroll a line.
    Up,
    Down,
    /// Scroll the document a screen up (the selection follows), or move the cursor of
    /// the focused list a screen up.
    PageUp,
    PageDown,
    /// Go to the start of the document or the focused list.
    Home,
    End,
    /// Open what the cursor of the focused panel is on: follow the chosen link of the
    /// selected block, jump to the outline entry's heading, or show the file.
    Open,
    /// Choose the next link of the selected block, wrapping around.
    NextLink,
    PreviousLink,
    /// Go back to where the last jump came from.
    Back,
    /// Open the search prompt.
    Search,
    /// Open the query prompt.
    Query,
    /// Go to the next match of the search or query, wrapping around.
    NextMatch,
    PreviousMatch,
    /// Forget the search or query and its highlighting.
    ClearMatches,
}

/// The key bindings for the help overlay: keys and what they do.
pub const KEYS: &[(&str, &str)] = &[
    ("Tab / Shift-Tab", "next / previous panel"),
    ("↑ ↓", "move the selection or the list cursor"),
    ("PgUp PgDn", "scroll a page"),
    ("Home End", "go to the start / end"),
    (
        "Enter",
        "follow the link / go to the heading / open the file",
    ),
    ("← →", "choose a link in the selected block"),
    ("Backspace", "go back"),
    ("/", "search (Enter: done, Esc: cancel)"),
    (">", "query (css: or jq: for those syntaxes)"),
    ("n / N", "next / previous match"),
    ("o", "toggle the outline"),
    ("?", "toggle this help"),
    ("Esc", "close this help / clear the matches"),
    ("q / Ctrl-c", "quit"),
];

/// The action `key` stands for in the current state of `app`, if any. `Ctrl-c` always
/// quits; while an overlay is open, the other keys only close it, and while a prompt is
/// open, they are its text (see [`App::handle`]) and stand for nothing.
pub fn action(key: &KeyEvent, app: &App) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return (key.code == KeyCode::Char('c')).then_some(Action::Quit);
    }
    if app.prompt().is_some() {
        return None;
    }
    if let Some(Overlay::Help) = app.overlay() {
        return matches!(key.code, KeyCode::Char('?' | 'q') | KeyCode::Esc)
            .then_some(Action::CloseOverlay);
    }
    Some(match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Tab => Action::NextPanel,
        KeyCode::BackTab => Action::PreviousPanel,
        KeyCode::Char('o') => Action::ToggleOutline,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Up => Action::Up,
        KeyCode::Down => Action::Down,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Home => Action::Home,
        KeyCode::End => Action::End,
        KeyCode::Enter => Action::Open,
        KeyCode::Right => Action::NextLink,
        KeyCode::Left => Action::PreviousLink,
        KeyCode::Backspace => Action::Back,
        KeyCode::Char('/') => Action::Search,
        KeyCode::Char('>') => Action::Query,
        KeyCode::Char('n') => Action::NextMatch,
        KeyCode::Char('N') => Action::PreviousMatch,
        KeyCode::Esc => Action::ClearMatches,
        _ => return None,
    })
}
