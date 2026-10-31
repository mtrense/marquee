//! The app state: the open files, which one is shown, the focused panel, the outline
//! toggle, the overlay, the terminal size and colour level, and the navigation state
//! (history, chosen link, status message) with link following in `navigate`.

use std::path::PathBuf;

use mrq_core::Document;
use mrq_core::ast::NodeId;
use mrq_core::document::{DocumentOptions, Workspace};
use mrq_core::toc::TocEntry;
use mrq_core::validate::Vault;
use mrq_render::ansi::{AnsiRenderer, ColorLevel, Line, StyledDocument};
use ratatui::crossterm::event::KeyEventKind;
use ratatui::layout::Rect;

use crate::event::{self, Action, Event};
use crate::open::{Opener, SystemOpener};
use crate::ui;
use crate::view::DocumentView;

mod navigate;

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

/// One open file: the document, its outline, and its layout and view (selection and
/// scroll position) for the document panel.
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
}

impl FileView {
    fn new(document: Document) -> Self {
        let outline = document.toc();
        // `outline` indents each heading under the closest lower-level one, one line each.
        let outline_text = mrq_core::toc::outline(&outline)
            .lines()
            .map(str::to_owned)
            .collect();
        Self {
            outline,
            outline_text,
            document,
            outline_cursor: 0,
            styled: None,
            view: DocumentView::default(),
        }
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
    /// Where each jump came from, the latest last.
    history: Vec<Location>,
    /// The link of the selected block chosen with `←`/`→`: file, block node and index.
    link: Option<(usize, NodeId, usize)>,
    /// A message for the status line, until the next key.
    status: Option<String>,
}

impl App {
    /// An app showing the first of `documents`, laid out by `renderer` (whose width is
    /// replaced by the document panel's), for an 80×24 terminal until [`resize`d]. Links
    /// resolve from the current directory ([`with_root`](Self::with_root)), and the
    /// [`SystemOpener`] opens what the viewer does not show ([`with_opener`](Self::with_opener)).
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
            history: Vec::new(),
            link: None,
            status: None,
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

    /// How the document's colours are drawn; see [`with_color_level`](Self::with_color_level).
    pub fn color_level(&self) -> ColorLevel {
        self.color_level
    }

    /// Reacts to `event`.
    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Release {
                    self.status = None;
                }
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
    /// longer shown, and lays the shown file out for the document panel.
    fn relayout(&mut self) {
        if !self.panels().contains(&self.focus) {
            self.focus = Panel::Document;
        }
        let width = usize::from(self.document_width());
        let height = usize::from(self.document_height());
        self.files[self.current].lay_out(&self.renderer, width, height);
    }
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
