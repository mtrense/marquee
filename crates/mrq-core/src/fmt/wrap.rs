//! Prose wrapping (`fmt.wrap`): a paragraph is printed as a [`Flow`] of words, and the
//! lines are chosen from it.
//!
//! A line may break only at whitespace of the paragraph's text and at its soft breaks
//! (each run of which is printed as one space when it does not break),
//! never inside a code span, a link destination or title, an autolink, raw HTML, math
//! or a comment, and never right before raw HTML or a comment (which could start an
//! HTML block). Hard breaks always end a line. Every line is escaped where it would
//! start a block (`Printer::line_start_marker`), once the lines are known.

use std::ops::Range;

use textwrap::core::Fragment;
use textwrap::wrap_algorithms::{Penalties, wrap_first_fit, wrap_optimal_fit};
use unicode_width::UnicodeWidthStr;

use super::Printer;
use super::block::split_lines;
use crate::ast::NodeId;
use crate::config::Wrap;

/// A paragraph's inlines printed for wrapping.
#[derive(Debug, Default)]
pub(super) struct Flow {
    /// The printed inlines; `\n` only after hard breaks and inside raw HTML, math or
    /// comments.
    pub(super) text: String,
    /// The whitespace runs of `text` where a line may break, in order.
    pub(super) breaks: Vec<Range<usize>>,
    /// The offsets of the `\n` of hard breaks.
    pub(super) hard: Vec<usize>,
    /// The raw HTML and comments of `text`.
    pub(super) raw: Vec<Range<usize>>,
}

/// The columns a block may fill: `width` on every line, less `lead` on its first (taken
/// by a task marker before it).
#[derive(Debug, Clone, Copy)]
pub(super) struct Room {
    pub(super) width: usize,
    pub(super) lead: usize,
}

impl Room {
    pub(super) const fn new(width: usize) -> Self {
        Self { width, lead: 0 }
    }

    /// The room inside a prefix `first` columns wide on the first line and `rest` on the
    /// others; at least one column is left.
    pub(super) fn inside(self, first: usize, rest: usize) -> Self {
        Self {
            width: self.width.saturating_sub(rest).max(1),
            lead: self.lead + first.saturating_sub(rest),
        }
    }

    /// The room of a block that does not start the first line.
    pub(super) const fn rest(self) -> Self {
        Self {
            width: self.width,
            lead: 0,
        }
    }
}

/// A word of a flow, what stands between two places where a line may break, and its
/// offset in the flow.
#[derive(Debug)]
struct Word<'a>(&'a str, usize);

impl Fragment for Word<'_> {
    #[allow(clippy::cast_precision_loss)] // line widths are far below 2^52
    fn width(&self) -> f64 {
        self.0.width() as f64
    }

    fn whitespace_width(&self) -> f64 {
        1.0
    }

    fn penalty_width(&self) -> f64 {
        0.0
    }
}

/// The words of one line of a flow, as it is between `\n`s.
struct Segment<'a> {
    words: Vec<Word<'a>>,
    /// It follows a hard break (or starts the paragraph), not a line break inside raw
    /// HTML, math or a comment.
    after_break: bool,
}

impl Printer<'_> {
    /// The lines of the paragraph `id`, wrapped following `fmt.wrap` in `room`.
    pub(super) fn paragraph(&self, id: NodeId, room: Room) -> Vec<String> {
        if self.config.wrap == Wrap::Preserve {
            return split_lines(&self.inlines(id, false));
        }
        let flow = self.flow(id);
        let mut lines = Vec::new();
        for (index, segment) in segments(&flow).iter().enumerate() {
            let groups = match self.config.wrap {
                Wrap::Width => {
                    let first = if index == 0 {
                        room.width.saturating_sub(room.lead).max(1)
                    } else {
                        room.width
                    };
                    fill(&segment.words, first, room.width)
                }
                Wrap::Sentence => sentences(&segment.words),
                Wrap::Never | Wrap::Preserve => std::iter::once(0..segment.words.len()).collect(),
            };
            for (n, group) in groups.into_iter().enumerate() {
                let words = &segment.words[group];
                let mut line = words
                    .iter()
                    .map(|word| word.0)
                    .collect::<Vec<_>>()
                    .join(" ");
                // A line inside raw HTML, math or a comment that spans lines stays as it
                // was, and so does raw HTML that starts a line: either started a line in
                // the source too.
                let starts_line = (n > 0 || segment.after_break)
                    && words
                        .first()
                        .is_some_and(|word| !flow.raw.iter().any(|raw| raw.start == word.1));
                if starts_line && let Some(at) = self.line_start_marker(&line) {
                    line.insert(at, '\\');
                }
                lines.push(line);
            }
        }
        lines
    }
}

