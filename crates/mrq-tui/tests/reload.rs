//! The external editor and live reload: `e` hands the shown file to the injectable
//! [`Editor`] at the selected block's line and reloads it afterwards, and
//! [`Event::Changed`] (from the file watcher) reloads a file that changed on disk,
//! keeping the selection near where it was. Unsaved edits are never overwritten: `e`
//! refuses, a change on disk only warns, and `R` reloads on request (undoably). Deleted
//! files keep their content. The watcher itself is a seam too ([`Watch`]); its notify
//! implementation has a test against the real file system at the end.

mod common;

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use common::{GUIDE, key, key_with, press, renderer, selected};
use mrq_core::Document;
use mrq_core::Slashed as _;
use mrq_core::document::DocumentOptions;
use mrq_core::source::SourceFile;
use mrq_tui::watch::{FileWatcher, Watch};
use mrq_tui::{App, Editor, Event, SystemEditor};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// An editor that records where it was opened and writes `replacement`, if any, to the
/// file, the way a user saving in the editor would.
#[derive(Debug, Clone, Default)]
struct Scripted {
    opened: Rc<RefCell<Vec<(PathBuf, usize)>>>,
    replacement: Option<String>,
    fails: bool,
}

impl Scripted {
    fn writing(text: &str) -> Self {
        Self {
            replacement: Some(text.to_owned()),
            ..Self::default()
        }
    }

    fn opened(&self) -> Vec<(PathBuf, usize)> {
        self.opened.borrow().clone()
    }
}

impl Editor for Scripted {
    fn edit(&mut self, path: &Path, line: usize) -> io::Result<()> {
        self.opened.borrow_mut().push((path.to_owned(), line));
        if let Some(text) = &self.replacement {
            fs::write(path, text)?;
        }
        if self.fails {
            return Err(io::Error::other("the editor exited with exit status: 1"));
        }
        Ok(())
    }
}

/// A watcher that records the paths it is asked to watch.
#[derive(Debug, Clone, Default)]
struct Watched(Rc<RefCell<Vec<PathBuf>>>);

impl Watch for Watched {
    fn watch(&mut self, path: &Path) -> io::Result<()> {
        self.0.borrow_mut().push(path.to_owned());
        Ok(())
    }
}

/// A temporary directory with `guide.md` holding `text`, and its path.
fn on_disk(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guide.md");
    fs::write(&path, text).unwrap();
    (dir, path)
}

/// An app over the file `path`, with `editor`.
fn editing(path: &Path, editor: impl Editor + 'static) -> App {
    let document = Document::load(path).unwrap();
    let mut app = App::new(vec![document], renderer()).with_editor(editor);
    app.handle(Event::Resize(80, 24));
    app
}

fn text(app: &App) -> &str {
    app.file().document().text()
}

/// A key without a meaning, which clears the status line.
fn nothing() -> Event {
    key(KeyCode::Char('x'))
}

/// `R`, as terminals send it.
fn reload() -> Event {
    key_with(KeyCode::Char('R'), KeyModifiers::SHIFT)
}

fn changed(path: &Path) -> Event {
    Event::Changed(path.to_owned())
}

/// `GUIDE` with a paragraph inserted before `## Install`.
fn guide_with_more() -> String {
    GUIDE.replace("## Install", "More text.\n\n## Install")
}

#[test]
fn e_opens_the_editor_at_the_selected_line_and_reloads_the_file() {
    let (_dir, path) = on_disk(GUIDE);
    let editor = Scripted::writing(&guide_with_more());
    let mut app = editing(&path, editor.clone());
    // `## Install`, on line 5.
    press(&mut app, vec![key(KeyCode::Down); 2]);
    assert!(!app.take_repaint());
    press(&mut app, [key(KeyCode::Char('e'))]);
    assert_eq!(editor.opened(), [(path.clone(), 5)]);
    assert_eq!(text(&app), guide_with_more());
    assert_eq!(selected(&app), "## Install");
    assert_eq!(app.status(), Some(&*format!("reloaded {}", path.slashed())));
    assert!(!app.file().is_dirty());
    // The editor had the terminal: the loop clears the screen before the next draw.
    assert!(app.take_repaint());
    assert!(!app.take_repaint());
}

