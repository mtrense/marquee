//! Structural editing: promoting, demoting, moving and deleting sections (planned with
//! [`edit::plan`], undone and redone with [`UndoStack`]), the dirty indicator, saving,
//! the question on quit, and what edits do to the selection, search hits and history.
//! The round trip of edit, undo and redo is also a property test.

mod common;

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use common::{GUIDE, NOTES, app, doc, draw, guide, key, key_with, press, selected};
use mrq_core::Document;
use mrq_core::Slashed as _;
use mrq_core::document::DocumentOptions;
use mrq_core::source::SourceFile;
use mrq_tui::edit::{self, Structural, UndoStack};
use mrq_tui::{Action, App, Event, Panel};
use proptest::prelude::*;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

fn shift(code: KeyCode) -> Event {
    key_with(code, KeyModifiers::SHIFT)
}

fn alt(code: KeyCode) -> Event {
    key_with(code, KeyModifiers::ALT)
}

fn ctrl(c: char) -> Event {
    key_with(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// `guide.md` with the selection moved `downs` blocks down.
fn guide_at(downs: usize) -> App {
    let mut app = guide(80, 24);
    press(&mut app, vec![key(KeyCode::Down); downs]);
    app
}

fn text(app: &App) -> &str {
    app.file().document().text()
}

const INTRO: &str = "# Guide\n\nAn introduction that is long enough to wrap when the document panel gets narrow.\n\n";

#[test]
fn shift_right_demotes_the_selected_section_with_its_subsections() {
    // `## Usage` (with `### Options`).
    let mut app = guide_at(4);
    assert_eq!(selected(&app), "## Usage");
    press(&mut app, [shift(KeyCode::Right)]);
    assert_eq!(
        text(&app),
        format!(
            "{INTRO}## Install\n\nRun the installer.\n\n### Usage\n\n#### Options\n\nPass `--help` for the options.\n"
        )
    );
    assert_eq!(selected(&app), "### Usage");
    assert_eq!(app.status(), Some("demoted `Usage`"));
    assert!(app.file().is_dirty());
    // The outline is the edited document's.
    assert_eq!(app.file().outline()[2].level, 3);
    assert_eq!(app.file().outline_cursor(), 2);
}

#[test]
fn shift_left_promotes_the_section_the_selection_is_in() {
    // `Pass --help for the options.`, in `### Options`.
    let mut app = guide_at(6);
    press(&mut app, [shift(KeyCode::Left)]);
    assert!(text(&app).contains("\n## Options\n"));
    assert_eq!(selected(&app), "## Options");
    assert_eq!(app.status(), Some("promoted `Options`"));
    // No link was chosen: Shift-← is not ←.
    assert_eq!(app.link_index(), None);
}

#[test]
fn promoting_level_1_and_demoting_level_6_are_refused() {
    let mut app = guide(80, 24);
    press(&mut app, [shift(KeyCode::Left)]);
    assert_eq!(text(&app), GUIDE);
    assert_eq!(app.status(), Some("cannot promote `Guide`: it is level 1"));
    assert!(!app.file().is_dirty());

    let mut app = app_over("# A\n\n###### Deep\n");
    press(&mut app, [shift(KeyCode::Right)]);
    assert_eq!(text(&app), "# A\n\n###### Deep\n");
    assert_eq!(app.status(), Some("cannot demote `A`: `Deep` is level 6"));
}

#[test]
fn before_the_first_heading_there_is_no_section() {
    let mut app = app_over("Intro.\n\n# A\n");
    press(&mut app, [shift(KeyCode::Right), key(KeyCode::Char('d'))]);
    assert_eq!(text(&app), "Intro.\n\n# A\n");
    assert_eq!(
        app.status(),
        Some("no section here: the selection is before the first heading")
    );
}

#[test]
fn alt_arrows_move_a_section_among_its_siblings() {
    let mut app = guide_at(4);
    press(&mut app, [alt(KeyCode::Up)]);
    let moved = format!(
        "{INTRO}## Usage\n\n### Options\n\nPass `--help` for the options.\n\n## Install\n\nRun the installer.\n"
    );
    assert_eq!(text(&app), moved);
    assert_eq!(selected(&app), "## Usage");
    assert_eq!(app.status(), Some("moved `Usage` up"));

    // `Usage` is the first section in `Guide` now.
    press(&mut app, [alt(KeyCode::Up)]);
    assert_eq!(text(&app), moved);
    assert_eq!(app.status(), Some("`Usage` is the first section already"));

    press(&mut app, [alt(KeyCode::Down)]);
    assert_eq!(text(&app), GUIDE);
    assert_eq!(selected(&app), "## Usage");
    assert_eq!(app.status(), Some("moved `Usage` down"));
    press(&mut app, [alt(KeyCode::Down)]);
    assert_eq!(app.status(), Some("`Usage` is the last section already"));

    // A subsection moves within its parent only.
    let mut app = guide_at(5);
    assert_eq!(selected(&app), "### Options");
    press(&mut app, [alt(KeyCode::Up)]);
    assert_eq!(text(&app), GUIDE);
    assert_eq!(app.status(), Some("`Options` is the first section already"));
}

#[test]
fn d_and_delete_delete_the_section() {
    let mut app = guide_at(2);
    press(&mut app, [key(KeyCode::Char('d'))]);
    assert_eq!(
        text(&app),
        format!("{INTRO}## Usage\n\n### Options\n\nPass `--help` for the options.\n")
    );
    assert_eq!(selected(&app), "## Usage");
    assert_eq!(app.status(), Some("deleted `Install` (u to undo)"));

    // The last section goes with the blank line before it.
    press(&mut app, [key(KeyCode::Delete)]);
    assert_eq!(text(&app), INTRO.trim_end_matches('\n').to_owned() + "\n");
    assert!(selected(&app).starts_with("An introduction"));

    // And the only one leaves the rest.
    press(&mut app, [key(KeyCode::Home), key(KeyCode::Char('d'))]);
    assert_eq!(text(&app), "\n");
}

#[test]
fn undo_and_redo_go_through_the_edits() {
    let mut app = guide_at(4);
    press(&mut app, [alt(KeyCode::Up), shift(KeyCode::Right)]);
    let edited = text(&app).to_owned();
    press(&mut app, [key(KeyCode::Char('u'))]);
    assert_eq!(app.status(), Some("undone: demoted `Usage`"));
    press(&mut app, [key(KeyCode::Char('u'))]);
    assert_eq!(text(&app), GUIDE);
    assert_eq!(selected(&app), "## Usage");
    assert!(!app.file().is_dirty());
    press(&mut app, [key(KeyCode::Char('u'))]);
    assert_eq!(app.status(), Some("nothing to undo"));

    press(&mut app, [ctrl('r'), ctrl('r')]);
    assert_eq!(text(&app), edited);
    assert_eq!(app.status(), Some("redone: demoted `Usage`"));
    assert_eq!(selected(&app), "### Usage");
    press(&mut app, [ctrl('r')]);
    assert_eq!(app.status(), Some("nothing to redo"));

    // A new edit drops what could be redone.
    press(&mut app, [key(KeyCode::Char('u')), key(KeyCode::Char('d'))]);
    press(&mut app, [ctrl('r')]);
    assert_eq!(app.status(), Some("nothing to redo"));
}

#[test]
fn the_outline_cursor_picks_the_section_while_the_outline_is_focused() {
    let mut app = guide(80, 24);
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Down)]);
    assert_eq!(app.focus(), Panel::Outline);
    press(&mut app, [shift(KeyCode::Right)]);
    assert!(text(&app).contains("\n### Install\n"));
    assert_eq!(app.focus(), Panel::Outline);
    assert_eq!(app.file().outline_cursor(), 1);
}

