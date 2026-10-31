//! An input line: a label and a line of text with a cursor, edited with the keys of a
//! shell prompt. The app draws it in the status line and decides what its text means:
//! a search, a query, or an answer to a question.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A line of text being typed after a label, with a cursor.
///
/// Characters are inserted at the cursor, `Backspace` and `Delete` remove the one
/// before or under it, `←`/`→` move it a character and `Home`/`End` to the ends. `Enter`
/// and `Esc` only report [`Outcome::Submit`] and [`Outcome::Cancel`]: the text stays,
/// and the caller closes the prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prompt {
    label: String,
    text: String,
    /// A byte offset into `text`, on a character boundary.
    cursor: usize,
}

/// What a key did to a [`Prompt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The text changed.
    Changed,
    /// Only the cursor moved.
    Moved,
    /// `Enter`: the text is done.
    Submit,
    /// `Esc`: the prompt is to be closed without its text.
    Cancel,
    /// Nothing happened: a key without a meaning here, a release, or a move past an end.
    Ignored,
}

impl Prompt {
    /// An empty prompt after `label`.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    /// What is drawn before the text, e.g. `/`.
    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor as a byte offset into the [text](Self::text).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The cursor as the number of characters before it.
    pub fn cursor_column(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }

    /// Puts the cursor at the byte offset `offset`, or the character boundary before it,
    /// at most the end of the text.
    pub fn set_cursor(&mut self, offset: usize) {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        self.cursor = offset;
    }

    /// Edits the text or moves the cursor as `key` says; see the [type docs](Self).
    pub fn handle(&mut self, key: &KeyEvent) -> Outcome {
        if key.kind == KeyEventKind::Release
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Outcome::Ignored;
        }
        match key.code {
            KeyCode::Char(c) => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                Outcome::Changed
            }
            KeyCode::Backspace => match self.before() {
                Some(len) => {
                    self.cursor -= len;
                    self.text.remove(self.cursor);
                    Outcome::Changed
                }
                None => Outcome::Ignored,
            },
            KeyCode::Delete if self.cursor < self.text.len() => {
                self.text.remove(self.cursor);
                Outcome::Changed
            }
            KeyCode::Left => match self.before() {
                Some(len) => self.move_to(self.cursor - len),
                None => Outcome::Ignored,
            },
            KeyCode::Right => match self.text[self.cursor..].chars().next() {
                Some(c) => self.move_to(self.cursor + c.len_utf8()),
                None => Outcome::Ignored,
            },
            KeyCode::Home => self.move_to(0),
            KeyCode::End => self.move_to(self.text.len()),
            KeyCode::Enter => Outcome::Submit,
            KeyCode::Esc => Outcome::Cancel,
            _ => Outcome::Ignored,
        }
    }

    /// The length of the character before the cursor, if any.
    fn before(&self) -> Option<usize> {
        self.text[..self.cursor]
            .chars()
            .next_back()
            .map(char::len_utf8)
    }

    fn move_to(&mut self, offset: usize) -> Outcome {
        if offset == self.cursor {
            return Outcome::Ignored;
        }
        self.cursor = offset;
        Outcome::Moved
    }
}
