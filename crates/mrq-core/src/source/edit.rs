//! Byte ranges, text edits and their application.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// A half-open range `start..end` of byte offsets into a UTF-8 source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ByteRange {
    /// Offset of the first byte in the range.
    pub start: usize,
    /// Offset one past the last byte in the range.
    pub end: usize,
}

impl From<Range<usize>> for ByteRange {
    fn from(range: Range<usize>) -> Self {
        Self::new(range.start, range.end)
    }
}

impl From<ByteRange> for Range<usize> {
    fn from(range: ByteRange) -> Self {
        range.start..range.end
    }
}

impl ByteRange {
    /// Creates the range `start..end`.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Creates the empty range `offset..offset`, e.g. an insertion point.
    pub const fn empty(offset: usize) -> Self {
        Self::new(offset, offset)
    }

    /// Number of bytes covered (zero if `start >= end`).
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the range covers no bytes.
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// A replacement of the bytes in `range` by `replacement`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextEdit {
    /// The bytes to replace; empty for a pure insertion.
    pub range: ByteRange,
    /// The new text; empty for a pure deletion.
    pub replacement: String,
}

/// Why [`apply_edits`] rejected a set of edits.
///
/// Edits are identified by their index in the slice passed to [`apply_edits`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// Edit `index` has `start > end`.
    #[error("edit #{index}: invalid range {}..{}", range.start, range.end)]
    InvalidRange {
        /// Index of the offending edit.
        index: usize,
        /// Its range.
        range: ByteRange,
    },
    /// Edit `index` ends past the end of the text.
    #[error("edit #{index}: range {}..{} is out of bounds for text of length {len}", range.start, range.end)]
    OutOfBounds {
        /// Index of the offending edit.
        index: usize,
        /// Its range.
        range: ByteRange,
        /// Length of the text in bytes.
        len: usize,
    },
    /// Edit `index` starts or ends inside a multi-byte UTF-8 character.
    #[error("edit #{index}: offset {offset} is not on a char boundary")]
    NotCharBoundary {
        /// Index of the offending edit.
        index: usize,
        /// The offending offset.
        offset: usize,
    },
    /// Two edits touch overlapping bytes, or an insertion lies strictly inside a replaced range.
    ///
    /// `first` is the edit that starts earlier in the text (ties broken by input order).
    #[error("edit #{first} ({}..{}) overlaps edit #{second} ({}..{})", first_range.start, first_range.end, second_range.start, second_range.end)]
    Overlap {
        /// Index of the edit that comes first in the text.
        first: usize,
        /// Its range.
        first_range: ByteRange,
        /// Index of the edit it conflicts with.
        second: usize,
        /// Its range.
        second_range: ByteRange,
    },
}

