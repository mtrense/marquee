//! Search and query: the prompt that `/` and `>` open in the status line, and going
//! through the hits with `n`/`N`.
//!
//! A search runs as it is typed: the selection moves to the first hit at or after where
//! the search started (wrapping around), and back there while nothing matches. A query
//! runs on `Enter`; the text is the selector DSL, CSS after `css:` or jq after `jq:`.
//! A query that does not compile keeps the prompt open, with the error in the status
//! line and the cursor on the problem. `Enter` on a search or query that found something
//! is a jump, which `Backspace` comes back from; `Esc` restores the selection and the
//! previous search or query. `n` and `N` go on from the hit they went to last, or from
//! the selection if it moved off it, and are not jumps.

use std::mem;

use mrq_core::query::{Query, QueryError};
use ratatui::crossterm::event::KeyEvent;

use super::{App, Location, Panel};
use crate::find::Find;
use crate::prompt::{Outcome, Prompt};
use crate::view::DocumentView;

/// What a prompt is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    Search,
    Query,
    /// The question whether to quit with unsaved edits: `y` quits, any other key does not.
    Quit,
}

/// An open prompt and what to go back to on `Esc`.
#[derive(Debug)]
pub(super) struct Asking {
    pub(super) prompt: Prompt,
    purpose: Purpose,
    /// The shown file's view when the prompt opened.
    view: DocumentView,
    /// The search or query when the prompt opened.
    find: Option<Find>,
}

impl App {
    /// Opens the prompt for a search or query, focusing the document.
    pub(super) fn ask(&mut self, purpose: Purpose) {
        let label = if purpose == Purpose::Search { "/" } else { ">" };
        self.focus = Panel::Document;
        self.ask_with(purpose, label);
    }

    /// Opens the prompt for `purpose` after `label`.
    pub(super) fn ask_with(&mut self, purpose: Purpose, label: impl Into<String>) {
        self.asking = Some(Asking {
            prompt: Prompt::new(label),
            purpose,
            view: self.file().view.clone(),
            find: self.find.clone(),
        });
    }

    /// Hands `key` to the open prompt and acts on what it did.
    pub(super) fn prompt_key(&mut self, key: &KeyEvent) {
        let Some(asking) = &mut self.asking else {
            return;
        };
        match (asking.prompt.handle(key), asking.purpose) {
            (Outcome::Ignored | Outcome::Moved, Purpose::Quit) => {}
            (_, Purpose::Quit) => {
                self.quit = asking.prompt.text().eq_ignore_ascii_case("y");
                self.asking = None;
            }
            (Outcome::Cancel, _) => {
                if let Some(asking) = self.asking.take() {
                    self.restore(&asking);
                }
            }
            (Outcome::Changed, Purpose::Search) => self.search_as_typed(),
            (Outcome::Submit, Purpose::Search) => self.confirm_search(),
            (Outcome::Submit, Purpose::Query) => self.run_query(),
            _ => {}
        }
    }

    /// Searches for the prompt's text from where the search started.
    fn search_as_typed(&mut self) {
        let Some(asking) = self.asking.take() else {
            return;
        };
        self.restore_view(&asking.view);
        let pattern = asking.prompt.text().to_owned();
        self.asking = Some(asking);
        if pattern.is_empty() {
            self.find = None;
            return;
        }
        let mut find = Find::search(&pattern);
        let width = usize::from(self.document_width());
        // A search cannot fail.
        let _ = find.refresh(self.current, width, &self.files[self.current]);
        let empty = find.hits().is_empty();
        self.find = Some(find);
        if empty {
            self.status = Some("no match".to_owned());
        } else {
            self.go_to_hit(self.first_hit());
        }
    }

    /// Closes the search prompt, keeping the search if it found something.
    fn confirm_search(&mut self) {
        let Some(asking) = self.asking.take() else {
            return;
        };
        let pattern = asking.prompt.text();
        if pattern.is_empty() {
            return self.restore(&asking);
        }
        if self.find.as_ref().is_none_or(|find| find.hits().is_empty()) {
            self.restore_view(&asking.view);
            self.find = None;
            self.status = Some(format!("no match for `{pattern}`"));
            return;
        }
        self.remember(&asking.view);
        let find = self.find.as_ref().expect("checked above");
        if let Some(current) = find.current() {
            let count = find.hits().len();
            self.status = Some(format!("match {}/{count}", current + 1));
        }
    }

