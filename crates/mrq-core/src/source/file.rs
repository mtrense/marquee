//! [`SourceFile`]: source text with a line index and offset/position conversions.

use std::path::{Path, PathBuf};

use super::edit::{ByteRange, EditError, TextEdit, apply_edits};

/// Unit in which [`Position::col`] is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PositionEncoding {
    /// UTF-8 bytes (mrq's native unit; matches byte offsets).
    Utf8,
    /// UTF-16 code units (the LSP default).
    Utf16,
}

/// A zero-based (line, column) position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    /// Zero-based line number.
    pub line: usize,
    /// Zero-based column, in the units of the [`PositionEncoding`] in use.
    pub col: usize,
}

/// Why an offset or position could not be converted.
///
/// Conversions are strict: nothing is clamped. Callers that want lenient behaviour (e.g.
/// an LSP server receiving a column past the end of a line) clamp before converting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PositionError {
    /// The byte offset lies past the end of the text.
    #[error("offset {offset} is out of bounds for text of length {len}")]
    OffsetOutOfBounds {
        /// The offending offset.
        offset: usize,
        /// Length of the text in bytes.
        len: usize,
    },
    /// The byte offset (given, or computed from a position) lies inside a multi-byte
    /// character; for UTF-16 positions it is the start of the character that the column
    /// splits (e.g. between the two halves of a surrogate pair).
    #[error("offset {offset} is not on a char boundary")]
    NotCharBoundary {
        /// The offending offset.
        offset: usize,
    },
    /// The line number is `>=` the number of lines.
    #[error("line {line} is out of bounds ({line_count} lines)")]
    LineOutOfBounds {
        /// The offending line.
        line: usize,
        /// Number of lines in the text.
        line_count: usize,
    },
    /// The column lies past the end of the line (before its `\n`).
    #[error("column {col} is out of bounds for line {line} of length {line_len}")]
    ColumnOutOfBounds {
        /// The line of the position.
        line: usize,
        /// The offending column.
        col: usize,
        /// Length of the line in the encoding's units, including a trailing `\r`.
        line_len: usize,
    },
}

/// A source text, where it came from, and an index of its line starts.
///
/// # Lines and columns
///
/// Lines are separated by `\n`; a text with `n` newlines has `n + 1` lines, so a trailing
/// newline yields an empty last line and the empty text has one line. Line `i` spans from
/// its start up to (not including) its `\n`, or the end of the text.
///
/// For column arithmetic a `\r` before the `\n` (CRLF) is an ordinary part of the line:
/// the offset between `\r` and `\n` is a valid position whose column is one past the
/// visible content. This keeps offset ↔ position conversion a bijection over all char
/// boundaries. [`SourceFile::line_range`] excludes the whole line ending instead.
///
/// Conversions are strict and return [`PositionError`] instead of clamping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    path: Option<PathBuf>,
    text: String,
    /// Byte offset of the start of each line; always non-empty, first entry is 0.
    line_starts: Vec<usize>,
}