#[test]
fn an_editor_that_changes_nothing_says_so() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    press(&mut app, [key(KeyCode::Char('e'))]);
    assert_eq!(text(&app), GUIDE);
    assert_eq!(
        app.status(),
        Some(&*format!("{} is unchanged", path.slashed()))
    );
}

#[test]
fn an_editor_that_fails_says_why_and_the_file_is_still_reloaded() {
    let (_dir, path) = on_disk(GUIDE);
    let editor = Scripted {
        fails: true,
        ..Scripted::writing("# Changed\n")
    };
    let mut app = editing(&path, editor);
    press(&mut app, [key(KeyCode::Char('e'))]);
    assert_eq!(text(&app), "# Changed\n");
    assert_eq!(
        app.status(),
        Some("editor: the editor exited with exit status: 1")
    );
    assert!(app.take_repaint());
}

#[test]
fn e_refuses_stdin_and_unsaved_edits() {
    let editor = Scripted::default();
    let stdin = DocumentOptions::new()
        .parse_source(SourceFile::new(None, GUIDE.to_owned()))
        .unwrap();
    let mut app = App::new(vec![stdin], renderer()).with_editor(editor.clone());
    press(&mut app, [key(KeyCode::Char('e'))]);
    assert_eq!(app.status(), Some("cannot edit <stdin>: it has no file"));

    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, editor.clone());
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    press(&mut app, [key(KeyCode::Char('d')), key(KeyCode::Char('e'))]);
    assert_eq!(
        app.status(),
        Some(&*format!(
            "unsaved edits in {}: w writes them, R drops them, then e edits the file",
            path.slashed()
        ))
    );
    assert_eq!(editor.opened(), []);
    assert!(app.file().is_dirty());
    assert!(!app.take_repaint());
}

#[test]
fn a_change_on_disk_reloads_the_file_keeping_the_selection() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    // `## Usage`.
    press(&mut app, vec![key(KeyCode::Down); 4]);
    assert_eq!(selected(&app), "## Usage");
    let row = app.file().view().selected().unwrap().lines.start - app.file().view().top();

    // Text appended after the selection keeps it on the same block.
    let appended = format!("{GUIDE}\n## Appendix\n\nMore.\n");
    fs::write(&path, &appended).unwrap();
    press(&mut app, [changed(&path)]);
    assert_eq!(text(&app), appended);
    assert_eq!(selected(&app), "## Usage");
    let view = app.file().view();
    assert_eq!(view.selected().unwrap().lines.start - view.top(), row);
    assert_eq!(app.status(), Some(&*format!("reloaded {}", path.slashed())));
    assert_eq!(app.file().outline().len(), 5);
    assert!(!app.file().is_dirty());
    // There is nothing to undo: the reload is not an edit.
    press(&mut app, [key(KeyCode::Char('u'))]);
    assert_eq!(app.status(), Some("nothing to undo"));
    // The terminal was not handed over.
    assert!(!app.take_repaint());
}

#[test]
fn changes_that_leave_the_text_as_it_is_are_ignored() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    // A touch, or the watcher reporting our own `w`.
    press(&mut app, [changed(&path)]);
    assert_eq!(app.status(), None);

    press(&mut app, vec![key(KeyCode::Down); 2]);
    press(&mut app, [key(KeyCode::Char('d')), key(KeyCode::Char('w'))]);
    let saved = text(&app).to_owned();
    press(&mut app, [changed(&path)]);
    assert_eq!(app.status(), Some(&*format!("wrote {}", path.slashed())));
    assert_eq!(text(&app), saved);
    // Files that are not open are no concern either.
    press(&mut app, [changed(Path::new("elsewhere.md"))]);
    assert_eq!(app.status(), Some(&*format!("wrote {}", path.slashed())));
}

