//! Source positions of keys in a YAML document, by key path.
//!
//! figment keeps values and their provider but not line numbers, so each config file
//! is also indexed with the span-aware YAML index shared with frontmatter
//! ([`crate::frontmatter::yaml::index`]) to map key paths to positions.

use std::collections::HashMap;

use crate::frontmatter::yaml;
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
    fn at(text: &str, range: ByteRange) -> Self {
        let before = &text[..range.start];
        let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
        Self {
            line: before.matches('\n').count() + 1,
            column: before[line_start..].chars().count() + 1,
            range,
        }
    }
}

/// Key path → position. Mapping keys are path segments as written; sequence items use
/// their index as a decimal segment.
#[derive(Debug, Clone, Default)]
pub(crate) struct KeyIndex {
    entries: HashMap<Vec<String>, Pos>,
}

impl KeyIndex {
    /// Indexes `text`. Positions are collected up to the first syntax error.
    pub(crate) fn build(text: &str) -> Self {
        let entries = yaml::index(text)
            .into_iter()
            .filter(|(path, _)| !path.is_empty())
            .map(|(path, entry)| (path, Pos::at(text, entry.key.unwrap_or(entry.value))))
            .collect();
        Self { entries }
    }

    /// Moves every entry to the path `map` gives it, dropping those it maps to `None`.
    pub(crate) fn remap(self, map: impl Fn(&[String]) -> Option<Vec<String>>) -> Self {
        let entries = self
            .entries
            .into_iter()
            .filter_map(|(path, pos)| Some((map(&path)?, pos)))
            .collect();
        Self { entries }
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