impl SourceFile {
    /// Creates a source file and indexes its lines. `path` is `None` for stdin or
    /// in-memory text.
    pub fn new(path: Option<PathBuf>, text: impl Into<String>) -> Self {
        let text = text.into();
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            path,
            text,
            line_starts,
        }
    }

    /// The file's path, or `None` for stdin or in-memory text.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The full source text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of lines (at least 1).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte range of line `line`'s content, excluding its `\n` or `\r\n` terminator;
    /// `None` if the line does not exist.
    pub fn line_range(&self, line: usize) -> Option<ByteRange> {
        let start = *self.line_starts.get(line)?;
        let mut end = self.line_end(line);
        if self.text[start..end].ends_with('\r') {
            end -= 1;
        }
        Some(ByteRange::new(start, end))
    }

    /// Offset of line `line`'s `\n`, or the text length for the last line.
    fn line_end(&self, line: usize) -> usize {
        self.line_starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1)
    }

    /// Converts a byte offset to a (line, column) position, with the column counted in
    /// `encoding` units. The offset equal to the text length (end of file) is valid.
    ///
    /// # Errors
    ///
    /// [`PositionError::OffsetOutOfBounds`] past the end of the text,
    /// [`PositionError::NotCharBoundary`] inside a multi-byte character.
    pub fn offset_to_position(
        &self,
        offset: usize,
        encoding: PositionEncoding,
    ) -> Result<Position, PositionError> {
        if offset > self.text.len() {
            return Err(PositionError::OffsetOutOfBounds {
                offset,
                len: self.text.len(),
            });
        }
        if !self.text.is_char_boundary(offset) {
            return Err(PositionError::NotCharBoundary { offset });
        }
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let prefix = &self.text[self.line_starts[line]..offset];
        let col = match encoding {
            PositionEncoding::Utf8 => prefix.len(),
            PositionEncoding::Utf16 => prefix.encode_utf16().count(),
        };
        Ok(Position { line, col })
    }

    /// Converts a (line, column) position, with the column in `encoding` units, to a byte
    /// offset. The column may equal the line's length (the position just before its `\n`).
    ///
    /// # Errors
    ///
    /// [`PositionError::LineOutOfBounds`], [`PositionError::ColumnOutOfBounds`], or
    /// [`PositionError::NotCharBoundary`] if the column falls inside a character.
    pub fn position_to_offset(
        &self,
        position: Position,
        encoding: PositionEncoding,
    ) -> Result<usize, PositionError> {
        let Position { line, col } = position;
        let Some(&start) = self.line_starts.get(line) else {
            return Err(PositionError::LineOutOfBounds {
                line,
                line_count: self.line_count(),
            });
        };
        let end = self.line_end(line);
        let content = &self.text[start..end];
        match encoding {
            PositionEncoding::Utf8 => {
                if col > content.len() {
                    return Err(PositionError::ColumnOutOfBounds {
                        line,
                        col,
                        line_len: content.len(),
                    });
                }
                let offset = start + col;
                if self.text.is_char_boundary(offset) {
                    Ok(offset)
                } else {
                    Err(PositionError::NotCharBoundary { offset })
                }
            }
            PositionEncoding::Utf16 => {
                let mut units = 0;
                for (i, c) in content.char_indices() {
                    if units == col {
                        return Ok(start + i);
                    }
                    units += c.len_utf16();
                    if units > col {
                        return Err(PositionError::NotCharBoundary { offset: start + i });
                    }
                }
                if units == col {
                    Ok(end)
                } else {
                    Err(PositionError::ColumnOutOfBounds {
                        line,
                        col,
                        line_len: units,
                    })
                }
            }
        }
    }

    /// Applies `edits` (see [`apply_edits`]) and returns a new, re-indexed source file with
    /// the same path.
    ///
    /// # Errors
    ///
    /// See [`apply_edits`].
    pub fn apply_edits(&self, edits: &[TextEdit]) -> Result<Self, EditError> {
        Ok(Self::new(
            self.path.clone(),
            apply_edits(&self.text, edits)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::PositionEncoding::{Utf8, Utf16};
    use super::*;

    fn pos(line: usize, col: usize) -> Position {
        Position { line, col }
    }

    fn src(text: &str) -> SourceFile {
        SourceFile::new(None, text)
    }

    #[test]
    fn keeps_path_and_text() {
        let file = SourceFile::new(Some(PathBuf::from("a.md")), "x");
        assert_eq!(file.path(), Some(Path::new("a.md")));
        assert_eq!(file.text(), "x");
        assert_eq!(src("x").path(), None);
    }

    #[test]
    fn line_count() {
        assert_eq!(src("").line_count(), 1);
        assert_eq!(src("a").line_count(), 1);
        assert_eq!(src("a\n").line_count(), 2);
        assert_eq!(src("a\nb").line_count(), 2);
        assert_eq!(src("a\r\nb\r\n").line_count(), 3);
        assert_eq!(src("\n\n").line_count(), 3);
    }

    #[test]
    fn line_range_excludes_line_ending() {
        let file = src("ab\r\nc\n\nd");
        assert_eq!(file.line_range(0), Some(ByteRange::new(0, 2)));
        assert_eq!(file.line_range(1), Some(ByteRange::new(4, 5)));
        assert_eq!(file.line_range(2), Some(ByteRange::new(6, 6)));
        assert_eq!(file.line_range(3), Some(ByteRange::new(7, 8)));
        assert_eq!(file.line_range(4), None);
    }

    #[test]
    fn ascii_offsets_to_positions() {
        let file = src("ab\ncd");
        assert_eq!(file.offset_to_position(0, Utf8), Ok(pos(0, 0)));
        assert_eq!(file.offset_to_position(2, Utf8), Ok(pos(0, 2)));
        assert_eq!(file.offset_to_position(3, Utf8), Ok(pos(1, 0)));
        assert_eq!(file.offset_to_position(5, Utf8), Ok(pos(1, 2)));
        assert_eq!(file.offset_to_position(5, Utf16), Ok(pos(1, 2)));
    }

    #[test]
    fn end_of_file_after_trailing_newline_is_on_empty_last_line() {
        let file = src("a\n");
        assert_eq!(file.offset_to_position(2, Utf8), Ok(pos(1, 0)));
        assert_eq!(file.position_to_offset(pos(1, 0), Utf8), Ok(2));
    }

    #[test]
    fn empty_text() {
        let file = src("");
        assert_eq!(file.offset_to_position(0, Utf16), Ok(pos(0, 0)));
        assert_eq!(file.position_to_offset(pos(0, 0), Utf16), Ok(0));
    }

    #[test]
    fn offset_past_end_is_an_error() {
        let err = src("ab").offset_to_position(3, Utf8).unwrap_err();
        assert_eq!(err, PositionError::OffsetOutOfBounds { offset: 3, len: 2 });
    }

    #[test]
    fn offset_inside_char_is_an_error() {
        let err = src("é").offset_to_position(1, Utf16).unwrap_err();
        assert_eq!(err, PositionError::NotCharBoundary { offset: 1 });
    }

    #[test]
    fn multi_byte_columns() {
        // ä: 2 bytes / 1 unit, €: 3 bytes / 1 unit, 🦀: 4 bytes / 2 units.
        let file = src("x\nä€🦀z");
        let line = 2; // bytes before line 1
        assert_eq!(file.offset_to_position(line + 2, Utf8), Ok(pos(1, 2)));
        assert_eq!(file.offset_to_position(line + 2, Utf16), Ok(pos(1, 1)));
        assert_eq!(file.offset_to_position(line + 5, Utf16), Ok(pos(1, 2)));
        assert_eq!(file.offset_to_position(line + 9, Utf8), Ok(pos(1, 9)));
        assert_eq!(file.offset_to_position(line + 9, Utf16), Ok(pos(1, 4)));
        assert_eq!(file.position_to_offset(pos(1, 4), Utf16), Ok(line + 9));
        assert_eq!(file.position_to_offset(pos(1, 5), Utf16), Ok(line + 10));
        assert_eq!(file.position_to_offset(pos(1, 10), Utf8), Ok(line + 10));
    }

    #[test]
    fn utf16_column_inside_surrogate_pair_is_an_error() {
        let file = src("🦀");
        assert_eq!(
            file.position_to_offset(pos(0, 1), Utf16),
            Err(PositionError::NotCharBoundary { offset: 0 })
        );
    }

    #[test]
    fn utf8_column_inside_char_is_an_error() {
        let file = src("a\né");
        assert_eq!(
            file.position_to_offset(pos(1, 1), Utf8),
            Err(PositionError::NotCharBoundary { offset: 3 })
        );
    }

    #[test]
    fn crlf_columns_count_the_carriage_return() {
        let file = src("ab\r\ncd");
        assert_eq!(file.offset_to_position(2, Utf8), Ok(pos(0, 2)));
        // Between '\r' and '\n': the '\r' is the line's last column unit.
        assert_eq!(file.offset_to_position(3, Utf8), Ok(pos(0, 3)));
        assert_eq!(file.offset_to_position(3, Utf16), Ok(pos(0, 3)));
        assert_eq!(file.offset_to_position(4, Utf16), Ok(pos(1, 0)));
        assert_eq!(file.position_to_offset(pos(0, 3), Utf16), Ok(3));
        assert_eq!(file.position_to_offset(pos(1, 2), Utf8), Ok(6));
    }

    #[test]
    fn line_past_end_is_an_error() {
        assert_eq!(
            src("a\nb").position_to_offset(pos(2, 0), Utf8),
            Err(PositionError::LineOutOfBounds {
                line: 2,
                line_count: 2
            })
        );
    }

    #[test]
    fn column_past_line_end_is_an_error() {
        let file = src("ab\ncd");
        // Column 3 on line 0 would be past the '\n'.
        assert_eq!(
            file.position_to_offset(pos(0, 3), Utf8),
            Err(PositionError::ColumnOutOfBounds {
                line: 0,
                col: 3,
                line_len: 2
            })
        );
        assert_eq!(
            file.position_to_offset(pos(1, 3), Utf16),
            Err(PositionError::ColumnOutOfBounds {
                line: 1,
                col: 3,
                line_len: 2
            })
        );
    }

    #[test]
    fn error_messages() {
        assert_eq!(
            PositionError::OffsetOutOfBounds { offset: 3, len: 2 }.to_string(),
            "offset 3 is out of bounds for text of length 2"
        );
        assert_eq!(
            PositionError::LineOutOfBounds {
                line: 2,
                line_count: 2
            }
            .to_string(),
            "line 2 is out of bounds (2 lines)"
        );
    }

    #[test]
    fn apply_edits_keeps_path_and_reindexes_lines() {
        let file = SourceFile::new(Some(PathBuf::from("a.md")), "a\nb");
        let edited = file
            .apply_edits(&[TextEdit {
                range: ByteRange::new(1, 2),
                replacement: "\r\n\n".into(),
            }])
            .unwrap();
        assert_eq!(edited.path(), Some(Path::new("a.md")));
        assert_eq!(edited.text(), "a\r\n\nb");
        assert_eq!(edited.line_count(), 3);
        assert_eq!(edited.offset_to_position(4, Utf8), Ok(pos(2, 0)));
    }
}
