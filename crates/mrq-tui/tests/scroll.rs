//! Smooth scrolling on a long document: a 10k-line layout must take a key and draw a
//! frame well within the budget of a 60 Hz frame, since only the visible lines are drawn
//! and moving the selection never walks the whole layout.
//!
//! Timings are printed (`cargo test -p mrq-tui --test scroll -- --nocapture`); the budget
//! holds for unoptimized test builds, so a release build has ample headroom.

mod common;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use common::{app, doc, draw, key};
use mrq_tui::App;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;

/// The most a key and a frame may take on average.
const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// The most laying out the whole document may take once (start, resize). Generous: CI
/// runners take twice as long as a laptop on an unoptimized build, and coverage
/// instrumentation slows it down further; the per-frame budget is what scrolling feels.
const LAYOUT_BUDGET: Duration = Duration::from_secs(20);

/// About 10 000 lines at 100 columns: sections with a paragraph, a list and code.
fn long_document() -> String {
    let mut text = String::from("# Long\n\n");
    for i in 0..750 {
        write!(
            text,
            "## Section {i}\n\nA paragraph of section {i} that is long enough to wrap onto a \
             second line at the width of the document panel, more or less.\n\n- one\n- two\n\
             - three\n\n```rust\nfn section_{i}() {{\n    println!(\"{i}\");\n}}\n```\n\n"
        )
        .unwrap();
    }
    text
}

/// Sends each of `keys` to `app` and draws a frame after each, returning the average time.
fn frames(app: &mut App, terminal: &mut Terminal<TestBackend>, keys: &[KeyCode]) -> Duration {
    let start = Instant::now();
    for &code in keys {
        app.handle(key(code));
        terminal
            .draw(|frame| mrq_tui::ui::draw(frame, app))
            .unwrap();
    }
    start.elapsed() / u32::try_from(keys.len()).unwrap()
}

#[test]
fn scrolling_a_10k_line_document_stays_within_the_frame_budget() {
    let text = long_document();
    let start = Instant::now();
    let mut app = app(vec![doc("long.md", &text)], 120, 40);
    let layout = start.elapsed();
    let lines = app.file().lines().len();
    assert!(lines >= 10_000, "{lines} lines");
    assert!(layout < LAYOUT_BUDGET, "layout took {layout:?}");

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let down = frames(&mut app, &mut terminal, &[KeyCode::Down; 500]);
    let pages = frames(&mut app, &mut terminal, &[KeyCode::PageDown; 300]);
    let jumps = frames(
        &mut app,
        &mut terminal,
        &[KeyCode::End, KeyCode::Home].repeat(50),
    );
    // The bottom of the document draws as fast as the top.
    app.handle(key(KeyCode::End));
    let up = frames(&mut app, &mut terminal, &[KeyCode::Up; 500]);
    eprintln!(
        "{lines} lines laid out in {layout:?}; per key and frame: down {down:?}, page down \
         {pages:?}, end/home {jumps:?}, up at the end {up:?}"
    );
    for (what, took) in [
        ("down", down),
        ("page down", pages),
        ("end/home", jumps),
        ("up", up),
    ] {
        assert!(took < FRAME_BUDGET, "{what}: {took:?} per frame");
    }
    app.handle(key(KeyCode::End));
    assert!(draw(&app).contains("section_749"));
}
