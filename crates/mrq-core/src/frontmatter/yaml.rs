//! YAML frontmatter: values via serde-saphyr, spans via its event parser (`granit-parser`).

use std::collections::HashMap;

use serde_json::Value;
use serde_saphyr::granit_parser::{Event, Parser, Span as YamlSpan};

use super::{EntrySpan, FrontmatterError, Index, Parsed, trimmed};
use crate::ast::Span;

/// Parses `text` (the frontmatter content) into a value and its span index. Spans and the
/// error range are relative to `text`.
pub(super) fn parse(text: &str) -> Result<Parsed, FrontmatterError> {
    let value = match serde_saphyr::from_str::<Value>(text) {
        Ok(Value::Null) => Value::Object(serde_json::Map::new()),
        Ok(value) => value,
        Err(err) => {
            let at = err.location().map_or(0, |loc| {
                offset_of(text, to_usize(loc.line()), to_usize(loc.column()))
            });
            return Err(FrontmatterError {
                message: format!("invalid YAML frontmatter: {}", message(&err.to_string())),
                range: char_at(text, at),
            });
        }
    };
    Ok(Parsed {
        value,
        entries: index(text),
    })
}

/// Key path → key and value spans, for every mapping entry and sequence item in `text`.
/// Mapping keys are path segments as written; sequence items use their index as a decimal
/// segment. The root node has the empty path. Entries are collected up to the first syntax
/// error; containers left open by it are missing.
pub(crate) fn index(text: &str) -> Index {
    let mut entries = HashMap::new();
    let mut stack: Vec<Frame> = Vec::new();
    // Depth inside a complex (non-scalar) mapping key, which is not indexed.
    let mut skip = 0usize;

    for event in Parser::new_from_str(text) {
        let Ok((event, span)) = event else { break };
        let opens = matches!(event, Event::MappingStart(..) | Event::SequenceStart(..));
        let closes = matches!(event, Event::MappingEnd | Event::SequenceEnd);
        let is_node = opens || matches!(event, Event::Scalar(..) | Event::Alias(_));
        let (start, end) = offsets(&span);

        if skip > 0 {
            if opens {
                skip += 1;
            } else if closes {
                skip -= 1;
            }
            if skip == 0 {
                // The complex key ended; its value follows under an empty segment.
                if let Some(Frame {
                    kind: Kind::Map { key },
                    ..
                }) = stack.last_mut()
                {
                    *key = Some((String::new(), None));
                }
            }
            continue;
        }

        if closes {
            let Some(frame) = stack.pop() else { continue };
            // Block collections end with an empty span at the next token; flow ones at
            // their closing bracket.
            let end = if start < end { end } else { frame.end };
            let value = trimmed(text, Span::new(frame.start, end.max(frame.start)));
            entries.insert(frame.path, EntrySpan::new(frame.key, value));
            value_done(&mut stack, value.end);
            continue;
        }
        if !is_node {
            continue;
        }

        // Where this node goes: a mapping key, a mapping value, a sequence item or the root.
        let (path, key) = match stack.last_mut() {
            Some(Frame {
                kind: Kind::Map { key: key @ None },
                end: frame_end,
                ..
            }) => {
                match &event {
                    Event::Scalar(name, ..) => {
                        *key = Some((name.to_string(), Some(Span::new(start, end))));
                    }
                    _ if opens => skip = 1,
                    _ => *key = Some((String::new(), None)),
                }
                *frame_end = (*frame_end).max(end);
                continue;
            }
            Some(Frame {
                kind: Kind::Map {
                    key: Some((name, key)),
                },
                path,
                ..
            }) => (child(path, name.clone()), *key),
            Some(Frame {
                kind: Kind::Seq { next },
                path,
                ..
            }) => {
                let path = child(path, next.to_string());
                *next += 1;
                (path, None)
            }
            None => (Vec::new(), None),
        };

        if opens {
            let kind = if matches!(event, Event::MappingStart(..)) {
                Kind::Map { key: None }
            } else {
                Kind::Seq { next: 0 }
            };
            stack.push(Frame {
                kind,
                path,
                key,
                start,
                end,
            });
        } else {
            let value = trimmed(text, Span::new(start, end));
            entries.insert(path, EntrySpan::new(key, value));
            value_done(&mut stack, value.end);
        }
    }
    entries
}

