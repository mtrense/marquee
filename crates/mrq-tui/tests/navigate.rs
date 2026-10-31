//! Navigation state: following links in the selected block (anchors, other files open or
//! on disk, wikilinks, the OS opener for the rest), choosing among several links, going
//! back through the history, jumping from the outline, and status messages for targets
//! that are not there.

mod common;

use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

use common::{GUIDE, doc, draw, key, press, renderer, selected};
use mrq_core::document::DocumentOptions;
use mrq_tui::{App, Event, Location, Opener, Panel};
use ratatui::crossterm::event::KeyCode;

const LINKS: &str = "\
# Links

See [usage](#usage), [the guide](guide.md#install) and [web](https://example.com/a?b=1).

[gone](#gone) [missing](missing.md) [mail](mailto:me@example.com)

No links here.

## Usage

Text.
";

/// An opener that records what it was asked to open.
#[derive(Debug, Clone, Default)]
struct Recorder(Rc<RefCell<Vec<OsString>>>);

impl Recorder {
    fn opened(&self) -> Vec<OsString> {
        self.0.borrow().clone()
    }
}

impl Opener for Recorder {
    fn open(&mut self, target: &OsStr) -> io::Result<()> {
        self.0.borrow_mut().push(target.to_owned());
        Ok(())
    }
}

/// An opener that always fails.
#[derive(Debug)]
struct Failing;

impl Opener for Failing {
    fn open(&mut self, _target: &OsStr) -> io::Result<()> {
        Err(io::Error::other("no opener"))
    }
}

/// `app` sized 100×30.
fn sized(mut app: App) -> App {
    app.handle(Event::Resize(100, 30));
    app
}

/// An app over `links.md` and `guide.md` with `opener`, sized 100×30.
fn links(opener: impl Opener + 'static) -> App {
    let docs = vec![doc("links.md", LINKS), doc("guide.md", GUIDE)];
    sized(App::new(docs, renderer()).with_opener(opener))
}

/// The status message, if any.
fn status(app: &App) -> Option<&str> {
    app.status()
}

#[test]
fn enter_follows_an_anchor_and_backspace_comes_back() {
    let mut app = links(Recorder::default());
    press(&mut app, [key(KeyCode::Down)]);
    assert!(selected(&app).starts_with("See "));
    assert_eq!(app.link_index(), Some(0));
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(selected(&app), "## Usage");
    assert_eq!(app.file().outline_cursor(), 1);
    assert_eq!(status(&app), None);
    let paragraph = LINKS.find("See ").unwrap();
    assert_eq!(
        app.history(),
        [Location {
            file: 0,
            offset: paragraph
        }]
    );

    press(&mut app, [key(KeyCode::Backspace)]);
    assert!(selected(&app).starts_with("See "));
    assert_eq!(app.history(), []);
    press(&mut app, [key(KeyCode::Backspace)]);
    assert_eq!(status(&app), Some("nothing to go back to"));
}

#[test]
fn left_and_right_choose_among_the_links_of_the_block() {
    let mut app = links(Recorder::default());
    // A block without links has no link to choose.
    assert_eq!(app.link_index(), None);
    press(&mut app, [key(KeyCode::Right)]);
    assert_eq!(status(&app), Some("no link in the selected block"));

    press(&mut app, [key(KeyCode::Down), key(KeyCode::Right)]);
    assert_eq!(app.link_index(), Some(1));
    assert_eq!(status(&app), Some("link 2/3: guide.md#install"));
    press(&mut app, [key(KeyCode::Right), key(KeyCode::Right)]);
    assert_eq!(app.link_index(), Some(0), "wraps around");
    press(&mut app, [key(KeyCode::Left)]);
    assert_eq!(app.link_index(), Some(2));
    assert_eq!(status(&app), Some("link 3/3: https://example.com/a?b=1"));
    // Moving the selection starts again at the first link.
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Up)]);
    assert_eq!(app.link_index(), Some(0));
}