#[test]
fn a_change_on_disk_keeps_unsaved_edits_and_r_reloads_undoably() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    press(&mut app, vec![key(KeyCode::Down); 2]);
    press(&mut app, [key(KeyCode::Char('d'))]);
    let edited = text(&app).to_owned();

    fs::write(&path, guide_with_more()).unwrap();
    press(&mut app, [changed(&path)]);
    assert_eq!(text(&app), edited);
    assert!(app.file().is_dirty());
    assert!(app.file().changed_on_disk());
    assert_eq!(
        app.status(),
        Some(&*format!(
            "{} changed on disk; your edits are kept: w writes them over it, R reloads it",
            path.slashed()
        ))
    );
    // The same change reported again is not news.
    press(&mut app, [nothing(), changed(&path)]);
    assert_eq!(app.status(), None);

    press(&mut app, [reload()]);
    assert_eq!(text(&app), guide_with_more());
    assert!(!app.file().is_dirty());
    assert!(!app.file().changed_on_disk());
    assert_eq!(
        app.status(),
        Some(&*format!(
            "reloaded {} (u brings your edits back)",
            path.slashed()
        ))
    );
    // Undoing the reload gives the edits back, unsaved.
    press(&mut app, [key(KeyCode::Char('u'))]);
    assert_eq!(text(&app), edited);
    assert!(app.file().is_dirty());
    // Redoing it is the text on disk again, as saved.
    press(
        &mut app,
        [key_with(KeyCode::Char('r'), KeyModifiers::CONTROL)],
    );
    assert_eq!(text(&app), guide_with_more());
    assert!(!app.file().is_dirty());
}

#[test]
fn w_after_a_change_on_disk_keeps_the_edits() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    press(&mut app, vec![key(KeyCode::Down); 2]);
    press(&mut app, [key(KeyCode::Char('d'))]);
    fs::write(&path, guide_with_more()).unwrap();
    press(&mut app, [changed(&path), key(KeyCode::Char('w'))]);
    assert_eq!(fs::read_to_string(&path).unwrap(), text(&app));
    assert!(!app.file().changed_on_disk());
    // The watcher then reports our own write, which is ignored.
    press(&mut app, [nothing(), changed(&path)]);
    assert_eq!(app.status(), None);
    assert!(!app.file().is_dirty());
}

#[test]
fn r_reloads_a_clean_file_and_refuses_stdin() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    press(&mut app, [reload()]);
    assert_eq!(
        app.status(),
        Some(&*format!("{} is unchanged", path.slashed()))
    );
    fs::write(&path, "# New\n").unwrap();
    press(&mut app, [reload()]);
    assert_eq!(text(&app), "# New\n");
    assert_eq!(app.status(), Some(&*format!("reloaded {}", path.slashed())));

    let stdin = DocumentOptions::new()
        .parse_source(SourceFile::new(None, GUIDE.to_owned()))
        .unwrap();
    let mut app = common::app(vec![stdin], 80, 24);
    press(&mut app, [reload()]);
    assert_eq!(app.status(), Some("cannot reload <stdin>: it has no file"));
}

#[test]
fn a_deleted_file_keeps_its_content_and_w_writes_it_back() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    fs::remove_file(&path).unwrap();
    press(&mut app, [changed(&path)]);
    assert_eq!(text(&app), GUIDE);
    assert!(app.file().is_deleted());
    assert_eq!(
        app.status(),
        Some(&*format!(
            "{} was deleted on disk; its content is kept (w writes it back)",
            path.slashed()
        ))
    );
    press(&mut app, [key(KeyCode::Char('w'))]);
    assert_eq!(fs::read_to_string(&path).unwrap(), GUIDE);
    assert!(!app.file().is_deleted());

    // Recreated with other text, it reloads.
    fs::remove_file(&path).unwrap();
    press(&mut app, [changed(&path)]);
    fs::write(&path, "# Back\n").unwrap();
    press(&mut app, [changed(&path)]);
    assert_eq!(text(&app), "# Back\n");
    assert!(!app.file().is_deleted());
}

#[test]
fn a_file_that_is_not_shown_reloads_too() {
    let (dir, path) = on_disk(GUIDE);
    let other = dir.path().join("other.md");
    fs::write(&other, "# Other\n\nText.\n").unwrap();
    let docs = vec![
        Document::load(&path).unwrap(),
        Document::load(&other).unwrap(),
    ];
    let mut app = common::app(docs, 100, 30);
    fs::write(&other, "# Other\n\nNew text.\n").unwrap();
    press(&mut app, [changed(&other)]);
    assert_eq!(app.current(), 0);
    assert_eq!(app.files()[1].document().text(), "# Other\n\nNew text.\n");
    assert_eq!(
        app.status(),
        Some(&*format!("reloaded {}", other.slashed()))
    );
    // Shown, it is laid out with the new text.
    press(
        &mut app,
        [key(KeyCode::Tab), key(KeyCode::Down), key(KeyCode::Enter)],
    );
    assert_eq!(app.current(), 1);
    assert!(common::draw(&app).contains("New text."));
}

