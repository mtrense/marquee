//! The external editor and live reload.
//!
//! `e` hands the shown file to the [`Editor`](crate::Editor) at the line of the selected
//! block, then reloads it. It refuses stdin input, and a file with unsaved edits: the
//! editor would not see them, and reloading afterwards would drop them, so they are to
//! be written (`w`) or dropped (`R`) first.
//!
//! A file reported changed ([`Event::Changed`](crate::Event::Changed)) is read again
//! and compared with the text last read from or written to it, so our own saves and
//! changes that leave the text as it was are ignored. Otherwise:
//!
//! - a file without unsaved edits is reloaded: parsed again (its flavor may have
//!   changed), laid out with the selection on the same line of text (moved with the
//!   lines added and removed before it, by a line diff) and on the same row of the
//!   screen, its undo history cleared;
//! - a file with unsaved edits keeps them, with a warning: `w` writes them over the
//!   change, `R` reloads it;
//! - a deleted file keeps its content, and `w` writes it back.
//!
//! `R` reloads the shown file whatever it holds. Over unsaved edits, the reload is an
//! edit on the undo stack, so `u` brings them back.

use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use mrq_core::Document;
use mrq_core::Slashed as _;
use mrq_core::source::{ByteRange, PositionEncoding, SourceFile, TextEdit};
use similar::{DiffTag, TextDiff};

use super::{App, FileView};
use crate::edit::{Change, UndoStack};

/// Why a file is read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// The watcher reported a change.
    Changed,
    /// The editor ran.
    Edited,
    /// `R`.
    Asked,
}

impl App {
    /// Opens `document`, after the open files, watching it; its index.
    pub(super) fn add_file(&mut self, document: Document) -> usize {
        self.files.push(FileView::new(document));
        let index = self.files.len() - 1;
        self.watch(index);
        index
    }

    /// Tells the watcher, if any, about file `index`, unless it has no file.
    pub(super) fn watch(&mut self, index: usize) {
        let document = &self.files[index].document;
        let (Some(watcher), Some(path)) = (&mut self.watcher, document.path()) else {
            return;
        };
        if let Err(err) = watcher.watch(path) {
            let name = document.name().slashed();
            self.status = Some(format!("cannot watch {name} for changes: {err}"));
        }
    }

    /// Edits the shown file in the editor at the selected block, then reloads it.
    pub(super) fn edit_externally(&mut self) {
        let file = self.file();
        let name = file.document.name().slashed();
        let Some(path) = file.document.path().map(Path::to_owned) else {
            self.status = Some(format!("cannot edit {name}: it has no file"));
            return;
        };
        if file.is_dirty() {
            self.status = Some(format!(
                "unsaved edits in {name}: w writes them, R drops them, then e edits the file"
            ));
            return;
        }
        let line = file
            .document
            .source()
            .offset_to_position(self.location().offset, PositionEncoding::Utf8)
            .map_or(1, |position| position.line + 1);
        let result = self.editor.edit(&path, line);
        self.repaint = true;
        // Also after a failure: the editor may have saved before it failed.
        self.reload(self.current, Why::Edited);
        if let Err(err) = result {
            self.status = Some(format!("editor: {err}"));
        }
    }

    /// Reloads the shown file from disk, also over unsaved edits.
    pub(super) fn reload_current(&mut self) {
        self.reload(self.current, Why::Asked);
    }

    /// Reloads the open file at `path`, if any, as the watcher reported it changed.
    pub(super) fn file_changed(&mut self, path: &Path) {
        if let Some(index) = self.open_index(path) {
            self.reload(index, Why::Changed);
        }
    }