#[test]
fn links_to_other_open_files_switch_to_them_at_the_anchor() {
    let mut app = links(Recorder::default());
    press(
        &mut app,
        [key(KeyCode::Down), key(KeyCode::Right), key(KeyCode::Enter)],
    );
    assert_eq!(app.current(), 1);
    assert_eq!(app.file_cursor(), 1);
    assert_eq!(selected(&app), "## Install");
    assert_eq!(app.files().len(), 2, "the open file is reused");

    press(&mut app, [key(KeyCode::Backspace)]);
    assert_eq!(app.current(), 0);
    assert!(selected(&app).starts_with("See "));
    assert_eq!(app.link_index(), Some(1), "the chosen link is kept");
}

#[test]
fn external_links_go_to_the_opener() {
    let recorder = Recorder::default();
    let mut app = links(recorder.clone());
    press(
        &mut app,
        [key(KeyCode::Down), key(KeyCode::Left), key(KeyCode::Enter)],
    );
    assert_eq!(recorder.opened(), ["https://example.com/a?b=1"]);
    assert_eq!(status(&app), Some("opened https://example.com/a?b=1"));
    assert_eq!(app.history(), []);
    assert!(selected(&app).starts_with("See "));
    // mailto: goes as written.
    press(
        &mut app,
        [key(KeyCode::Down), key(KeyCode::Left), key(KeyCode::Enter)],
    );
    assert_eq!(recorder.opened()[1], "mailto:me@example.com");
}

#[test]
fn a_failing_opener_is_a_status_message() {
    let mut app = links(Failing);
    press(
        &mut app,
        [key(KeyCode::Down), key(KeyCode::Left), key(KeyCode::Enter)],
    );
    assert_eq!(
        status(&app),
        Some("cannot open https://example.com/a?b=1: no opener")
    );
}

#[test]
fn broken_targets_are_status_messages() {
    let mut app = links(Recorder::default());
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    let before = selected(&app);
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(status(&app), Some("no anchor `#gone` in this document"));
    press(&mut app, [key(KeyCode::Right), key(KeyCode::Enter)]);
    assert_eq!(status(&app), Some("no file `missing.md`"));
    assert_eq!(selected(&app), before);
    assert_eq!(app.current(), 0);
    assert_eq!(app.history(), []);
    // Enter on a block without links says so.
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Enter)]);
    assert_eq!(status(&app), Some("no link in the selected block"));
}

#[test]
fn the_status_message_is_drawn_until_the_next_key() {
    let mut app = links(Recorder::default());
    press(&mut app, [key(KeyCode::Backspace)]);
    assert!(draw(&app).contains("nothing to go back to"));
    press(&mut app, [key(KeyCode::Down)]);
    assert_eq!(status(&app), None);
    assert!(!draw(&app).contains("nothing to go back to"));
}

/// Writes `files` (path, text) under a new temporary directory.
fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

/// An app over the file `name` of `root`, with links resolving under `root`.
fn on_disk(root: &Path, name: &str, opener: impl Opener + 'static) -> App {
    let options = DocumentOptions::new();
    let document = options.load(root.join(name)).unwrap();
    sized(
        App::new(vec![document], renderer())
            .with_root(root, options)
            .with_opener(opener),
    )
}

#[test]
fn markdown_files_on_disk_open_as_new_files() {
    let dir = tree(&[
        (
            "a.md",
            "# A\n\n[b](sub/b.md#part) [top](sub/b.md) [bad](sub/b.md#nope)\n",
        ),
        ("sub/b.md", "# B\n\nIntro.\n\n## Part\n\nText.\n"),
    ]);
    let mut app = on_disk(dir.path(), "a.md", Recorder::default());
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Enter)]);
    assert_eq!(app.files().len(), 2);
    assert_eq!(app.current(), 1);
    assert_eq!(
        app.file().document().path(),
        Some(dir.path().join("sub/b.md").as_path())
    );
    assert_eq!(selected(&app), "## Part");
    assert_eq!(
        app.panels(),
        [Panel::Files, Panel::Outline, Panel::Document]
    );

    // Back, then to the same file again: it is not loaded twice, and without an anchor
    // it opens at the top.
    press(
        &mut app,
        [
            key(KeyCode::Backspace),
            key(KeyCode::Right),
            key(KeyCode::Enter),
        ],
    );
    assert_eq!(app.files().len(), 2);
    assert_eq!(app.current(), 1);
    assert_eq!(selected(&app), "# B");
    assert_eq!(app.history().len(), 1);

    // A missing anchor in another file opens it at the top and says so.
    press(
        &mut app,
        [
            key(KeyCode::Backspace),
            key(KeyCode::Right),
            key(KeyCode::Enter),
        ],
    );
    assert_eq!(app.current(), 1);
    assert_eq!(selected(&app), "# B");
    assert_eq!(status(&app), Some("no anchor `#nope` in `sub/b.md`"));
}