#[test]
fn edits_keep_the_selection_on_its_row() {
    let mut app = app_over(&sections(30));
    press(&mut app, vec![key(KeyCode::PageDown); 2]);
    let before = app.file().view().clone();
    let row = before.selected().unwrap().lines.start - before.top();
    press(&mut app, [shift(KeyCode::Right)]);
    let view = app.file().view();
    assert_eq!(view.selected().unwrap().lines.start - view.top(), row);
}

#[test]
fn the_status_line_and_the_file_list_mark_unsaved_files() {
    let mut app = app(vec![doc("guide.md", GUIDE), doc("notes.md", NOTES)], 80, 24);
    press(&mut app, [shift(KeyCode::Right), key(KeyCode::Down)]);
    let screen = draw(&app);
    assert!(screen.contains("│› guide.md [+]"), "{screen}");
    assert!(screen.contains("\" guide.md [+] (1/2)"), "{screen}");
    press(&mut app, [key(KeyCode::Char('u')), key(KeyCode::Down)]);
    assert!(!draw(&app).contains("[+]"));
}

#[test]
fn w_writes_the_file_and_it_is_clean_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guide.md");
    fs::write(&path, GUIDE).unwrap();
    let mut app = app(vec![Document::load(&path).unwrap()], 80, 24);
    press(&mut app, [key(KeyCode::Char('w'))]);
    assert_eq!(app.status(), Some("no changes to write"));

    press(&mut app, vec![key(KeyCode::Down); 2]);
    press(&mut app, [key(KeyCode::Char('d')), key(KeyCode::Char('w'))]);
    assert_eq!(fs::read_to_string(&path).unwrap(), text(&app));
    assert_eq!(app.status(), Some(&*format!("wrote {}", path.slashed())));
    assert!(!app.file().is_dirty());
    // Undoing past the save makes it dirty again.
    press(&mut app, [key(KeyCode::Char('u'))]);
    assert!(app.file().is_dirty());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_file_that_cannot_be_written_says_so() {
    let mut app = app_over_named(None, GUIDE);
    press(&mut app, [key(KeyCode::Char('d')), key(KeyCode::Char('w'))]);
    assert_eq!(app.status(), Some("cannot write <stdin>: it has no file"));
    assert!(app.file().is_dirty());

    let missing = PathBuf::from("no/such/dir/guide.md");
    let mut app = app_over_named(Some(missing), GUIDE);
    press(&mut app, [key(KeyCode::Char('d')), key(KeyCode::Char('w'))]);
    let status = app.status().unwrap();
    assert!(
        status.starts_with("cannot write no/such/dir/guide.md: "),
        "{status}"
    );
    assert!(app.file().is_dirty());
}