/// `flow` split at its `\n`s, each part into words at its breaks.
fn segments(flow: &Flow) -> Vec<Segment<'_>> {
    let text = flow.text.as_str();
    let mut segments = Vec::new();
    let mut breaks = flow.breaks.iter().peekable();
    let mut start = 0;
    let mut after_break = true;
    loop {
        let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        let mut words = Vec::new();
        let mut word_start = start;
        while let Some(gap) = breaks.next_if(|gap| gap.end <= end) {
            words.push(Word(&text[word_start..gap.start], word_start));
            word_start = gap.end;
        }
        words.push(Word(&text[word_start..end], word_start));
        // A break at the start or end of a segment leaves an empty word.
        words.retain(|word| !word.0.is_empty());
        if !words.is_empty() {
            segments.push(Segment { words, after_break });
        }
        if end == text.len() {
            break;
        }
        after_break = flow.hard.contains(&end);
        start = end + 1;
    }
    segments
}

/// `words` in lines of at most `first` columns, then `rest`, by textwrap's optimal fit:
/// the index ranges of the lines' words.
#[allow(clippy::cast_precision_loss)]
fn fill(words: &[Word<'_>], first: usize, rest: usize) -> Vec<Range<usize>> {
    let widths = [first as f64, rest as f64];
    let lines = wrap_optimal_fit(words, &widths, &Penalties::new())
        .unwrap_or_else(|_| wrap_first_fit(words, &widths));
    let mut start = 0;
    lines
        .into_iter()
        .map(|line| {
            let range = start..start + line.len();
            start = range.end;
            range
        })
        .collect()
}

/// `words` with one sentence per line: the index ranges of the lines' words.
fn sentences(words: &[Word<'_>]) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, pair) in words.windows(2).enumerate() {
        if ends_sentence(pair[0].0, pair[1].0) {
            lines.push(start..i + 1);
            start = i + 1;
        }
    }
    lines.push(start..words.len());
    lines
}

/// What may close a sentence after its `.`, `!` or `?`: quotes, brackets and emphasis.
const CLOSING: &[char] = &['"', '\'', ')', ']', '*', '_', '~', '”', '’', '»', '›'];
/// What may open a sentence before its first letter.
const OPENING: &[char] = &['"', '\'', '(', '[', '*', '_', '~', '“', '‘', '«', '‹', '\\'];

/// Abbreviations that end with a period but rarely end a sentence, in lowercase.
const ABBREVIATIONS: &[&str] = &[
    "al.", "approx.", "ca.", "cf.", "ch.", "co.", "dept.", "dr.", "e.g.", "eq.", "esp.", "etc.",
    "fig.", "i.e.", "inc.", "jr.", "ltd.", "mr.", "mrs.", "ms.", "no.", "p.", "pp.", "prof.",
    "resp.", "sec.", "sr.", "st.", "viz.", "vol.", "vs.",
];

/// Whether the sentence ends after `word` when `next` follows: the word ends in `.`, `!`
/// or `?` (before closing quotes, brackets or emphasis), is no abbreviation or initial,
/// and `next` does not start in lowercase.
fn ends_sentence(word: &str, next: &str) -> bool {
    let core = word.trim_end_matches(CLOSING);
    if !core.ends_with(['.', '!', '?', '…']) {
        return false;
    }
    if core.ends_with('.') && is_abbreviation(core.trim_start_matches(OPENING)) {
        return false;
    }
    !next
        .trim_start_matches(OPENING)
        .chars()
        .next()
        .is_some_and(char::is_lowercase)
}

/// Whether `word` (ending with `.`) is a known abbreviation, an initial (`J.`) or dotted
/// like `U.S.`.
fn is_abbreviation(word: &str) -> bool {
    let lower = word.to_lowercase();
    let body = &word[..word.len() - 1];
    ABBREVIATIONS.contains(&lower.as_str())
        || (body.chars().count() == 1 && body.chars().all(char::is_alphabetic))
        || (body.contains('.') && body.chars().all(|c| c == '.' || c.is_alphabetic()))
}
