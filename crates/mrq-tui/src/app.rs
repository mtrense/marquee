//! The app state: the open files, which one is shown, the focused panel, the outline
//! toggle, the overlay, the terminal size and colour level, and the navigation state
//! (history, chosen link, status message) with link following in `navigate`, the
//! prompt with search and query in `search`, structural editing, saving and the
//! question on quit in `editing`, and the external editor and live reload in `reload`.

use std::path::PathBuf;

use mrq_core::Document;
use mrq_core::ast::NodeId;
use mrq_core::document::{DocumentOptions, Workspace};
use mrq_core::toc::TocEntry;
use mrq_core::validate::Vault;
use mrq_render::ansi::{AnsiRenderer, ColorLevel, Line, StyledDocument};
use ratatui::crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::edit::{Structural, UndoStack};
use crate::editor::{Editor, SystemEditor};
use crate::event::{self, Action, Event};
use crate::find::Find;
use crate::open::{Opener, SystemOpener};
use crate::prompt::Prompt;
use crate::ui;
use crate::view::DocumentView;
use crate::watch::Watch;

mod editing;
mod navigate;
mod reload;
mod search;

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

/// A place to go back to: a file and the selected block in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// The index of the file.
    pub file: usize,
    /// The source offset of the selected block's start (0 without blocks).
    pub offset: usize,
}

/// One open file: the document, its outline, its layout and view (selection and scroll
/// position) for the document panel, its edits, and what is known of it on disk.
#[derive(Debug)]
pub struct FileView {
    document: Document,
    outline: Vec<TocEntry>,
    /// The outline as drawn: one indented line per entry.
    outline_text: Vec<String>,
    outline_cursor: usize,
    /// The document laid out, with the width it was laid out for.
    styled: Option<(usize, StyledDocument)>,
    view: DocumentView,
    undo: UndoStack,
    /// The text last read from or written to the file, to tell news from our own saves.
    disk: String,
    /// The file was deleted on disk.
    deleted: bool,
    /// The file changed on disk while it had unsaved edits, which were kept.
    changed_on_disk: bool,
}

impl FileView {
    fn new(document: Document) -> Self {
        let (outline, outline_text) = outline(&document);
        Self {
            disk: document.text().to_owned(),
            outline,
            outline_text,
            outline_cursor: 0,
            styled: None,
            view: DocumentView::default(),
            undo: UndoStack::default(),
            deleted: false,
            changed_on_disk: false,
            document,
        }
    }

    /// Replaces the document with `document`, an edited version of it, with its outline;
    /// the next [`lay_out`](Self::lay_out) lays it out.
    fn set_document(&mut self, document: Document) {
        (self.outline, self.outline_text) = outline(&document);
        self.document = document;
        self.styled = None;
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The headings, in document order.
    pub fn outline(&self) -> &[TocEntry] {
        &self.outline
    }

    /// The outline as drawn, one line per entry.
    pub fn outline_text(&self) -> &[String] {
        &self.outline_text
    }

    /// The index of the outline entry the cursor is on (0 for an empty outline). It
    /// follows the selection in the document, and moves on its own in the outline.
    pub fn outline_cursor(&self) -> usize {
        self.outline_cursor
    }

    /// The document panel's blocks, selection and scroll position.
    pub fn view(&self) -> &DocumentView {
        &self.view
    }

    /// Whether the document has edits that are not saved.
    pub fn is_dirty(&self) -> bool {
        self.undo.is_dirty()
    }

    /// Whether the file was deleted on disk (its content is kept, and `w` writes it
    /// back).
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// Whether the file changed on disk while it had unsaved edits, which were kept: `w`
    /// writes them over it, `R` reloads it.
    pub fn changed_on_disk(&self) -> bool {
        self.changed_on_disk
    }

    /// The styled lines for the document panel's current width; empty until the file
    /// was shown.
    pub fn lines(&self) -> &[Line] {
        self.styled
            .as_ref()
            .map_or(&[], |(_, styled)| styled.lines.as_slice())
    }

    /// Lays the document out for `width` columns, unless it already is, keeping the
    /// selection, and gives the view `height` rows.
    fn lay_out(&mut self, renderer: &AnsiRenderer, width: usize, height: usize) {
        if self.styled.as_ref().is_none_or(|(w, _)| *w != width) {
            let styled = renderer.clone().with_width(width).render(&self.document);
            self.view = DocumentView::new(&styled.lines, self.document.ast(), &self.outline)
                .following(&self.view);
            self.styled = Some((width, styled));
        }
        self.view.set_height(height);
        self.sync_outline();
    }

    /// Moves the outline cursor to the section of the selection.
    fn sync_outline(&mut self) {
        self.outline_cursor = self.view.section().unwrap_or(0);
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
    color_level: ColorLevel,
    quit: bool,
    /// Where links resolve and how linked files are loaded; its documents are unused.
    workspace: Workspace,
    /// The files wikilinks name, listed on the first wikilink followed.
    vault: Option<Vault>,
    opener: Box<dyn Opener>,
    editor: Box<dyn Editor>,
    /// Told about every open file, for live reload.
    watcher: Option<Box<dyn Watch>>,
    /// The terminal was handed to the editor: clear it before the next draw.
    repaint: bool,
    /// Where each jump came from, the latest last.
    history: Vec<Location>,
    /// The link of the selected block chosen with `←`/`→`: file, block node and index.
    link: Option<(usize, NodeId, usize)>,
    /// A message for the status line, until the next key.
    status: Option<String>,
    /// The open prompt, which takes the keys.
    asking: Option<search::Asking>,
    /// The active search or query.
    find: Option<Find>,
}

impl App {
    /// An app showing the first of `documents`, laid out by `renderer` (whose width is
    /// replaced by the document panel's), for an 80×24 terminal until [`resize`d]. Links
    /// resolve from the current directory ([`with_root`](Self::with_root)), and the
    /// [`SystemOpener`] opens what the viewer does not show ([`with_opener`](Self::with_opener)),
    /// and `e` runs the [`SystemEditor`] ([`with_editor`](Self::with_editor)). Nothing
    /// is watched for changes until [`set_watcher`](Self::set_watcher).
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
            color_level: ColorLevel::TrueColor,
            quit: false,
            workspace: Workspace::new("."),
            vault: None,
            opener: Box::new(SystemOpener),
            editor: Box::new(SystemEditor::default()),
            watcher: None,
            repaint: false,
            history: Vec::new(),
            link: None,
            status: None,
            asking: None,
            find: None,
        };
        app.relayout();
        app
    }

