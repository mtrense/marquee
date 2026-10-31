//! Inline content as [`Piece`]s, wrapped into lines by display width.

use std::borrow::Cow;

use unicode_width::UnicodeWidthChar;

use super::{StyledSpan, TextStyle};

/// Inline content: styled text (with spaces where it may wrap), or a hard break.
#[derive(Debug, Clone)]
pub(super) enum Piece {
    Text(StyledSpan),
    Break,
}

impl Piece {
    pub(super) fn text(text: &str, style: TextStyle, link: Option<&str>) -> Self {
        Self::Text(StyledSpan {
            text: clean(text).into_owned(),
            style,
            link: link.map(str::to_owned),
        })
    }
}

/// `text` without control characters: tabs and line breaks become spaces, others
/// U+FFFD, so the source cannot send escape sequences to the terminal.
pub(super) fn clean(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| match c {
                '\t' | '\n' | '\r' => ' ',
                c if c.is_control() => '\u{fffd}',
                c => c,
            })
            .collect(),
    )
}

/// The display width of `c`; zero for combining marks and control characters.
pub(super) fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Appends `c` to `spans`, to the last span when it has the same style and link.
pub(super) fn push_char(
    spans: &mut Vec<StyledSpan>,
    c: char,
    style: TextStyle,
    link: Option<&String>,
) {
    match spans.last_mut() {
        Some(last) if last.style == style && last.link.as_ref() == link => last.text.push(c),
        _ => spans.push(StyledSpan {
            text: c.to_string(),
            style,
            link: link.cloned(),
        }),
    }
}

/// Appends `span` to `spans`, merging it into the last span when they look the same.
pub(super) fn push_span(spans: &mut Vec<StyledSpan>, span: StyledSpan) {
    match spans.last_mut() {
        Some(last) if last.style == span.style && last.link == span.link => {
            last.text.push_str(&span.text);
        }
        _ if span.text.is_empty() => {}
        _ => spans.push(span),
    }
}

/// `pieces` wrapped into lines of at most `width` columns: greedy, at whitespace (runs
/// collapse to one space, and none starts or ends a line), breaking words wider than a
/// line. At least one line.
pub(super) fn wrap(pieces: &[Piece], width: usize) -> Vec<Vec<StyledSpan>> {
    let mut wrapper = Wrapper {
        width: width.max(1),
        ..Wrapper::default()
    };
    for piece in pieces {
        match piece {
            Piece::Text(span) => {
                for c in span.text.chars() {
                    if c.is_whitespace() {
                        wrapper.place_word();
                        if wrapper.space.is_none() {
                            wrapper.space = Some(StyledSpan {
                                text: " ".to_owned(),
                                style: span.style,
                                link: span.link.clone(),
                            });
                        }
                    } else {
                        push_char(&mut wrapper.word, c, span.style, span.link.as_ref());
                        wrapper.word_width += char_width(c);
                    }
                }
            }
            Piece::Break => {
                wrapper.place_word();
                wrapper.end_line();
            }
        }
    }
    wrapper.place_word();
    if !wrapper.line.is_empty() || wrapper.lines.is_empty() {
        wrapper.end_line();
    }
    wrapper.lines
}

/// The width of `pieces` on one line per hard break: the widest of those lines.
pub(super) fn natural_width(pieces: &[Piece]) -> usize {
    wrap(pieces, usize::MAX)
        .iter()
        .map(|line| line.iter().map(StyledSpan::width).sum())
        .max()
        .unwrap_or(0)
}

/// The width of the widest word of `pieces`.
pub(super) fn longest_word(pieces: &[Piece]) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for piece in pieces {
        match piece {
            Piece::Text(span) => {
                for c in span.text.chars() {
                    if c.is_whitespace() {
                        current = 0;
                    } else {
                        current += char_width(c);
                        longest = longest.max(current);
                    }
                }
            }
            Piece::Break => current = 0,
        }
    }
    longest
}

#[derive(Default)]
struct Wrapper {
    width: usize,
    lines: Vec<Vec<StyledSpan>>,
    line: Vec<StyledSpan>,
    line_width: usize,
    word: Vec<StyledSpan>,
    word_width: usize,
    /// The space before the word, if any.
    space: Option<StyledSpan>,
}

impl Wrapper {
    fn end_line(&mut self) {
        self.lines.push(std::mem::take(&mut self.line));
        self.line_width = 0;
        self.space = None;
    }

    /// Puts the pending word on the line, or on the next one.
    fn place_word(&mut self) {
        if self.word.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word);
        let word_width = std::mem::take(&mut self.word_width);
        let space = self.space.take().filter(|_| !self.line.is_empty());
        let space_width = usize::from(space.is_some());
        if self.line_width + space_width + word_width <= self.width {
            if let Some(space) = space {
                push_span(&mut self.line, space);
                self.line_width += 1;
            }
        } else if !self.line.is_empty() {
            self.end_line();
        }
        if self.line_width + word_width <= self.width {
            for span in word {
                push_span(&mut self.line, span);
            }
            self.line_width += word_width;
            return;
        }
        // Wider than a line: break it anywhere.
        for span in word {
            for c in span.text.chars() {
                let w = char_width(c);
                if self.line_width + w > self.width && self.line_width > 0 {
                    self.end_line();
                }
                push_char(&mut self.line, c, span.style, span.link.as_ref());
                self.line_width += w;
            }
        }
    }
}

/// `text` (one line, no control characters) cut into pieces of at most `width` columns.
pub(super) fn break_line(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let mut line_width = 0;
    for c in text.chars() {
        let w = char_width(c);
        if line_width + w > width && line_width > 0 {
            lines.push(String::new());
            line_width = 0;
        }
        lines.last_mut().expect("never empty").push(c);
        line_width += w;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Piece {
        Piece::text(text, TextStyle::default(), None)
    }

    fn texts(lines: &[Vec<StyledSpan>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.iter().map(|s| s.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn spaces_collapse_and_never_start_a_line() {
        let lines = wrap(&[plain("  a   b  "), Piece::Break, plain(" c")], 3);
        assert_eq!(texts(&lines), ["a b", "c"]);
    }

    #[test]
    fn words_continue_across_styles() {
        let bold = TextStyle {
            bold: true,
            ..TextStyle::default()
        };
        let lines = wrap(
            &[plain("ab"), Piece::text("cd", bold, None), plain(" ef")],
            4,
        );
        assert_eq!(texts(&lines), ["abcd", "ef"]);
        assert_eq!(lines[0].len(), 2);
    }

    #[test]
    fn measures() {
        let pieces = [plain("one three"), Piece::Break, plain("seventeen")];
        assert_eq!(natural_width(&pieces), 9);
        assert_eq!(longest_word(&pieces), 9);
        assert_eq!(break_line("abcde", 2), ["ab", "cd", "e"]);
        assert_eq!(clean("a\tb\x1bc"), "a b\u{fffd}c");
    }
}
