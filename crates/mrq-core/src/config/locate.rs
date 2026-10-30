//! Source positions of keys in a YAML document, by key path.
//!
//! figment keeps values and their provider but not line numbers, so each config file
//! is also scanned with the span-aware event parser behind serde-saphyr
//! (`granit-parser`) to map key paths to positions.

use std::collections::HashMap;

use serde_saphyr::granit_parser::{Event, Parser, Span};

use crate::source::ByteRange;

/// A position in a YAML text: 1-based line and column (in characters), and the byte range
/// of the key (for mapping entries) or the item (for sequence entries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pos {
    pub line: usize,
    pub column: usize,
    pub range: ByteRange,
}

impl Pos {
    pub(crate) fn from_span(span: &Span) -> Self {
        let start = span.start.byte_offset().unwrap_or(0);
        let end = span.end.byte_offset().unwrap_or(start).max(start);
        Self {
            line: span.start.line(),
            column: span.start.col() + 1,
            range: ByteRange::new(start, end),
        }
    }
}

/// Key path → position. Mapping keys are path segments as written; sequence items use
/// their index as a decimal segment.
#[derive(Debug, Clone, Default)]
pub(crate) struct KeyIndex {
    entries: HashMap<Vec<String>, Pos>,
}

enum Frame {
    Map { key: Option<String> },
    Seq { next: usize },
}

impl KeyIndex {
    /// Indexes `text`. Positions are collected up to the first syntax error.
    pub(crate) fn build(text: &str) -> Self {
        let mut index = Self::default();
        let mut stack: Vec<Frame> = Vec::new();
        let mut path: Vec<String> = Vec::new();
        // Depth inside a complex (non-scalar) mapping key, which is not indexed.
        let mut skip = 0usize;

        for event in Parser::new_from_str(text) {
            let Ok((event, span)) = event else { break };
            let opens = matches!(event, Event::MappingStart(..) | Event::SequenceStart(..));
            let closes = matches!(event, Event::MappingEnd | Event::SequenceEnd);
            let is_node = opens || matches!(event, Event::Scalar(..) | Event::Alias(_));

            if skip > 0 {
                if opens {
                    skip += 1;
                } else if closes {
                    skip -= 1;
                }
                if skip == 0 {
                    // The complex key ended; its value follows.
                    if let Some(Frame::Map { key }) = stack.last_mut() {
                        *key = Some(String::new());
                        path.push(String::new());
                    }
                }
                continue;
            }

            if closes {
                stack.pop();
                Self::value_done(&mut stack, &mut path);
                continue;
            }
            if !is_node {
                continue;
            }

            match stack.last_mut() {
                Some(Frame::Map { key }) if key.is_none() => {
                    if let Event::Scalar(value, ..) = &event {
                        *key = Some(value.to_string());
                        path.push(value.to_string());
                        index.entries.insert(path.clone(), Pos::from_span(&span));
                    } else if opens {
                        skip = 1;
                    } else {
                        *key = Some(String::new());
                        path.push(String::new());
                    }
                    continue;
                }
                Some(Frame::Seq { next }) => {
                    path.push(next.to_string());
                    *next += 1;
                    index.entries.insert(path.clone(), Pos::from_span(&span));
                }
                _ => {}
            }

            if opens {
                stack.push(if matches!(event, Event::MappingStart(..)) {
                    Frame::Map { key: None }
                } else {
                    Frame::Seq { next: 0 }
                });
            } else {
                Self::value_done(&mut stack, &mut path);
            }
        }
        index
    }

    /// A value finished in the innermost container: drop its path segment.
    fn value_done(stack: &mut [Frame], path: &mut Vec<String>) {
        match stack.last_mut() {
            Some(Frame::Map { key }) => {
                if key.take().is_some() {
                    path.pop();
                }
            }
            Some(Frame::Seq { .. }) => {
                path.pop();
            }
            None => {}
        }
    }

    /// Position of the key or item at `path`.
    pub(crate) fn get<S: AsRef<str>>(&self, path: &[S]) -> Option<Pos> {
        let key: Vec<String> = path.iter().map(|s| s.as_ref().to_owned()).collect();
        self.entries.get(&key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(index: &KeyIndex, path: &[&str]) -> Option<usize> {
        index.get(path).map(|pos| pos.line)
    }

    #[test]
    fn indexes_nested_keys_and_sequence_items() {
        let text = "a: 1\nb:\n  c: 2\n  d:\n    - x\n    - e: 3\n      f: [4, 5]\ng: { h: 6 }\n";
        let index = KeyIndex::build(text);
        assert_eq!(line(&index, &["a"]), Some(1));
        assert_eq!(line(&index, &["b"]), Some(2));
        assert_eq!(line(&index, &["b", "c"]), Some(3));
        assert_eq!(line(&index, &["b", "d"]), Some(4));
        assert_eq!(line(&index, &["b", "d", "0"]), Some(5));
        assert_eq!(line(&index, &["b", "d", "1"]), Some(6));
        assert_eq!(line(&index, &["b", "d", "1", "e"]), Some(6));
        assert_eq!(line(&index, &["b", "d", "1", "f"]), Some(7));
        assert_eq!(line(&index, &["b", "d", "1", "f", "1"]), Some(7));
        assert_eq!(line(&index, &["g", "h"]), Some(8));
        assert_eq!(line(&index, &["c"]), None);
    }

    #[test]
    fn positions_carry_columns_and_byte_ranges() {
        let text = "é: 1\nfmt:\n  wrapp: never\n";
        let pos = KeyIndex::build(text).get(&["fmt", "wrapp"]).unwrap();
        assert_eq!((pos.line, pos.column), (3, 3));
        assert_eq!(&text[pos.range.start..pos.range.end], "wrapp");
    }

    #[test]
    fn complex_keys_are_skipped() {
        let text = "? [a, b]\n: 1\nc: 2\n";
        let index = KeyIndex::build(text);
        assert_eq!(line(&index, &["c"]), Some(3));
    }

    #[test]
    fn stops_at_syntax_errors() {
        let index = KeyIndex::build("a: 1\nb: [\n");
        assert_eq!(line(&index, &["a"]), Some(1));
    }
}
