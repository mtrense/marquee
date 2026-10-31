//! Structural editing of the shown file, undo and redo, saving, and the question on quit.
//!
//! An edit acts on the section of the selection, or of the outline cursor while the
//! outline is focused ([`edit::plan`]). Each edit, undo and redo goes through the file's
//! [`UndoStack`](crate::edit::UndoStack), then the document is parsed again and laid
//! out, with the selection on the edited section at the row it was on. The search or
//! query is run again, the remembered locations in the file move with the edits, and the
//! chosen link is forgotten. `w` writes the file atomically (also when it has no edits
//! but was deleted on disk); `q` asks before quitting while a file has unsaved edits.

use mrq_core::source::write_atomic;

use super::search::Purpose;
use super::{App, Panel};
use crate::edit::{self, Applied, Structural};

impl App {
    /// Whether any open file has unsaved edits.
    pub fn is_dirty(&self) -> bool {
        self.files.iter().any(super::FileView::is_dirty)
    }

    /// Makes `edit` on the section at the selection, or says why it cannot.
    pub(super) fn structural(&mut self, edit: Structural) {
        let file = self.file();
        let offset = match file.outline.get(file.outline_cursor) {
            Some(entry) if self.focus == Panel::Outline => {
                file.document.ast().span(entry.node).start
            }
            _ => self.location().offset,
        };
        let change = match edit::plan(&file.document, offset, edit) {
            Ok(change) => change,
            Err(message) => {
                self.status = Some(message);
                return;
            }
        };
        let before = self.location().offset;
        let file = &mut self.files[self.current];
        let applied = file
            .undo
            .push(file.document.text(), change, before)
            .expect("a planned edit fits its document");
        self.status = Some(applied.message.clone());
        self.replace(applied);
    }

    /// Undoes the last edit of the shown file, or with `redo` redoes the last undone one.
    pub(super) fn undo(&mut self, redo: bool) {
        let file = &mut self.files[self.current];
        let text = file.document.text();
        let applied = if redo {
            file.undo.redo(text)
        } else {
            file.undo.undo(text)
        };
        let Some(applied) = applied else {
            self.status = Some(format!("nothing to {}", if redo { "redo" } else { "undo" }));
            return;
        };
        let done = if redo { "redone" } else { "undone" };
        self.status = Some(format!("{done}: {}", applied.message));
        self.replace(applied);
    }

    /// Shows the text of `applied` in place of the shown file's: parsed again, laid out
    /// with its selection, and the search, query and history brought along.
    fn replace(&mut self, applied: Applied) {
        self.link = None;
        for location in &mut self.history {
            if location.file == self.current {
                location.offset = edit::map_offset(&applied.edits, location.offset);
            }
        }
        let file = &mut self.files[self.current];
        let old = file.view.clone();
        file.set_document(file.document.with_text(applied.text));
        if let Some(find) = &mut self.find {
            find.invalidate();
        }
        self.relayout();
        let file = &mut self.files[self.current];
        file.view = std::mem::take(&mut file.view).following_offset(&old, applied.select);
        file.sync_outline();
    }

    /// Writes the shown file if it has unsaved edits, or was deleted on disk.
    pub(super) fn save(&mut self) {
        let file = &mut self.files[self.current];
        let name = file.document.name().display().to_string();
        self.status = Some(if !file.is_dirty() && !file.deleted {
            "no changes to write".to_owned()
        } else if let Some(path) = file.document.path() {
            match write_atomic(path, file.document.text()) {
                Ok(()) => {
                    file.undo.mark_saved();
                    file.disk = file.document.text().to_owned();
                    file.deleted = false;
                    file.changed_on_disk = false;
                    format!("wrote {name}")
                }
                Err(err) => format!("cannot write {name}: {err}"),
            }
        } else {
            format!("cannot write {name}: it has no file")
        });
    }

    /// Quits, or asks first if a file has unsaved edits.
    pub(super) fn ask_quit(&mut self) {
        let dirty: Vec<String> = self
            .files
            .iter()
            .filter(|file| file.is_dirty())
            .map(|file| file.document.name().display().to_string())
            .collect();
        if dirty.is_empty() {
            self.quit = true;
        } else {
            let label = format!(
                "unsaved changes in {}; quit anyway? (y/n) ",
                dirty.join(", ")
            );
            self.ask_with(Purpose::Quit, label);
        }
    }
}