    /// Runs the query prompt's text: goes to its first hit and closes the prompt, or
    /// says why it cannot run and keeps it open.
    fn run_query(&mut self) {
        let Some(asking) = &mut self.asking else {
            return;
        };
        let text = asking.prompt.text().to_owned();
        if text.trim().is_empty() {
            if let Some(asking) = self.asking.take() {
                self.restore(&asking);
            }
            return;
        }
        let (prefix, compiled) = compile(&text);
        let query = match compiled {
            Ok(query) => query,
            Err(QueryError::Syntax(err)) => {
                asking.prompt.set_cursor(prefix + err.offset());
                let column = asking.prompt.cursor_column() + 1;
                let message = err.message();
                self.status = Some(format!("invalid query at column {column}: {message}"));
                return;
            }
            Err(err) => {
                self.status = Some(err.to_string());
                return;
            }
        };
        let mut find = Find::query(query, &text);
        let width = usize::from(self.document_width());
        if let Err(err) = find.refresh(self.current, width, &self.files[self.current]) {
            self.status = Some(err.to_string());
            return;
        }
        let Some(asking) = self.asking.take() else {
            return;
        };
        if find.hits().is_empty() {
            self.find = None;
            self.status = Some(format!("no match for `{text}`"));
            return;
        }
        self.find = Some(find);
        self.go_to_hit(self.first_hit());
        self.remember(&asking.view);
    }

    /// Goes `step` hits on (`1` or `-1`) from the last one gone to, wrapping around; from
    /// the selection instead if it is not on that hit.
    pub(super) fn next_match(&mut self, step: isize) {
        let Some(find) = &self.find else {
            self.status = Some("no search or query to repeat".to_owned());
            return;
        };
        let hits = find.hits();
        if hits.is_empty() {
            self.status = Some(format!("no match for `{}`", find.pattern()));
            return;
        }
        let view = &self.file().view;
        let lines = view.selected().map(|block| block.lines.clone());
        let index = match (find.current(), &lines) {
            (Some(current), Some(lines)) if lines.contains(&hits[current].line) => {
                (current.cast_signed() + step)
                    .rem_euclid(hits.len().cast_signed())
                    .cast_unsigned()
            }
            _ => {
                let start = lines.map_or(view.top(), |lines| lines.start);
                if step > 0 {
                    hits.iter().position(|hit| hit.line >= start).unwrap_or(0)
                } else {
                    let before = hits.partition_point(|hit| hit.line < start);
                    before.checked_sub(1).unwrap_or(hits.len() - 1)
                }
            }
        };
        self.go_to_hit(index);
    }

    /// The first hit at or after the selection, else the first one.
    fn first_hit(&self) -> usize {
        let view = &self.file().view;
        let start = view
            .selected()
            .map_or(view.top(), |block| block.lines.start);
        self.find
            .as_ref()
            .and_then(|find| find.hits().iter().position(|hit| hit.line >= start))
            .unwrap_or(0)
    }

    /// Selects hit `index` of the search or query, with `match i/n` in the status line.
    fn go_to_hit(&mut self, index: usize) {
        let Some(find) = &mut self.find else {
            return;
        };
        find.set_current(index);
        let line = find.hits()[index].line;
        let count = find.hits().len();
        self.link = None;
        let file = &mut self.files[self.current];
        file.view.select_line(line);
        file.sync_outline();
        self.focus = Panel::Document;
        self.status = Some(format!("match {}/{count}", index + 1));
    }

    /// Closes a prompt without its text: back to the selection and the search or query
    /// it opened with.
    fn restore(&mut self, asking: &Asking) {
        self.restore_view(&asking.view);
        self.find.clone_from(&asking.find);
        let width = usize::from(self.document_width());
        if let Some(find) = &mut self.find {
            let _ = find.refresh(self.current, width, &self.files[self.current]);
        }
    }

    /// Selects in the shown file what `old` had selected, at the same row; `old` may be
    /// of another layout if the terminal was resized meanwhile.
    fn restore_view(&mut self, old: &DocumentView) {
        let file = &mut self.files[self.current];
        let height = file.view.height();
        file.view = mem::take(&mut file.view).following(old);
        file.view.set_height(height);
        file.sync_outline();
    }

    /// Remembers the selection of `old`, the view before a search or query, for
    /// `Backspace`, unless the selection is still there.
    fn remember(&mut self, old: &DocumentView) {
        let origin = Location {
            file: self.current,
            offset: old.selected().map_or(0, |block| block.span.start),
        };
        if origin != self.location() {
            self.history.push(origin);
        }
    }
}

/// The query a `>` prompt's text stands for: CSS after `css:`, jq after `jq:`, else the
/// selector DSL; with the length of the prefix.
fn compile(text: &str) -> (usize, Result<Query, QueryError>) {
    if let Some(css) = text.strip_prefix("css:") {
        ("css:".len(), Query::css(css))
    } else if let Some(jq) = text.strip_prefix("jq:") {
        ("jq:".len(), Query::jq(jq))
    } else {
        (0, Query::selector(text))
    }
}