    /// Reads file `index` again and shows what is on disk, unless that would overwrite
    /// unsaved edits it was not asked to; see the [module docs](self).
    fn reload(&mut self, index: usize, why: Why) {
        let file = &mut self.files[index];
        let name = file.document.name().slashed();
        let Some(path) = file.document.path().map(Path::to_owned) else {
            self.status = Some(format!("cannot reload {name}: it has no file"));
            return;
        };
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                file.deleted = true;
                self.status = Some(format!(
                    "{name} was deleted on disk; its content is kept (w writes it back)"
                ));
                return;
            }
            Err(err) => {
                self.status = Some(format!("cannot reload {name}: {err}"));
                return;
            }
        };
        file.deleted = false;
        if why == Why::Changed && text == file.disk {
            return;
        }
        file.disk.clone_from(&text);
        if text == file.document.text() {
            // What is shown is on disk now.
            file.undo.mark_saved();
            file.changed_on_disk = false;
            if why != Why::Changed {
                self.status = Some(format!("{name} is unchanged"));
            }
            return;
        }
        let dirty = file.is_dirty();
        if dirty && why == Why::Changed {
            file.changed_on_disk = true;
            self.status = Some(format!(
                "{name} changed on disk; your edits are kept: w writes them over it, R reloads it"
            ));
            return;
        }
        let source = SourceFile::new(Some(path), text.clone());
        let document = match self.workspace.options().parse_source(source) {
            Ok(document) => document,
            Err(err) => {
                self.status = Some(format!("cannot reload {name}: {err}"));
                return;
            }
        };
        let file = &mut self.files[index];
        let old = file.view.clone();
        let selected = old.selected().map_or(0, |block| block.span.start);
        let offset = follow_offset(file.document.source(), document.source(), selected);
        if dirty {
            let change = Change {
                edits: vec![TextEdit {
                    range: ByteRange::new(0, file.document.text().len()),
                    replacement: text,
                }],
                select: offset,
                message: format!("reloaded {name}"),
            };
            file.undo
                .push(file.document.text(), change, selected)
                .expect("the whole text fits it");
            file.undo.mark_saved();
            self.status = Some(format!("reloaded {name} (u brings your edits back)"));
        } else {
            file.undo = UndoStack::default();
            self.status = Some(format!("reloaded {name}"));
        }
        file.changed_on_disk = false;
        file.set_document(document);
        if index == self.current {
            self.link = None;
            if let Some(find) = &mut self.find {
                find.invalidate();
            }
            self.relayout();
        } else {
            let width = usize::from(self.document_width());
            let height = usize::from(self.document_height());
            self.files[index].lay_out(&self.renderer, width, height);
        }
        let file = &mut self.files[index];
        file.view = std::mem::take(&mut file.view).following_offset(&old, offset);
        file.sync_outline();
    }
}

/// How long the line diff of a reload may take; after that, it is coarser.
const DIFF_TIMEOUT: Duration = Duration::from_millis(50);

/// Where the source offset `offset` of `old` is in `new`, a changed version of it: on
/// the same line, moved with the lines added and removed before it, at the same column;
/// at the start of what replaced the line if it changed.
fn follow_offset(old: &SourceFile, new: &SourceFile, offset: usize) -> usize {
    let Ok(position) = old.offset_to_position(offset, PositionEncoding::Utf8) else {
        return offset;
    };
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(old.text(), new.text());
    let line = diff.ops().iter().find_map(|op| {
        let (from, to) = (op.old_range(), op.new_range());
        from.contains(&position.line).then(|| match op.tag() {
            DiffTag::Equal => (to.start + position.line - from.start, position.col),
            _ => (to.start, 0),
        })
    });
    // Past the last line: at the end.
    let Some((line, col)) = line else {
        return new.text().len();
    };
    new.line_range(line)
        .map_or(new.text().len(), |range| range.start + col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(text: &str) -> SourceFile {
        SourceFile::new(None, text)
    }

    #[test]
    fn offsets_follow_their_line_through_the_change() {
        let old = source("a\nbb\nccc\n");
        // The second and third `c` of `ccc`, after a line inserted before it and one
        // removed.
        let new = source("x\na\nccc\n");
        assert_eq!(follow_offset(&old, &new, 6), 5);
        assert_eq!(follow_offset(&old, &new, 7), 6);
        // A changed line: the start of what replaced it.
        let new = source("a\nBB!\nccc\n");
        assert_eq!(follow_offset(&old, &new, 3), 2);
        // The end of the text.
        assert_eq!(
            follow_offset(&old, &new, old.text().len()),
            new.text().len()
        );
    }
}
