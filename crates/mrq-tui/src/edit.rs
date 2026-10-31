//! Structural editing: promoting, demoting, moving and deleting the section at the
//! selection, and undoing and redoing it.
//!
//! An edit is [planned](plan) as [`TextEdit`]s on the document's source and nothing
//! else, so every byte outside the edited section stays as it was:
//!
//! - promote and demote shift every heading of the section, subsections included, by
//!   one level with [`shift_headings`] (M7.1), and are refused where a heading would
//!   leave levels 1–6;
//! - moving swaps the section with the previous or next one of the same parent section
//!   in the same container (the blank lines between them stay where they are);
//! - deleting removes the section with the blank lines after it (before it, for the last
//!   one in its container).
//!
//! The [`UndoStack`] applies a [`Change`], keeping its inverse ([`invert_edits`]), so
//! undoing and redoing give back the exact bytes of before and after. It also knows
//! whether the text is the one last saved.

use mrq_core::Document;
use mrq_core::ast::NodeKind;
use mrq_core::query::{self, Section};
use mrq_core::source::{ByteRange, EditError, TextEdit, apply_edits, invert_edits};
use mrq_core::transform::{ShiftOptions, shift_headings};

/// A structural edit of a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Structural {
    /// One level up (`##` to `#`), with the subsections.
    Promote,
    /// One level down (`#` to `##`), with the subsections.
    Demote,
    /// Before the previous sibling section.
    MoveUp,
    /// After the next sibling section.
    MoveDown,
    Delete,
}

/// A planned edit: what to change in the source, and what to say and select afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Edits against the document's source.
    pub edits: Vec<TextEdit>,
    /// The source offset to select in the edited text: the edited section's heading, or
    /// what took the place of a deleted section.
    pub select: usize,
    /// What the edit did, for the status line, e.g. ``moved `Usage` up``.
    pub message: String,
}

/// Plans `edit` on the section of `document` at the source offset `offset`: the
/// section of the heading starting there, else the innermost section around it.
///
/// # Errors
///
/// Why the edit cannot be made, for the status line: there is no section at `offset`,
/// a heading would leave levels 1–6, or there is no sibling to move past.
pub fn plan(document: &Document, offset: usize, edit: Structural) -> Result<Change, String> {
    let ast = document.ast();
    let sections = query::sections(ast);
    let index = sections
        .iter()
        .position(|s| s.heading().span().start == offset)
        .or_else(|| {
            let around = |s: &&Section<'_>| s.span().start <= offset && offset < s.span().end;
            let innermost = sections
                .iter()
                .filter(around)
                .min_by_key(|s| s.span().end - s.span().start)?;
            sections.iter().position(|s| s == innermost)
        })
        .ok_or("no section here: the selection is before the first heading")?;
    let section = sections[index];
    let name = section.heading().text();
    match edit {
        Structural::Promote => shift(document, section, -1, &name),
        Structural::Demote => shift(document, section, 1, &name),
        Structural::MoveUp => {
            let siblings = siblings(&sections, index);
            let at = siblings
                .iter()
                .position(|&s| s == index)
                .expect("among them");
            let Some(&previous) = at.checked_sub(1).and_then(|at| siblings.get(at)) else {
                return Err(format!("`{name}` is the first section already"));
            };
            let (edit, _) = swap(document.text(), sections[previous], section);
            Ok(Change {
                select: edit.range.start,
                edits: vec![edit],
                message: format!("moved `{name}` up"),
            })
        }
        Structural::MoveDown => {
            let siblings = siblings(&sections, index);
            let at = siblings
                .iter()
                .position(|&s| s == index)
                .expect("among them");
            let Some(&next) = siblings.get(at + 1) else {
                return Err(format!("`{name}` is the last section already"));
            };
            let (edit, select) = swap(document.text(), section, sections[next]);
            Ok(Change {
                select,
                edits: vec![edit],
                message: format!("moved `{name}` down"),
            })
        }
        Structural::Delete => {
            let range = deleted(section);
            Ok(Change {
                select: range.start,
                edits: vec![TextEdit {
                    range,
                    replacement: String::new(),
                }],
                message: format!("deleted `{name}` (u to undo)"),
            })
        }
    }
}

