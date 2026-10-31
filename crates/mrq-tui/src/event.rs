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
    /// Quit, asking first if there are unsaved edits.
    Quit,
    /// Quit without asking.
    ForceQuit,
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
    /// Promote the section of the selection (or of the outline cursor while the outline
    /// is focused), subsections included, one level.
    Promote,
    /// Demote the section one level.
    Demote,
    /// Move the section before the previous section with the same parent.
    MoveSectionUp,
    /// Move the section after the next section with the same parent.
    MoveSectionDown,
    DeleteSection,
    /// Undo the last edit of the shown file.
    Undo,
    /// Redo the last undone edit of the shown file.
    Redo,
    /// Write the shown file, if it has unsaved edits.
    Save,
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
    ("Shift-← →", "promote / demote the section"),
    ("Alt-↑ ↓", "move the section up / down"),
    ("d / Delete", "delete the section"),
    ("u / Ctrl-r", "undo / redo"),
    ("w", "save"),
    ("o", "toggle the outline"),
    ("?", "toggle this help"),
    ("Esc", "close this help / clear the matches"),
    ("q / Ctrl-c", "quit (q asks if there are unsaved edits)"),
];

/// The action `key` stands for in the current state of `app`, if any. `Ctrl-c` always
/// quits; while an overlay is open, the other keys only close it, and while a prompt is
/// open, they are its text (see [`App::handle`]) and stand for nothing. `Shift-←`/`→`
/// and `Alt-↑`/`↓` edit; `Alt` with other keys stands for nothing.
pub fn action(key: &KeyEvent, app: &App) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(Action::ForceQuit),
            KeyCode::Char('r') if app.prompt().is_none() && app.overlay().is_none() => {
                Some(Action::Redo)
            }
            _ => None,
        };
    }
    if app.prompt().is_some() {
        return None;
    }
    if let Some(Overlay::Help) = app.overlay() {
        return matches!(key.code, KeyCode::Char('?' | 'q') | KeyCode::Esc)
            .then_some(Action::CloseOverlay);
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return match key.code {
            KeyCode::Up => Some(Action::MoveSectionUp),
            KeyCode::Down => Some(Action::MoveSectionDown),
            _ => None,
        };
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::Left => return Some(Action::Promote),
            KeyCode::Right => return Some(Action::Demote),
            _ => {}
        }
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
        KeyCode::Char('d') | KeyCode::Delete => Action::DeleteSection,
        KeyCode::Char('u') => Action::Undo,
        KeyCode::Char('w') => Action::Save,
        _ => return None,
    })
}
