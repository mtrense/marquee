//! The app state: the open files, which one is shown, the focused panel, the outline
//! toggle, the overlay and the terminal size.

use mrq_core::Document;
use mrq_core::toc::TocEntry;
use mrq_render::ansi::{AnsiRenderer, Line, StyledDocument};
use ratatui::layout::Rect;

use crate::event::{self, Action, Event};
use crate::ui;

/// A panel that can have the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// The list of files, shown only for several files.
    Files,
    /// The headings of the shown file.
    Outline,
    /// The shown file.
    Document,
}

/// A popup drawn over the panels that takes the keys while open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
}

/// One open file: the document, its outline, and its layout for the document panel.
#[derive(Debug)]
pub struct FileView {
    document: Document,
    outline: Vec<TocEntry>,
    outline_cursor: usize,
    /// The document laid out, with the width it was laid out for.
    styled: Option<(usize, StyledDocument)>,
}

impl FileView {
    fn new(document: Document) -> Self {
        Self {
            outline: document.toc(),
            document,
            outline_cursor: 0,
            styled: None,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The headings, in document order.
    pub fn outline(&self) -> &[TocEntry] {
        &self.outline
    }

    /// The index of the outline entry the cursor is on (0 for an empty outline).
    pub fn outline_cursor(&self) -> usize {
        self.outline_cursor
    }

    /// The styled lines for the document panel's current width; empty until the file
    /// was shown.
    pub fn lines(&self) -> &[Line] {
        self.styled
            .as_ref()
            .map_or(&[], |(_, styled)| styled.lines.as_slice())
    }

    /// Lays the document out for `width` columns, unless it already is.
    fn lay_out(&mut self, renderer: &AnsiRenderer, width: usize) {
        if self.styled.as_ref().is_some_and(|(w, _)| *w == width) {
            return;
        }
        let styled = renderer.clone().with_width(width).render(&self.document);
        self.styled = Some((width, styled));
    }
}

/// The state of `mrq show`; see the [crate docs](crate).
#[derive(Debug)]
pub struct App {
    files: Vec<FileView>,
    /// The index of the shown file.
    current: usize,
    /// The index of the file the file list's cursor is on.
    file_cursor: usize,
    focus: Panel,
    outline_visible: bool,
    overlay: Option<Overlay>,
    /// The terminal size in columns and rows.
    size: (u16, u16),
    renderer: AnsiRenderer,
    quit: bool,
}

impl App {
    /// An app showing the first of `documents`, laid out by `renderer` (whose width is
    /// replaced by the document panel's), for an 80×24 terminal until [`resize`d].
    ///
    /// [`resize`d]: Self::resize
    ///
    /// # Panics
    ///
    /// If `documents` is empty.
    pub fn new(documents: Vec<Document>, renderer: AnsiRenderer) -> Self {
        assert!(!documents.is_empty(), "an app needs at least one document");
        let mut app = Self {
            files: documents.into_iter().map(FileView::new).collect(),
            current: 0,
            file_cursor: 0,
            focus: Panel::Document,
            outline_visible: true,
            overlay: None,
            size: (80, 24),
            renderer,
            quit: false,
        };
        app.relayout();
        app
    }

    /// Reacts to `event`.
    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if let Some(action) = event::action(&key, self) {
                    self.apply(action);
                }
            }
            Event::Resize(width, height) => self.resize(width, height),
        }
    }

    /// Does what `action` stands for.
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::NextPanel => self.cycle_focus(1),
            Action::PreviousPanel => self.cycle_focus(-1),
            Action::ToggleOutline => {
                self.outline_visible = !self.outline_visible;
                self.relayout();
            }
            Action::ToggleHelp => {
                self.overlay = match self.overlay {
                    Some(Overlay::Help) => None,
                    None => Some(Overlay::Help),
                };
            }
            Action::CloseOverlay => self.overlay = None,
            Action::Up => self.move_cursor(false),
            Action::Down => self.move_cursor(true),
            Action::Open => {
                if self.focus == Panel::Files {
                    self.current = self.file_cursor;
                    self.relayout();
                }
            }
        }
    }

    /// Adapts to a terminal of `width` columns and `height` rows; does nothing if that
    /// is the current size.
    pub fn resize(&mut self, width: u16, height: u16) {
        if self.size != (width, height) {
            self.size = (width, height);
            self.relayout();
        }
    }

    /// The terminal size in columns and rows.
    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn files(&self) -> &[FileView] {
        &self.files
    }

    /// The index of the shown file.
    pub fn current(&self) -> usize {
        self.current
    }

    /// The shown file.
    pub fn file(&self) -> &FileView {
        &self.files[self.current]
    }

    /// The index of the file the file list's cursor is on.
    pub fn file_cursor(&self) -> usize {
        self.file_cursor
    }

    pub fn focus(&self) -> Panel {
        self.focus
    }

    /// The panels on screen, in screen order: the file list (several files only) and the
    /// outline (unless toggled off) when the terminal is wide enough, and the document.
    pub fn panels(&self) -> Vec<Panel> {
        let layout = self.layout();
        [
            layout.files.map(|_| Panel::Files),
            layout.outline.map(|_| Panel::Outline),
            Some(Panel::Document),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// Whether the outline is toggled on; it may still be hidden in a narrow terminal.
    pub fn outline_visible(&self) -> bool {
        self.outline_visible
    }

    pub fn overlay(&self) -> Option<Overlay> {
        self.overlay
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// The width of the document panel's content in columns.
    pub fn document_width(&self) -> u16 {
        ui::inner(self.layout().document).width
    }

    fn layout(&self) -> ui::AppLayout {
        let (width, height) = self.size;
        ui::layout(self, Rect::new(0, 0, width, height))
    }

    /// Moves the focus `step` panels on among the visible ones.
    fn cycle_focus(&mut self, step: isize) {
        let panels = self.panels();
        let at = panels.iter().position(|&p| p == self.focus).unwrap_or(0);
        let next = (at.cast_signed() + step).rem_euclid(panels.len().cast_signed());
        self.focus = panels[next.cast_unsigned()];
    }

    /// Moves the cursor of the focused list one entry down (`down`) or up, stopping at
    /// the ends.
    fn move_cursor(&mut self, down: bool) {
        let (cursor, len) = match self.focus {
            Panel::Files => (&mut self.file_cursor, self.files.len()),
            Panel::Outline => {
                let file = &mut self.files[self.current];
                (&mut file.outline_cursor, file.outline.len())
            }
            Panel::Document => return,
        };
        *cursor = if down {
            (*cursor + 1).min(len.saturating_sub(1))
        } else {
            cursor.saturating_sub(1)
        };
    }

    /// After a change to the size or the panels: moves the focus off a panel that is no
    /// longer shown, and lays the shown file out for the document panel.
    fn relayout(&mut self) {
        if !self.panels().contains(&self.focus) {
            self.focus = Panel::Document;
        }
        let width = usize::from(self.document_width());
        self.files[self.current].lay_out(&self.renderer, width);
    }
}