/// Promotes (`by` −1) or demotes (`by` 1) `section`, named `name`, unless one of its
/// headings is at the end of the levels already.
fn shift(document: &Document, section: Section<'_>, by: i32, name: &str) -> Result<Change, String> {
    let ast = document.ast();
    let span = section.span();
    let (limit, verb) = if by < 0 {
        (1, "promote")
    } else {
        (6, "demote")
    };
    let stuck = ast.descendants(ast.root()).find(|&id| {
        let start = ast.span(id).start;
        matches!(ast.kind(id), NodeKind::Heading(heading) if heading.level == limit)
            && span.start <= start
            && start < span.end
    });
    if let Some(stuck) = stuck {
        let heading = ast.node(stuck).text();
        return Err(if stuck == section.heading().id() {
            format!("cannot {verb} `{name}`: it is level {limit}")
        } else {
            format!("cannot {verb} `{name}`: `{heading}` is level {limit}")
        });
    }
    let shifted = shift_headings(
        document,
        ByteRange::new(span.start, span.end),
        ShiftOptions::by(by),
    );
    Ok(Change {
        edits: shifted.edits,
        select: span.start,
        message: format!("{verb}d `{name}`"),
    })
}

/// The indices into `sections` of the siblings of section `index`, itself included:
/// the sections in its container with the same parent section.
fn siblings(sections: &[Section<'_>], index: usize) -> Vec<usize> {
    let parent = |i: usize| {
        let section = sections[i];
        let start = section.span().start;
        let container = section.heading().parent();
        sections[..i]
            .iter()
            .rposition(|outer| outer.heading().parent() == container && start < outer.span().end)
    };
    let container = sections[index].heading().parent();
    let own = parent(index);
    (0..sections.len())
        .filter(|&i| sections[i].heading().parent() == container && parent(i) == own)
        .collect()
}

/// The edit that swaps `first` and the section right after it, `second`, in `text`,
/// keeping what is between them, and where `first` starts afterwards.
fn swap(text: &str, first: Section<'_>, second: Section<'_>) -> (TextEdit, usize) {
    let (a, b) = (first.span(), second.span());
    let gap = &text[a.end..b.start];
    let second_text = &text[b.start..b.end];
    let edit = TextEdit {
        range: ByteRange::new(a.start, b.end),
        replacement: [second_text, gap, &text[a.start..a.end]].concat(),
    };
    (edit, a.start + second_text.len() + gap.len())
}

/// What deleting `section` removes: the section up to the next block of its container,
/// else from the end of the previous one, else just the section.
fn deleted(section: Section<'_>) -> ByteRange {
    let span = section.span();
    let last = section.nodes().last().expect("a section has its heading");
    if let Some(next) = last.next_sibling() {
        ByteRange::new(span.start, next.span().start)
    } else if let Some(previous) = section.heading().prev_sibling() {
        ByteRange::new(previous.span().end, span.end)
    } else {
        ByteRange::new(span.start, span.end)
    }
}

/// An edit, undo or redo made: the new text, the edits that made it, the source offset
/// to select and the edit's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub text: String,
    /// Edits against the text before, e.g. to move remembered offsets along.
    pub edits: Vec<TextEdit>,
    pub select: usize,
    /// The [`Change::message`] of the edit (also for its undo and redo).
    pub message: String,
}

/// One edit and how to take it back.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    /// Against the text before.
    forward: Vec<TextEdit>,
    /// Against the text after.
    backward: Vec<TextEdit>,
    /// The offsets to select before and after.
    before: usize,
    after: usize,
    message: String,
}