#[test]
fn other_local_files_go_to_the_opener() {
    let dir = tree(&[
        ("a.md", "# A\n\n[logo](img/logo.png) [gone](img/gone.png)\n"),
        ("img/logo.png", "png"),
    ]);
    let recorder = Recorder::default();
    let mut app = on_disk(dir.path(), "a.md", recorder.clone());
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Enter)]);
    let logo = dir.path().join("img/logo.png");
    assert_eq!(recorder.opened(), [logo.as_os_str()]);
    assert_eq!(app.history(), []);
    press(&mut app, [key(KeyCode::Right), key(KeyCode::Enter)]);
    assert_eq!(status(&app), Some("no file `img/gone.png`"));
    assert_eq!(recorder.opened().len(), 1);
}

#[test]
fn wikilinks_are_found_in_the_vault() {
    let front = "---\nmrq:\n  flavor: obsidian\n---\n";
    let home = format!("{front}# Home\n\n[[Note#Part]] [[Note#^blk]] [[Nowhere]]\n");
    let note = format!("{front}# Note\n\nA block. ^blk\n\n## Part\n\nText.\n");
    let dir = tree(&[("Home.md", &home), ("deep/Note.md", &note)]);
    let mut app = on_disk(dir.path(), "Home.md", Recorder::default());
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Enter)]);
    assert_eq!(
        app.file().document().name().file_name(),
        Some(OsStr::new("Note.md"))
    );
    assert_eq!(selected(&app), "## Part");
    press(
        &mut app,
        [
            key(KeyCode::Backspace),
            key(KeyCode::Right),
            key(KeyCode::Enter),
        ],
    );
    assert!(selected(&app).starts_with("A block."));
    press(
        &mut app,
        [
            key(KeyCode::Backspace),
            key(KeyCode::Right),
            key(KeyCode::Enter),
        ],
    );
    assert_eq!(status(&app), Some("no file named `Nowhere` in the vault"));
}

#[test]
fn enter_on_an_outline_entry_jumps_to_its_heading() {
    let docs = vec![doc("guide.md", GUIDE)];
    let mut app = sized(App::new(docs, renderer()).with_opener(Recorder::default()));
    press(
        &mut app,
        [
            key(KeyCode::Tab),
            key(KeyCode::Down),
            key(KeyCode::Down),
            key(KeyCode::Enter),
        ],
    );
    assert_eq!(app.focus(), Panel::Document);
    assert_eq!(selected(&app), "## Usage");
    assert_eq!(app.file().outline_cursor(), 2);
    assert_eq!(app.history(), [Location { file: 0, offset: 0 }]);
    // Backspace works from any panel and focuses the document.
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Backspace)]);
    assert_eq!(app.focus(), Panel::Document);
    assert_eq!(selected(&app), "# Guide");
    assert_eq!(app.file().outline_cursor(), 0);
}

#[test]
fn opening_from_the_file_list_is_in_the_history() {
    let mut app = common::several(100, 20);
    press(
        &mut app,
        [key(KeyCode::Tab), key(KeyCode::End), key(KeyCode::Enter)],
    );
    assert_eq!(app.current(), 2);
    assert_eq!(app.history(), [Location { file: 0, offset: 0 }]);
    press(&mut app, [key(KeyCode::Backspace)]);
    assert_eq!(app.current(), 0);
    assert_eq!(app.file_cursor(), 0);
    // Opening the shown file again is no jump.
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Enter)]);
    assert_eq!(app.history(), []);
}