#[test]
fn the_watcher_is_given_every_open_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.md");
    let b = dir.path().join("b.md");
    fs::write(&a, "# A\n\n[b](b.md)\n").unwrap();
    fs::write(&b, "# B\n").unwrap();
    let options = DocumentOptions::new();
    let stdin = DocumentOptions::new()
        .parse_source(SourceFile::new(None, "# Stdin\n".to_owned()))
        .unwrap();
    let documents = vec![options.load(&a).unwrap(), stdin];
    let mut app = App::new(documents, renderer()).with_root(dir.path(), options);
    let watched = Watched::default();
    app.set_watcher(watched.clone());
    // Stdin has no file to watch.
    assert_eq!(*watched.0.borrow(), std::slice::from_ref(&a));
    // A file opened by a link is watched from then on.
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Enter)]);
    assert_eq!(app.files().len(), 3);
    assert_eq!(*watched.0.borrow(), [a, b]);
}

#[test]
fn a_change_event_runs_through_the_event_loop() {
    let (_dir, path) = on_disk(GUIDE);
    let mut app = editing(&path, Scripted::default());
    fs::write(&path, "# Looped\n").unwrap();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    mrq_tui::run_on(
        &mut terminal,
        &mut app,
        [Ok::<_, std::convert::Infallible>(changed(&path))],
    )
    .unwrap();
    assert!(terminal.backend().to_string().contains("Looped"));
}

#[test]
fn the_system_editor_runs_visual_or_editor_with_the_line() {
    let path = Path::new("docs/guide.md");
    let command = SystemEditor::new("code --wait").command(path, 12);
    assert_eq!(command.get_program(), "code");
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, ["--wait", "+12", "docs/guide.md"]);

    // Without a command, `vi`.
    let command = SystemEditor::from_vars(None, Some("  ")).command(path, 1);
    assert_eq!(command.get_program(), "vi");
    let command = SystemEditor::from_vars(Some("nvim"), Some("nano")).command(path, 1);
    assert_eq!(command.get_program(), "nvim");
    let command = SystemEditor::from_vars(Some(""), Some("nano")).command(path, 1);
    assert_eq!(command.get_program(), "nano");
}

/// Waits up to 10 seconds for `watcher` to report `path`.
fn reported(changes: &mut mrq_tui::watch::Changes, path: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if changes.next(Instant::now()).as_deref() == Some(path) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// The notify watcher on the real file system (`FSEvents`, `inotify`, …): an atomic save
/// (a temporary file renamed over the watched one) is reported under the path the app
/// knows, debounced into one change, and files next to it are not reported.
///
/// Sandboxes on macOS (such as Seatbelt) may not let `FSEvents` start a stream; the test
/// then says so on stderr and checks nothing more.
#[test]
fn the_file_watcher_reports_atomic_saves_of_watched_files() {
    let (dir, path) = on_disk(GUIDE);
    let other = dir.path().join("other.md");
    let (mut watcher, mut changes) = FileWatcher::new().unwrap();
    if let Err(err) = watcher.watch(&path) {
        let unavailable = cfg!(target_os = "macos") && err.to_string().contains("FSEvent");
        assert!(unavailable, "cannot watch: {err}");
        eprintln!("skipped: FSEvents is unavailable here ({err})");
        return;
    }
    // FSEvents may report events from just before the watch started.
    std::thread::sleep(Duration::from_millis(200));
    while changes
        .next(Instant::now() + Duration::from_secs(1))
        .is_some()
    {}

    fs::write(&other, "# Other\n").unwrap();
    mrq_core::source::write_atomic(&path, "# Saved\n").unwrap();
    assert!(reported(&mut changes, &path), "no change reported");
    std::thread::sleep(Duration::from_millis(300));
    // One change, however many events the save made; nothing for `other.md`.
    assert_eq!(changes.next(Instant::now() + Duration::from_secs(1)), None);
}