    /// This app drawing the document's colours for a terminal of `level`: as they are
    /// (true colour, the default), the closest of the 256-colour palette, or not at all.
    #[must_use]
    pub fn with_color_level(mut self, level: ColorLevel) -> Self {
        self.color_level = level;
        self
    }

    /// This app resolving links against the workspace root `root` (for paths starting
    /// with `/`, and the vault of wikilinks) and loading linked files with `options`.
    #[must_use]
    pub fn with_root(mut self, root: impl Into<PathBuf>, options: DocumentOptions) -> Self {
        self.workspace = Workspace::with_options(root, options);
        self.vault = None;
        self
    }

    /// This app handing web links, `mailto:` addresses and local files other than
    /// Markdown to `opener`.
    #[must_use]
    pub fn with_opener(mut self, opener: impl Opener + 'static) -> Self {
        self.opener = Box::new(opener);
        self
    }

    /// This app editing files with `editor` on `e`.
    #[must_use]
    pub fn with_editor(mut self, editor: impl Editor + 'static) -> Self {
        self.editor = Box::new(editor);
        self
    }

    /// Watches every open file with `watcher`, and the files opened later. A file that
    /// cannot be watched says so in the status line.
    pub fn set_watcher(&mut self, watcher: impl Watch + 'static) {
        self.watcher = Some(Box::new(watcher));
        for index in 0..self.files.len() {
            self.watch(index);
        }
    }

    /// Whether the terminal was handed to the editor since the last call, so the screen
    /// must be cleared before the next draw.
    pub fn take_repaint(&mut self) -> bool {
        std::mem::take(&mut self.repaint)
    }

    /// How the document's colours are drawn; see [`with_color_level`](Self::with_color_level).
    pub fn color_level(&self) -> ColorLevel {
        self.color_level
    }

