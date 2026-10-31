//! Search and query state: the `/` prompt searching incrementally over the rendered text
//! (smart case), Enter and Esc, `n`/`N` with wraparound, the `>` query prompt with its
//! `css:` and `jq:` prefixes, query errors, the history, other files and layouts; and
//! snapshots of the highlighting.

mod common;

use common::{GUIDE, NOTES, doc, draw, guide, key, key_with, press, selected, several};
use mrq_core::query::{Query, QueryError};
use mrq_tui::find::find_all;
use mrq_tui::{App, Event, Location};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::Color;

/// The key presses that type `text`.
fn typed(text: &str) -> Vec<Event> {
    text.chars().map(|c| key(KeyCode::Char(c))).collect()
}

/// Opens the prompt `label` (`/` or `>`) and types `text` into it.
fn ask(app: &mut App, label: char, text: &str) {
    press(app, [key(KeyCode::Char(label))]);
    press(app, typed(text));
}

/// The number of hits of the active search or query.
fn hits(app: &App) -> usize {
    app.find().map_or(0, |find| find.hits().len())
}

/// Every search hit's text as drawn, which `pattern` must match ignoring case.
fn assert_hits_match(app: &App, pattern: &str) {
    let lines = app.file().lines();
    let find = app.find().unwrap();
    assert_ne!(find.hits(), []);
    for hit in find.hits() {
        let text = lines[hit.line].text();
        assert!(
            text[hit.columns.clone()].eq_ignore_ascii_case(pattern),
            "{text}"
        );
    }
}

#[test]
fn find_all_is_smart_case() {
    assert_eq!(
        find_all("Run the Installer, install", "install"),
        [8..15, 19..26]
    );
    let none: [std::ops::Range<usize>; 0] = [];
    assert_eq!(
        find_all("Run the Installer, install", "Install"),
        vec![8..15]
    );
    assert_eq!(find_all("Run the Installer", "INSTALL"), none);
    assert_eq!(
        find_all("GRÖSSE größe", "ö"),
        ["GR".len().."GRÖ".len(), 10..12]
    );
    assert_eq!(find_all("aaaa", "aa"), [0..2, 2..4], "without overlaps");
    assert_eq!(find_all("text", ""), none);
}

#[test]
fn slash_opens_a_prompt_that_searches_as_you_type() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Char('/'))]);
    let prompt = app.prompt().unwrap();
    assert_eq!((prompt.label(), prompt.text()), ("/", ""));
    assert_eq!(app.find(), None);

    press(&mut app, typed("inst"));
    assert_eq!(app.prompt().unwrap().text(), "inst");
    assert_eq!(hits(&app), 2);
    assert_hits_match(&app, "inst");
    assert_eq!(selected(&app), "## Install");
    assert_eq!(app.file().outline_cursor(), 1);
    assert_eq!(app.status(), Some("match 1/2"));

    press(&mut app, typed("aller"));
    assert_eq!(hits(&app), 1);
    assert_eq!(selected(&app), "Run the installer.");
    // Deleting characters searches again from where the search started.
    press(&mut app, vec![key(KeyCode::Backspace); 5]);
    assert_eq!(selected(&app), "## Install");
    press(&mut app, typed("zz"));
    assert_eq!(hits(&app), 0);
    assert_eq!(app.status(), Some("no match"));
    assert_eq!(selected(&app), "# Guide", "back where the search started");
}

#[test]
fn the_search_is_smart_case() {
    let mut app = guide(80, 16);
    ask(&mut app, '/', "options");
    assert_eq!(hits(&app), 2, "heading and paragraph");
    press(&mut app, [key(KeyCode::Esc)]);
    ask(&mut app, '/', "Options");
    assert_eq!(
        hits(&app),
        1,
        "an upper-case letter makes it case-sensitive"
    );
}