/// The edits made to a text, to undo and redo them, and which of its states was saved
/// last; see the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoStack {
    done: Vec<Step>,
    /// The latest undone last.
    undone: Vec<Step>,
    /// How many edits were done when the text was saved; `None` once that state cannot
    /// come back.
    saved: Option<usize>,
}

impl Default for UndoStack {
    /// No edits, and the text as saved.
    fn default() -> Self {
        Self {
            done: Vec::new(),
            undone: Vec::new(),
            saved: Some(0),
        }
    }
}

impl UndoStack {
    /// Applies `change` to `text`, remembering `before`, the offset selected before, for
    /// the undo. Nothing is left to redo afterwards.
    ///
    /// # Errors
    ///
    /// The edits of `change` do not apply to `text`.
    pub fn push(
        &mut self,
        text: &str,
        change: Change,
        before: usize,
    ) -> Result<Applied, EditError> {
        let edited = apply_edits(text, &change.edits)?;
        let backward = invert_edits(text, &change.edits)?;
        if self.saved.is_some_and(|saved| saved > self.done.len()) {
            self.saved = None;
        }
        self.undone.clear();
        self.done.push(Step {
            forward: change.edits.clone(),
            backward,
            before,
            after: change.select,
            message: change.message.clone(),
        });
        Ok(Applied {
            text: edited,
            edits: change.edits,
            select: change.select,
            message: change.message,
        })
    }

    /// Takes back the last edit of `text`, the text it gave; `None` without one.
    ///
    /// # Panics
    ///
    /// If `text` is not the text the last edit gave.
    pub fn undo(&mut self, text: &str) -> Option<Applied> {
        let step = self.done.pop()?;
        let applied = Applied {
            text: apply_edits(text, &step.backward).expect("an undo fits its edit's text"),
            edits: step.backward.clone(),
            select: step.before,
            message: step.message.clone(),
        };
        self.undone.push(step);
        Some(applied)
    }

    /// Makes the last undone edit again on `text`, the text its undo gave; `None` without
    /// one.
    ///
    /// # Panics
    ///
    /// If `text` is not the text the undo gave.
    pub fn redo(&mut self, text: &str) -> Option<Applied> {
        let step = self.undone.pop()?;
        let applied = Applied {
            text: apply_edits(text, &step.forward).expect("a redo fits its undo's text"),
            edits: step.forward.clone(),
            select: step.after,
            message: step.message.clone(),
        };
        self.done.push(step);
        Some(applied)
    }

    /// Whether the text differs from the one last saved (or loaded): edits were made or
    /// undone since.
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.done.len())
    }

    /// Notes that the current text was saved.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.done.len());
    }
}

/// Where the source offset `offset` is after `edits`: moved by the edits before it, or at
/// the start of the edit it is inside of. An insertion at `offset` goes before it.
pub(crate) fn map_offset(edits: &[TextEdit], offset: usize) -> usize {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|edit| (edit.range.start, edit.range.end));
    let (mut added, mut removed) = (0, 0);
    for edit in sorted {
        if edit.range.start > offset || (edit.range.start == offset && !edit.range.is_empty()) {
            break;
        }
        if edit.range.end > offset {
            return edit.range.start + added - removed;
        }
        added += edit.replacement.len();
        removed += edit.range.len();
    }
    offset + added - removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(start: usize, end: usize, replacement: &str) -> TextEdit {
        TextEdit {
            range: ByteRange::new(start, end),
            replacement: replacement.to_owned(),
        }
    }

    #[test]
    fn offsets_move_with_the_edits_before_them() {
        let edits = [edit(2, 4, "xyz"), edit(0, 0, "+"), edit(8, 9, "")];
        assert_eq!(map_offset(&edits, 0), 1);
        assert_eq!(map_offset(&edits, 1), 2);
        // Inside the replaced `2..4`: its start.
        assert_eq!(map_offset(&edits, 2), 3);
        assert_eq!(map_offset(&edits, 3), 3);
        assert_eq!(map_offset(&edits, 4), 6);
        assert_eq!(map_offset(&edits, 9), 10);
    }
}