    /// Reacts to `event`. While a prompt is open, keys other than `Ctrl-c` edit it.
    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Release {
                    self.status = None;
                }
                let quit =
                    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
                if self.asking.is_some() && !quit {
                    return self.prompt_key(&key);
                }
                if let Some(action) = event::action(&key, self) {
                    self.apply(action);
                }
            }
            Event::Resize(width, height) => self.resize(width, height),
            Event::Changed(path) => self.file_changed(&path),
        }
    }

    /// Does what `action` stands for.
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.ask_quit(),
            Action::ForceQuit => self.quit = true,
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
            Action::Up => self.step(Step::Up),
            Action::Down => self.step(Step::Down),
            Action::PageUp => self.step(Step::PageUp),
            Action::PageDown => self.step(Step::PageDown),
            Action::Home => self.step(Step::Home),
            Action::End => self.step(Step::End),
            Action::Open => match self.focus {
                Panel::Files => self.open_file_cursor(),
                Panel::Outline => self.open_outline_cursor(),
                Panel::Document => self.follow(),
            },
            Action::NextLink => self.choose_link(1),
            Action::PreviousLink => self.choose_link(-1),
            Action::Back => self.back(),
            Action::Search => self.ask(search::Purpose::Search),
            Action::Query => self.ask(search::Purpose::Query),
            Action::NextMatch => self.next_match(1),
            Action::PreviousMatch => self.next_match(-1),
            Action::ClearMatches => self.find = None,
            Action::Promote => self.structural(Structural::Promote),
            Action::Demote => self.structural(Structural::Demote),
            Action::MoveSectionUp => self.structural(Structural::MoveUp),
            Action::MoveSectionDown => self.structural(Structural::MoveDown),
            Action::DeleteSection => self.structural(Structural::Delete),
            Action::Undo => self.undo(false),
            Action::Redo => self.undo(true),
            Action::Save => self.save(),
            Action::Edit => self.edit_externally(),
            Action::Reload => self.reload_current(),
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

    /// The message for the status line: what a key did or why it could not, shown until
    /// the next key.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Shows `message` in the status line, until the next key.
    pub(crate) fn say(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }

    /// The open prompt (search, query, or the question on quit), which takes the keys.
    pub fn prompt(&self) -> Option<&Prompt> {
        self.asking.as_ref().map(|asking| &asking.prompt)
    }

    /// The active search or query, with its hits in the shown file.
    pub fn find(&self) -> Option<&Find> {
        self.find.as_ref()
    }

    /// Where `Backspace` goes, the latest jump last.
    pub fn history(&self) -> &[Location] {
        &self.history
    }

    /// The width of the document panel's content in columns.
    pub fn document_width(&self) -> u16 {
        ui::inner(self.layout().document)
            .width
            .saturating_sub(ui::GUTTER)
    }

    /// The rows of the document panel's content.
    pub fn document_height(&self) -> u16 {
        ui::inner(self.layout().document).height
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

    /// Moves the focused panel: the selection in the document (with the outline cursor
    /// following it), or the cursor of a list, stopping at the ends.
    fn step(&mut self, step: Step) {
        let layout = self.layout();
        let (cursor, len, area) = match self.focus {
            Panel::Files => (&mut self.file_cursor, self.files.len(), layout.files),
            Panel::Outline => {
                let file = &mut self.files[self.current];
                (&mut file.outline_cursor, file.outline.len(), layout.outline)
            }
            Panel::Document => {
                self.link = None;
                let file = &mut self.files[self.current];
                let view = &mut file.view;
                match step {
                    Step::Up => view.up(),
                    Step::Down => view.down(),
                    Step::PageUp => view.page_up(),
                    Step::PageDown => view.page_down(),
                    Step::Home => view.home(),
                    Step::End => view.end(),
                }
                file.sync_outline();
                return;
            }
        };
        let page = area.map_or(1, |area| usize::from(ui::inner(area).height).max(1));
        let last = len.saturating_sub(1);
        *cursor = match step {
            Step::Up => cursor.saturating_sub(1),
            Step::Down => (*cursor + 1).min(last),
            Step::PageUp => cursor.saturating_sub(page),
            Step::PageDown => (*cursor + page).min(last),
            Step::Home => 0,
            Step::End => last,
        };
    }

    /// After a change to the size or the panels: moves the focus off a panel that is no
    /// longer shown, lays the shown file out for the document panel, and finds the hits
    /// of the search or query in it.
    fn relayout(&mut self) {
        if !self.panels().contains(&self.focus) {
            self.focus = Panel::Document;
        }
        let width = usize::from(self.document_width());
        let height = usize::from(self.document_height());
        self.files[self.current].lay_out(&self.renderer, width, height);
        if let Some(find) = &mut self.find {
            // A query that fails in this file finds nothing in it.
            let _ = find.refresh(self.current, width, &self.files[self.current]);
        }
    }
}

/// The headings of `document`, and the outline as drawn: each heading indented under the
/// closest lower-level one, one line each.
fn outline(document: &Document) -> (Vec<TocEntry>, Vec<String>) {
    let outline = document.toc();
    let text = mrq_core::toc::outline(&outline)
        .lines()
        .map(str::to_owned)
        .collect();
    (outline, text)
}

/// A move of the focused panel.
#[derive(Debug, Clone, Copy)]
enum Step {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}