#[test]
fn esc_cancels_and_restores_the_position_and_the_previous_search() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Down)]);
    let before = selected(&app);
    ask(&mut app, '/', "usage");
    assert_eq!(selected(&app), "## Usage");
    press(&mut app, [key(KeyCode::Esc)]);
    assert_eq!(app.prompt(), None);
    assert_eq!(app.find(), None);
    assert_eq!(selected(&app), before);
    assert_eq!(app.file().outline_cursor(), 0);
    assert_eq!(app.history(), []);

    ask(&mut app, '/', "install");
    press(&mut app, [key(KeyCode::Enter)]);
    ask(&mut app, '/', "options");
    press(&mut app, [key(KeyCode::Esc)]);
    assert_eq!(app.find().unwrap().pattern(), "install");
    assert_eq!(selected(&app), "## Install");
}

#[test]
fn enter_confirms_and_backspace_goes_back_to_where_the_search_started() {
    let mut app = guide(80, 16);
    ask(&mut app, '/', "options");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.prompt(), None);
    assert_eq!(selected(&app), "### Options");
    assert_eq!(app.status(), Some("match 1/2"));
    assert_eq!(app.find().unwrap().current(), Some(0));
    assert_eq!(app.history(), [Location { file: 0, offset: 0 }]);

    press(&mut app, [key(KeyCode::Char('n'))]);
    assert!(selected(&app).starts_with("Pass "));
    assert_eq!(app.status(), Some("match 2/2"));
    press(&mut app, [key(KeyCode::Char('n'))]);
    assert_eq!(selected(&app), "### Options", "wraps around");
    assert_eq!(app.status(), Some("match 1/2"));
    press(&mut app, [key(KeyCode::Char('N'))]);
    assert_eq!(app.status(), Some("match 2/2"), "wraps around backwards");
    assert_eq!(app.history().len(), 1, "n and N are not jumps");

    press(&mut app, [key(KeyCode::Backspace)]);
    assert_eq!(selected(&app), "# Guide");
    assert!(app.find().is_some(), "the matches stay");
}

#[test]
fn the_search_starts_at_the_selection_and_wraps_around() {
    let mut app = guide(80, 16);
    press(&mut app, vec![key(KeyCode::Down); 4]);
    assert_eq!(selected(&app), "## Usage");
    ask(&mut app, '/', "install");
    assert_eq!(selected(&app), "## Install");
    assert_eq!(app.status(), Some("match 1/2"));
}

#[test]
fn several_matches_on_a_line_are_visited_in_turn() {
    let text = "# Title\n\nab ab\n\nab\n";
    let mut app = common::app(vec![doc("ab.md", text)], 80, 16);
    ask(&mut app, '/', "ab");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(selected(&app), "ab ab");
    press(&mut app, [key(KeyCode::Char('n'))]);
    assert_eq!(selected(&app), "ab ab");
    assert_eq!(app.status(), Some("match 2/3"));
    press(&mut app, [key(KeyCode::Char('n'))]);
    assert_eq!(selected(&app), "ab");
}

#[test]
fn n_goes_on_from_a_moved_selection() {
    let mut app = guide(80, 16);
    ask(&mut app, '/', "options");
    press(&mut app, [key(KeyCode::Enter), key(KeyCode::Home)]);
    press(&mut app, [key(KeyCode::Char('n'))]);
    assert_eq!(selected(&app), "### Options");
    assert_eq!(app.status(), Some("match 1/2"));
    press(&mut app, [key(KeyCode::Home), key(KeyCode::Char('N'))]);
    assert!(
        selected(&app).starts_with("Pass "),
        "the last one, wrapping"
    );
}

#[test]
fn a_search_without_matches_says_so_and_stays_put() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Down)]);
    ask(&mut app, '/', "zzz");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.status(), Some("no match for `zzz`"));
    assert_eq!(app.find(), None);
    assert!(selected(&app).starts_with("An introduction"));
    assert_eq!(app.history(), []);
}

#[test]
fn an_empty_search_is_cancelled() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Char('/')), key(KeyCode::Enter)]);
    assert_eq!(app.prompt(), None);
    assert_eq!(app.find(), None);
    assert_eq!(app.status(), None);
}

#[test]
fn n_without_a_search_says_so() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Char('n'))]);
    assert_eq!(app.status(), Some("no search or query to repeat"));
}

#[test]
fn esc_clears_the_matches() {
    let mut app = guide(80, 16);
    ask(&mut app, '/', "install");
    press(&mut app, [key(KeyCode::Enter), key(KeyCode::Esc)]);
    assert_eq!(app.find(), None);
    assert_eq!(selected(&app), "## Install");
}