#[test]
fn quitting_with_unsaved_changes_asks() {
    let mut app = guide(80, 24);
    press(&mut app, [key(KeyCode::Char('d')), key(KeyCode::Char('q'))]);
    assert!(!app.should_quit());
    let prompt = app.prompt().unwrap();
    assert_eq!(
        prompt.label(),
        "unsaved changes in guide.md; quit anyway? (y/n) "
    );
    press(&mut app, [key(KeyCode::Char('n'))]);
    assert!(app.prompt().is_none());
    assert!(!app.should_quit());

    press(&mut app, [key(KeyCode::Char('q')), key(KeyCode::Esc)]);
    assert!(app.prompt().is_none());
    assert!(!app.should_quit());

    press(&mut app, [key(KeyCode::Char('q')), key(KeyCode::Char('y'))]);
    assert!(app.should_quit());

    // Ctrl-c does not ask.
    let mut app = guide(80, 24);
    press(&mut app, [key(KeyCode::Char('d')), ctrl('c')]);
    assert!(app.should_quit());
}

#[test]
fn search_hits_follow_the_edited_document() {
    let mut app = guide(80, 24);
    press(&mut app, [key(KeyCode::Char('/'))]);
    press(&mut app, "install".chars().map(|c| key(KeyCode::Char(c))));
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.find().unwrap().hits().len(), 2);
    // On `## Install`; deleting it leaves no match.
    press(&mut app, [key(KeyCode::Char('d'))]);
    assert_eq!(app.find().unwrap().hits().len(), 0);
    press(&mut app, [key(KeyCode::Char('u'))]);
    assert_eq!(app.find().unwrap().hits().len(), 2);
}

#[test]
fn history_locations_move_with_the_edits() {
    // Jump from `Options` to the top, then move `Usage` above `Install`.
    let mut app = guide_at(5);
    press(
        &mut app,
        [key(KeyCode::Tab), key(KeyCode::Home), key(KeyCode::Enter)],
    );
    assert_eq!(app.focus(), Panel::Document);
    assert_eq!(selected(&app), "# Guide");
    press(&mut app, vec![key(KeyCode::Down); 4]);
    press(&mut app, [alt(KeyCode::Up), key(KeyCode::Backspace)]);
    assert_eq!(selected(&app), "## Usage");
}

#[test]
fn shift_and_alt_only_edit_with_the_arrow_keys() {
    let app = guide(80, 24);
    let action = |code, modifiers| {
        let Event::Key(key) = key_with(code, modifiers) else {
            unreachable!()
        };
        mrq_tui::event::action(&key, &app)
    };
    assert_eq!(
        action(KeyCode::Left, KeyModifiers::SHIFT),
        Some(Action::Promote)
    );
    assert_eq!(
        action(KeyCode::Left, KeyModifiers::NONE),
        Some(Action::PreviousLink)
    );
    assert_eq!(
        action(KeyCode::Up, KeyModifiers::ALT),
        Some(Action::MoveSectionUp)
    );
    assert_eq!(action(KeyCode::Char('x'), KeyModifiers::ALT), None);
    assert_eq!(
        action(KeyCode::Char('N'), KeyModifiers::SHIFT),
        Some(Action::PreviousMatch)
    );
    assert_eq!(
        action(KeyCode::BackTab, KeyModifiers::SHIFT),
        Some(Action::PreviousPanel)
    );
    assert_eq!(
        action(KeyCode::Char('r'), KeyModifiers::CONTROL),
        Some(Action::Redo)
    );
    assert_eq!(
        action(KeyCode::Char('c'), KeyModifiers::CONTROL),
        Some(Action::ForceQuit)
    );
}

