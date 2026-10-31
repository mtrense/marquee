//! The input line: typing, moving the cursor, deleting, and what Enter and Esc report.

use mrq_tui::prompt::{Outcome, Prompt};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Types `text` into `prompt`, one key per character.
fn type_in(prompt: &mut Prompt, text: &str) {
    for c in text.chars() {
        assert_eq!(prompt.handle(&key(KeyCode::Char(c))), Outcome::Changed);
    }
}

#[test]
fn starts_empty_with_its_label() {
    let prompt = Prompt::new("/");
    assert_eq!(prompt.label(), "/");
    assert_eq!(prompt.text(), "");
    assert_eq!(prompt.cursor(), 0);
}

#[test]
fn typing_inserts_at_the_cursor() {
    let mut prompt = Prompt::new(">");
    type_in(&mut prompt, "ac");
    assert_eq!(prompt.handle(&key(KeyCode::Left)), Outcome::Moved);
    type_in(&mut prompt, "b");
    assert_eq!(prompt.text(), "abc");
    assert_eq!(prompt.cursor(), 2);
    // Shifted characters are text too.
    let shifted = KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT);
    assert_eq!(prompt.handle(&shifted), Outcome::Changed);
    assert_eq!(prompt.text(), "abDc");
}

#[test]
fn the_cursor_moves_by_characters_and_stops_at_the_ends() {
    let mut prompt = Prompt::new("/");
    type_in(&mut prompt, "größe");
    assert_eq!(prompt.cursor(), "größe".len());
    assert_eq!(prompt.handle(&key(KeyCode::Right)), Outcome::Ignored);
    prompt.handle(&key(KeyCode::Left));
    prompt.handle(&key(KeyCode::Left));
    prompt.handle(&key(KeyCode::Left));
    assert_eq!(prompt.cursor(), "gr".len());
    assert_eq!(prompt.cursor_column(), 2);
    assert_eq!(prompt.handle(&key(KeyCode::Home)), Outcome::Moved);
    assert_eq!(prompt.cursor(), 0);
    assert_eq!(prompt.handle(&key(KeyCode::Left)), Outcome::Ignored);
    assert_eq!(prompt.handle(&key(KeyCode::End)), Outcome::Moved);
    assert_eq!(prompt.cursor_column(), 5);
}

#[test]
fn backspace_and_delete_remove_a_character() {
    let mut prompt = Prompt::new("/");
    type_in(&mut prompt, "aöc");
    prompt.handle(&key(KeyCode::Left));
    assert_eq!(prompt.handle(&key(KeyCode::Backspace)), Outcome::Changed);
    assert_eq!(prompt.text(), "ac");
    assert_eq!(prompt.cursor(), 1);
    assert_eq!(prompt.handle(&key(KeyCode::Delete)), Outcome::Changed);
    assert_eq!(prompt.text(), "a");
    assert_eq!(prompt.handle(&key(KeyCode::Delete)), Outcome::Ignored);
    prompt.handle(&key(KeyCode::Home));
    assert_eq!(prompt.handle(&key(KeyCode::Backspace)), Outcome::Ignored);
    assert_eq!(prompt.text(), "a");
}

#[test]
fn enter_submits_and_esc_cancels() {
    let mut prompt = Prompt::new("/");
    type_in(&mut prompt, "x");
    assert_eq!(prompt.handle(&key(KeyCode::Enter)), Outcome::Submit);
    assert_eq!(prompt.handle(&key(KeyCode::Esc)), Outcome::Cancel);
    assert_eq!(prompt.text(), "x", "the text is left for the caller");
}

#[test]
fn control_keys_releases_and_other_keys_are_ignored() {
    let mut prompt = Prompt::new("/");
    let control = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(prompt.handle(&control), Outcome::Ignored);
    let alt = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
    assert_eq!(prompt.handle(&alt), Outcome::Ignored);
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    assert_eq!(prompt.handle(&release), Outcome::Ignored);
    assert_eq!(prompt.handle(&key(KeyCode::F(1))), Outcome::Ignored);
    assert_eq!(prompt.text(), "");
}

#[test]
fn set_cursor_clamps_to_a_character_boundary() {
    let mut prompt = Prompt::new(">");
    type_in(&mut prompt, "aö");
    prompt.set_cursor(2);
    assert_eq!(prompt.cursor(), 1, "inside `ö`, so before it");
    prompt.set_cursor(99);
    assert_eq!(prompt.cursor(), "aö".len());
}