#[test]
fn keys_go_to_the_prompt_while_it_is_open() {
    let mut app = guide(80, 16);
    // `q`, `?`, `o`, Tab and the arrows would do something else.
    ask(&mut app, '/', "q?o");
    press(&mut app, [key(KeyCode::Tab), key(KeyCode::Left)]);
    assert!(!app.should_quit());
    assert_eq!(app.overlay(), None);
    assert!(app.outline_visible());
    assert_eq!(app.prompt().unwrap().text(), "q?o");
    assert_eq!(app.prompt().unwrap().cursor(), 2);
    // Ctrl-c still quits.
    press(
        &mut app,
        [key_with(KeyCode::Char('c'), KeyModifiers::CONTROL)],
    );
    assert!(app.should_quit());
}

#[test]
fn the_search_follows_a_new_layout() {
    let mut app = guide(80, 16);
    ask(&mut app, '/', "narrow");
    press(&mut app, [key(KeyCode::Enter)]);
    let line = app.find().unwrap().hits()[0].line;
    app.handle(Event::Resize(120, 16));
    assert_hits_match(&app, "narrow");
    assert_ne!(app.find().unwrap().hits()[0].line, line, "unwrapped");
}

#[test]
fn the_search_applies_to_the_shown_file() {
    let mut app = several(100, 20);
    ask(&mut app, '/', "o");
    press(&mut app, [key(KeyCode::Enter)]);
    // Show notes.md from the file list.
    press(
        &mut app,
        [key(KeyCode::Tab), key(KeyCode::Down), key(KeyCode::Enter)],
    );
    assert_eq!(app.current(), 1);
    assert_eq!(hits(&app), 3, "{NOTES}");
    assert_eq!(app.find().unwrap().current(), None);
    press(&mut app, [key(KeyCode::Char('n'))]);
    assert_eq!(selected(&app), "# Notes");
    assert_eq!(app.status(), Some("match 1/3"));
}

#[test]
fn the_search_focuses_the_document() {
    let mut app = guide(80, 16);
    press(&mut app, [key(KeyCode::Tab)]);
    ask(&mut app, '/', "usage");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.focus(), mrq_tui::Panel::Document);
}

#[test]
fn a_query_selects_its_matches_and_n_goes_through_them() {
    let mut app = guide(80, 16);
    ask(&mut app, '>', "# Usage");
    assert_eq!(app.prompt().unwrap().label(), ">");
    assert_eq!(app.find(), None, "a query runs on Enter only");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.prompt(), None);
    let find = app.find().unwrap();
    assert!(find.is_query());
    assert_eq!(find.hits().len(), 1, "the section");
    assert_eq!(selected(&app), "## Usage");
    assert_eq!(app.status(), Some("match 1/1"));
    assert_eq!(app.history(), [Location { file: 0, offset: 0 }]);
    // The section's blocks are marked, the others are not.
    let view = app.file().view();
    let marked: Vec<bool> = (0..view.blocks().len())
        .map(|i| find.is_marked(i))
        .collect();
    assert_eq!(marked, [false, false, false, false, true, true, true]);
}

#[test]
fn css_and_jq_prefixes_choose_the_syntax() {
    let mut app = guide(80, 16);
    ask(&mut app, '>', "css:heading");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(hits(&app), 4);
    assert_eq!(app.status(), Some("match 1/4"));
    press(&mut app, [key(KeyCode::Char('n')), key(KeyCode::Char('n'))]);
    assert_eq!(selected(&app), "## Usage");

    ask(&mut app, '>', r#"jq:.. | select(.type? == "inlineCode")"#);
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(hits(&app), 1, "the inline code");
    assert!(selected(&app).starts_with("Pass "));
}

#[test]
fn a_query_without_matches_says_so() {
    let mut app = guide(80, 16);
    ask(&mut app, '>', "- [ ]");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.status(), Some("no match for `- [ ]`"));
    assert_eq!(app.find(), None);
    assert_eq!(app.prompt(), None);
    // jq values that are not nodes have nowhere to go.
    ask(&mut app, '>', "jq:.. | .type?");
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.status(), Some("no match for `jq:.. | .type?`"));
}