#[test]
fn edits_and_undo_are_listed_in_the_help() {
    let keys: Vec<&str> = mrq_tui::event::KEYS.iter().map(|(key, _)| *key).collect();
    for wanted in ["Shift-← →", "Alt-↑ ↓", "d / Delete", "u / Ctrl-r", "w"] {
        assert!(keys.contains(&wanted), "{wanted}");
    }
}

#[test]
fn plan_describes_an_edit_without_making_it() {
    let doc = Document::parse(GUIDE);
    let install = GUIDE.find("## Install").unwrap();
    let change = edit::plan(&doc, install, Structural::Demote).unwrap();
    assert_eq!(change.message, "demoted `Install`");
    assert_eq!(change.select, install);
    assert_eq!(doc.text(), GUIDE);

    let mut stack = UndoStack::default();
    let applied = stack.push(GUIDE, change, 0).unwrap();
    assert!(applied.text.contains("### Install"));
    assert!(stack.is_dirty());
    let undone = stack.undo(&applied.text).unwrap();
    assert_eq!(undone.text, GUIDE);
    assert_eq!(undone.select, 0);
    assert!(!stack.is_dirty());
    assert_eq!(stack.redo(&undone.text).unwrap().text, applied.text);
}

/// An app over the in-memory file `notes.md` with `text`.
fn app_over(text: &str) -> App {
    app_over_named(Some(PathBuf::from("notes.md")), text)
}

fn app_over_named(path: Option<PathBuf>, text: &str) -> App {
    let doc = DocumentOptions::new()
        .parse_source(SourceFile::new(path, text.to_owned()))
        .unwrap();
    app(vec![doc], 80, 24)
}

/// `count` level-2 sections with a paragraph each.
fn sections(count: usize) -> String {
    (0..count).fold(String::new(), |mut text, i| {
        writeln!(text, "## Section {i}\n\nText {i}.\n").unwrap();
        text
    })
}

/// A document of headings (levels 1–6, ATX or setext) and paragraphs, with a section in
/// a block quote now and then.
fn document() -> impl Strategy<Value = String> {
    let block = prop_oneof![
        3 => (1usize..=6, "[A-Za-z]{1,6}").prop_map(|(level, text)| {
            format!("{} {text}\n", "#".repeat(level))
        }),
        1 => "[A-Za-z]{1,6}".prop_map(|text| format!("{text}\n===\n")),
        1 => "[A-Za-z]{1,6}".prop_map(|text| format!("{text}\n---\n")),
        3 => "[a-z ]{1,12}".prop_map(|text| format!("{text}.\n")),
        1 => "[A-Za-z]{1,6}".prop_map(|text| format!("> ## {text}\n>\n> quoted.\n")),
    ];
    (prop::collection::vec(block, 1..14), any::<bool>()).prop_map(|(blocks, newline)| {
        let text = blocks.join("\n");
        if newline {
            text
        } else {
            text.trim_end().to_owned()
        }
    })
}

fn structural() -> impl Strategy<Value = Structural> {
    prop_oneof![
        Just(Structural::Promote),
        Just(Structural::Demote),
        Just(Structural::MoveUp),
        Just(Structural::MoveDown),
        Just(Structural::Delete),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Edits at arbitrary offsets, all undone and then all redone, give the original and
    /// the edited bytes back exactly.
    #[test]
    fn edit_undo_redo_round_trips(
        source in document(),
        edits in prop::collection::vec((structural(), any::<prop::sample::Index>()), 1..8),
    ) {
        let mut stack = UndoStack::default();
        let mut texts = vec![source.clone()];
        let mut doc = Document::parse(source);
        for (edit, at) in edits {
            let offset = at.index(doc.text().len() + 1);
            let Ok(change) = edit::plan(&doc, offset, edit) else { continue };
            let applied = stack.push(doc.text(), change, offset).unwrap();
            texts.push(applied.text.clone());
            doc = doc.with_text(applied.text);
        }
        let mut text = texts.last().unwrap().clone();
        for expected in texts.iter().rev().skip(1) {
            text = stack.undo(&text).unwrap().text;
            prop_assert_eq!(&text, expected);
        }
        prop_assert!(stack.undo(&text).is_none());
        prop_assert!(!stack.is_dirty());
        for expected in &texts[1..] {
            text = stack.redo(&text).unwrap().text;
            prop_assert_eq!(&text, expected);
        }
        prop_assert!(stack.redo(&text).is_none());
    }

    /// Every planned edit applies to the document it was planned for.
    #[test]
    fn planned_edits_apply(source in document(), edit in structural(), at in any::<prop::sample::Index>()) {
        let doc = Document::parse(source);
        let offset = at.index(doc.text().len() + 1);
        if let Ok(change) = edit::plan(&doc, offset, edit) {
            let text = mrq_core::source::apply_edits(doc.text(), &change.edits).unwrap();
            prop_assert!(change.select <= text.len());
        }
    }
}