/// Applies `edits` to `text` and returns the edited text.
///
/// All ranges refer to the *original* `text`; edits may be given in any order. Bytes not
/// covered by any edit are copied through unchanged.
///
/// # Ordering and conflicts
///
/// Edits are stably sorted by `(start, end)` and must not overlap:
///
/// - Adjacent edits (`a..b` and `b..c`) are fine.
/// - Several insertions (empty ranges) at the same offset are allowed and are applied in
///   their input order.
/// - An insertion at the start of a replaced range is placed before the replacement, one at
///   its end after it. An insertion strictly inside a replaced range is a conflict.
/// - Any two edits sharing at least one byte conflict, including two identical ranges.
///
/// # Errors
///
/// Returns an [`EditError`] naming the offending edit(s) if a range is inverted, out of
/// bounds, not on a UTF-8 char boundary, or conflicts with another edit. Nothing is applied
/// in that case.
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    for (index, edit) in edits.iter().enumerate() {
        let range = edit.range;
        if range.start > range.end {
            return Err(EditError::InvalidRange { index, range });
        }
        if range.end > text.len() {
            return Err(EditError::OutOfBounds {
                index,
                range,
                len: text.len(),
            });
        }
        for offset in [range.start, range.end] {
            if !text.is_char_boundary(offset) {
                return Err(EditError::NotCharBoundary { index, offset });
            }
        }
    }

    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|&i| (edits[i].range.start, edits[i].range.end));

    for pair in order.windows(2) {
        let (a, b) = (edits[pair[0]].range, edits[pair[1]].range);
        if a.end > b.start {
            return Err(EditError::Overlap {
                first: pair[0],
                first_range: a,
                second: pair[1],
                second_range: b,
            });
        }
    }

    let removed: usize = edits.iter().map(|e| e.range.len()).sum();
    let added: usize = edits.iter().map(|e| e.replacement.len()).sum();
    let mut out = String::with_capacity(text.len() - removed + added);
    let mut cursor = 0;
    for &i in &order {
        let edit = &edits[i];
        out.push_str(&text[cursor..edit.range.start]);
        out.push_str(&edit.replacement);
        cursor = edit.range.end;
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(start: usize, end: usize, replacement: &str) -> TextEdit {
        TextEdit {
            range: ByteRange::new(start, end),
            replacement: replacement.to_owned(),
        }
    }

    #[test]
    fn no_edits_returns_text_unchanged() {
        assert_eq!(apply_edits("hello", &[]).unwrap(), "hello");
    }

    #[test]
    fn replaces_insert_and_delete() {
        let out = apply_edits(
            "hello world",
            &[edit(0, 5, "goodbye"), edit(5, 5, ","), edit(6, 11, "")],
        );
        assert_eq!(out.unwrap(), "goodbye, ");
    }

    #[test]
    fn edits_are_sorted_before_application() {
        let out = apply_edits("abc", &[edit(2, 3, "C"), edit(0, 1, "A")]);
        assert_eq!(out.unwrap(), "AbC");
    }

    #[test]
    fn adjacent_edits_are_allowed() {
        let out = apply_edits("abcd", &[edit(2, 4, "Y"), edit(0, 2, "X")]);
        assert_eq!(out.unwrap(), "XY");
    }

    #[test]
    fn inserts_at_same_offset_keep_input_order() {
        let out = apply_edits("ac", &[edit(1, 1, "1"), edit(1, 1, "2"), edit(1, 1, "3")]);
        assert_eq!(out.unwrap(), "a123c");
    }

    #[test]
    fn insert_at_start_of_replacement_goes_first() {
        let out = apply_edits("abc", &[edit(1, 2, "B"), edit(1, 1, "+")]);
        assert_eq!(out.unwrap(), "a+Bc");
    }

    #[test]
    fn insert_at_end_of_replacement_goes_after() {
        let out = apply_edits("abc", &[edit(2, 2, "+"), edit(1, 2, "B")]);
        assert_eq!(out.unwrap(), "aB+c");
    }

    #[test]
    fn overlapping_edits_are_rejected_naming_both() {
        let err = apply_edits(
            "abcdef",
            &[edit(0, 1, ""), edit(3, 5, "x"), edit(2, 4, "y")],
        )
        .unwrap_err();
        assert_eq!(
            err,
            EditError::Overlap {
                first: 2,
                first_range: ByteRange::new(2, 4),
                second: 1,
                second_range: ByteRange::new(3, 5),
            }
        );
        assert_eq!(err.to_string(), "edit #2 (2..4) overlaps edit #1 (3..5)");
    }

    #[test]
    fn identical_ranges_conflict() {
        let err = apply_edits("abc", &[edit(0, 2, "x"), edit(0, 2, "y")]).unwrap_err();
        assert!(matches!(
            err,
            EditError::Overlap {
                first: 0,
                second: 1,
                ..
            }
        ));
    }

    #[test]
    fn insert_strictly_inside_replacement_conflicts() {
        let err = apply_edits("abc", &[edit(2, 2, "+"), edit(0, 3, "x")]).unwrap_err();
        assert!(matches!(
            err,
            EditError::Overlap {
                first: 1,
                second: 0,
                ..
            }
        ));
    }

    #[test]
    fn out_of_bounds_range_is_rejected() {
        let err = apply_edits("abc", &[edit(1, 4, "")]).unwrap_err();
        assert_eq!(
            err,
            EditError::OutOfBounds {
                index: 0,
                range: ByteRange::new(1, 4),
                len: 3
            }
        );
    }

    #[test]
    fn inverted_range_is_rejected() {
        let err = apply_edits("abc", &[edit(2, 1, "")]).unwrap_err();
        assert_eq!(
            err,
            EditError::InvalidRange {
                index: 0,
                range: ByteRange::new(2, 1)
            }
        );
    }

    #[test]
    fn non_char_boundary_is_rejected() {
        // 'é' is two bytes: 0xC3 0xA9.
        let err = apply_edits("é", &[edit(1, 2, "")]).unwrap_err();
        assert_eq!(
            err,
            EditError::NotCharBoundary {
                index: 0,
                offset: 1
            }
        );
    }

    #[test]
    fn multi_byte_replacement() {
        let out = apply_edits("a€b", &[edit(1, 4, "ü🦀")]);
        assert_eq!(out.unwrap(), "aü🦀b");
    }

    #[test]
    fn byte_range_helpers() {
        assert_eq!(ByteRange::empty(3), ByteRange::new(3, 3));
        assert!(ByteRange::empty(3).is_empty());
        assert_eq!(ByteRange::new(2, 5).len(), 3);
        assert_eq!(ByteRange::new(5, 2).len(), 0);
        assert_eq!(std::ops::Range::from(ByteRange::new(2, 5)), 2..5);
        assert_eq!(ByteRange::from(2..5), ByteRange::new(2, 5));
    }
}