#[test]
fn a_query_error_keeps_the_prompt_open_with_the_cursor_on_the_problem() {
    let mut app = guide(80, 16);
    ask(&mut app, '>', "css:heading[");
    press(&mut app, [key(KeyCode::Enter)]);
    let Err(QueryError::Syntax(err)) = Query::css("heading[") else {
        panic!("`heading[` is a CSS syntax error");
    };
    let column = "css:".len() + err.offset();
    let expected = format!("invalid query at column {}: {}", column + 1, err.message());
    assert_eq!(app.status(), Some(expected.as_str()));
    let prompt = app.prompt().unwrap();
    assert_eq!(prompt.text(), "css:heading[");
    assert_eq!(prompt.cursor(), column);
    assert_eq!(app.find(), None);
    assert!(draw(&app).contains("invalid query"));
    // Fixing it runs it.
    press(
        &mut app,
        [
            key(KeyCode::End),
            key(KeyCode::Backspace),
            key(KeyCode::Enter),
        ],
    );
    assert_eq!(app.prompt(), None);
    assert_eq!(hits(&app), 4);
}

#[test]
fn other_query_errors_are_status_messages() {
    let mut app = guide(80, 16);
    ask(&mut app, '>', r#"jq:error("boom")"#);
    press(&mut app, [key(KeyCode::Enter)]);
    assert_eq!(app.status(), Some("jq error: boom"));
    assert!(app.prompt().is_some());
}

#[test]
fn esc_cancels_a_query() {
    let mut app = guide(80, 16);
    ask(&mut app, '>', "# Usage");
    press(&mut app, [key(KeyCode::Esc)]);
    assert_eq!(app.prompt(), None);
    assert_eq!(app.find(), None);
    assert_eq!(selected(&app), "# Guide");
}

#[test]
fn search_typed() {
    let mut app = guide(80, 16);
    ask(&mut app, '/', "install");
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn search_confirmed() {
    let mut app = guide(80, 16);
    ask(&mut app, '/', "install");
    press(&mut app, [key(KeyCode::Enter), key(KeyCode::Char('n'))]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn query_marked() {
    let mut app = guide(80, 20);
    ask(&mut app, '>', "css:heading");
    press(&mut app, [key(KeyCode::Enter)]);
    insta::assert_snapshot!(draw(&app));
}

#[test]
fn query_error() {
    let mut app = guide(80, 16);
    ask(&mut app, '>', "# A | ?");
    press(&mut app, [key(KeyCode::Enter)]);
    insta::assert_snapshot!(draw(&app));
}

/// The background colours of the cells of `text` where it is drawn in `app`, by row.
fn backgrounds(app: &App, text: &str) -> Vec<Vec<Color>> {
    let (width, height) = app.size();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| mrq_tui::ui::draw(frame, app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut found = Vec::new();
    for y in 0..height {
        let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
        let Some(at) = row.find(text) else { continue };
        let x = u16::try_from(row[..at].chars().count()).unwrap();
        let len = u16::try_from(text.chars().count()).unwrap();
        found.push((x..x + len).map(|x| buffer[(x, y)].bg).collect());
    }
    found
}

#[test]
fn matches_are_highlighted_and_the_current_one_stands_out() {
    let mut app = guide(80, 16);
    // Without the outline, which shows `Install` too.
    press(&mut app, [key(KeyCode::Char('o'))]);
    ask(&mut app, '/', "install");
    press(&mut app, [key(KeyCode::Enter)]);
    let current = backgrounds(&app, "Install");
    assert_eq!(current, [vec![Color::Cyan; 7]]);
    let other = backgrounds(&app, "install");
    assert_eq!(other, [vec![Color::Yellow; 7]]);
    // Only the match: the text around it keeps its colours.
    assert_ne!(backgrounds(&app, "Run")[0][0], Color::Yellow);
}

#[test]
fn the_document_without_a_search_is_unchanged() {
    let mut app = guide(80, 16);
    let before = draw(&app);
    ask(&mut app, '/', "install");
    press(&mut app, [key(KeyCode::Esc)]);
    assert_eq!(draw(&app), before);
    assert!(GUIDE.contains("installer"));
}