/// An open mapping or sequence.
struct Frame {
    kind: Kind,
    /// Path of the collection itself.
    path: Vec<String>,
    /// Its key span, when it is a mapping value.
    key: Option<Span>,
    start: usize,
    /// The end of the last node seen inside.
    end: usize,
}

enum Kind {
    /// `key` is the current entry's key name and span, once it has been read.
    Map {
        key: Option<(String, Option<Span>)>,
    },
    Seq {
        next: usize,
    },
}

/// A value ending at `end` finished in the innermost collection.
fn value_done(stack: &mut [Frame], end: usize) {
    if let Some(frame) = stack.last_mut() {
        frame.end = frame.end.max(end);
        if let Kind::Map { key } = &mut frame.kind {
            *key = None;
        }
    }
}

fn child(path: &[String], segment: String) -> Vec<String> {
    let mut path = path.to_vec();
    path.push(segment);
    path
}

fn offsets(span: &YamlSpan) -> (usize, usize) {
    let start = span.start.byte_offset().unwrap_or(0);
    let end = span.end.byte_offset().unwrap_or(start).max(start);
    (start, end)
}

/// serde-saphyr's message without its `error: line L column C:` prefix and source snippet.
fn message(error: &str) -> String {
    let first = error.lines().next().unwrap_or_default().trim();
    let first = first.strip_prefix("error: ").unwrap_or(first);
    let rest = first
        .strip_prefix("line ")
        .and_then(|rest| rest.split_once(": ").map(|(_, rest)| rest));
    rest.unwrap_or(first).to_owned()
}

/// Byte offset of a 1-based line and character column of `text`, clamped to its end.
fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let line_start: usize = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum();
    let rest = &text[line_start.min(text.len())..];
    line_start.min(text.len())
        + rest
            .char_indices()
            .nth(column.saturating_sub(1))
            .map_or(rest.len(), |(offset, _)| offset)
}

/// The character at `offset`, or an empty range at a line ending or the end of `text`.
fn char_at(text: &str, offset: usize) -> Span {
    let end = text[offset..]
        .chars()
        .next()
        .filter(|c| !matches!(c, '\n' | '\r'))
        .map_or(offset, |c| offset + c.len_utf8());
    Span::new(offset, end)
}

/// A 1-based line or column from serde-saphyr; saturates on 32-bit targets.
fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_drops_the_location_prefix() {
        assert_eq!(
            message("error: line 1 column 4: unclosed bracket '['\n --> x"),
            "unclosed bracket '['"
        );
        assert_eq!(message("something else"), "something else");
    }

    #[test]
    fn offsets_of_lines_and_columns() {
        let text = "é: 1\nab: [\n";
        assert_eq!(offset_of(text, 1, 1), 0);
        assert_eq!(offset_of(text, 1, 2), 2);
        assert_eq!(offset_of(text, 2, 5), 10);
        assert_eq!(offset_of(text, 9, 9), text.len());
        assert_eq!(char_at(text, 2), Span::new(2, 3));
        assert_eq!(char_at(text, 5), Span::empty(5));
        assert_eq!(char_at(text, text.len()), Span::empty(text.len()));
    }

    #[test]
    fn complex_keys_are_skipped() {
        let entries = index("? [a, b]\n: 1\nc: 2\n");
        assert!(entries.contains_key(&vec![String::from("c")]));
        assert!(entries.contains_key(&vec![String::new()]));
    }

    #[test]
    fn stops_at_syntax_errors() {
        let entries = index("a: 1\nb: [\n");
        assert!(entries.contains_key(&vec![String::from("a")]));
    }

    #[test]
    fn aliases_are_values() {
        let text = "a: &x 1\nb: *x\n";
        let entry = index(text)[&vec![String::from("b")]];
        assert_eq!(&text[entry.value.start..entry.value.end], "*x");
    }
}
