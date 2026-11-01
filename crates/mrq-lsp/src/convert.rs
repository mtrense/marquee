//! Conversions between LSP and mrq types: positions in the negotiated encoding, byte
//! offsets and ranges, and `file:` URIs and paths.
//!
//! LSP positions are lenient (a column past the end of a line means its end, a line past
//! the end of the text means the end), so these conversions clamp where
//! [`SourceFile`]'s are strict.

use std::path::{Path, PathBuf};

use mrq_core::document::normalize_path;
use mrq_core::source::{
    ByteRange, Position as SourcePosition, PositionEncoding, PositionError, SourceFile,
};
use tower_lsp_server::ls_types::{ClientCapabilities, Position, PositionEncodingKind, Range, Uri};

/// The position encoding to use with a client: UTF-8 if it offers it (no conversion
/// needed), else UTF-16, which every client supports.
pub fn negotiate_encoding(capabilities: &ClientCapabilities) -> PositionEncoding {
    let offered = capabilities
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_ref());
    if offered.is_some_and(|kinds| kinds.contains(&PositionEncodingKind::UTF8)) {
        PositionEncoding::Utf8
    } else {
        PositionEncoding::Utf16
    }
}

/// The LSP name of `encoding`.
pub fn encoding_kind(encoding: PositionEncoding) -> PositionEncodingKind {
    match encoding {
        PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
        PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
    }
}

/// The byte offset of `position` (columns in `encoding` units) in `source`, clamped: a
/// column past the end of its line is the line's end, a line past the last one the end
/// of the text, and a column inside a character (UTF-16: between the halves of a
/// surrogate pair) the start of that character.
pub fn offset(source: &SourceFile, position: Position, encoding: PositionEncoding) -> usize {
    let position = SourcePosition {
        line: position.line as usize,
        col: position.character as usize,
    };
    let text = source.text();
    match source.position_to_offset(position, encoding) {
        Ok(offset) => offset,
        Err(PositionError::NotCharBoundary { offset }) => text.floor_char_boundary(offset),
        Err(PositionError::ColumnOutOfBounds { line, .. }) => {
            // Before the line's `\n` (a `\r` counts as part of the line); the last line
            // ends with the text.
            let start = source.line_range(line).map_or(text.len(), |r| r.start);
            text[start..].find('\n').map_or(text.len(), |i| start + i)
        }
        Err(PositionError::LineOutOfBounds { .. } | PositionError::OffsetOutOfBounds { .. }) => {
            text.len()
        }
    }
}

/// The LSP position of byte `offset` in `source`, clamped to the end of the text and
/// down to a character boundary.
pub fn position(source: &SourceFile, offset: usize, encoding: PositionEncoding) -> Position {
    let offset = source.text().floor_char_boundary(offset);
    let position = source
        .offset_to_position(offset, encoding)
        .expect("a char boundary within the text");
    Position::new(saturate(position.line), saturate(position.col))
}

/// `value` as an LSP `uinteger`, saturating.
fn saturate(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The LSP range of `range` in `source`; see [`position`].
pub fn range(source: &SourceFile, range: ByteRange, encoding: PositionEncoding) -> Range {
    Range::new(
        position(source, range.start, encoding),
        position(source, range.end, encoding),
    )
}

/// The byte range of the LSP `range` in `source`; see [`offset`]. A reversed range is
/// put in order.
pub fn byte_range(source: &SourceFile, range: Range, encoding: PositionEncoding) -> ByteRange {
    let start = offset(source, range.start, encoding);
    let end = offset(source, range.end, encoding);
    ByteRange::new(start.min(end), start.max(end))
}

/// The lexically normalized path of a `file:` URI; `None` for other schemes.
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let scheme = uri.scheme().as_str();
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    Some(normalize_path(&uri.to_file_path()?))
}

/// The `file:` URI of the absolute `path`.
pub fn path_to_uri(path: &Path) -> Option<Uri> {
    Uri::from_file_path(path)
}

#[cfg(test)]
mod tests {
    use mrq_core::source::PositionEncoding::{Utf8, Utf16};
    use tower_lsp_server::ls_types::GeneralClientCapabilities;

    use super::*;

    fn source(text: &str) -> SourceFile {
        SourceFile::new(None, text)
    }

    #[test]
    fn utf8_is_chosen_only_when_offered() {
        let offering = |encodings: Option<Vec<PositionEncodingKind>>| ClientCapabilities {
            general: Some(GeneralClientCapabilities {
                position_encodings: encodings,
                ..GeneralClientCapabilities::default()
            }),
            ..ClientCapabilities::default()
        };
        assert_eq!(negotiate_encoding(&ClientCapabilities::default()), Utf16);
        assert_eq!(negotiate_encoding(&offering(None)), Utf16);
        let both = offering(Some(vec![
            PositionEncodingKind::UTF16,
            PositionEncodingKind::UTF8,
        ]));
        assert_eq!(negotiate_encoding(&both), Utf8);
        let utf32 = offering(Some(vec![PositionEncodingKind::UTF32]));
        assert_eq!(negotiate_encoding(&utf32), Utf16);
        assert_eq!(encoding_kind(Utf8), PositionEncodingKind::UTF8);
        assert_eq!(encoding_kind(Utf16), PositionEncodingKind::UTF16);
    }

    #[test]
    fn offsets_count_in_the_encoding() {
        let src = source("a😀b\nc");
        assert_eq!(offset(&src, Position::new(0, 3), Utf16), 5);
        assert_eq!(offset(&src, Position::new(0, 5), Utf8), 5);
        assert_eq!(offset(&src, Position::new(1, 1), Utf16), 8);
    }

    #[test]
    fn offsets_are_clamped() {
        let src = source("a😀b\r\nc");
        // Inside the surrogate pair, or inside the UTF-8 bytes: the emoji's start.
        assert_eq!(offset(&src, Position::new(0, 2), Utf16), 1);
        assert_eq!(offset(&src, Position::new(0, 3), Utf8), 1);
        // Past the line's end (which includes the `\r`): before the `\n`.
        assert_eq!(offset(&src, Position::new(0, 40), Utf16), 7);
        assert_eq!(offset(&src, Position::new(0, 40), Utf8), 7);
        // Past the last line: the end of the text.
        assert_eq!(offset(&src, Position::new(9, 0), Utf16), 9);
    }

    #[test]
    fn positions_are_clamped() {
        let src = source("a😀b\nc");
        assert_eq!(position(&src, 5, Utf16), Position::new(0, 3));
        assert_eq!(position(&src, 5, Utf8), Position::new(0, 5));
        assert_eq!(position(&src, 3, Utf16), Position::new(0, 1));
        assert_eq!(position(&src, 99, Utf16), Position::new(1, 1));
        let range = ByteRange::new(1, 8);
        assert_eq!(
            super::range(&src, range, Utf16),
            Range::new(Position::new(0, 1), Position::new(1, 1))
        );
        let reversed = Range::new(Position::new(1, 1), Position::new(0, 1));
        assert_eq!(byte_range(&src, reversed, Utf16), range);
    }

    #[test]
    fn file_uris_round_trip_through_paths() {
        let path = std::env::temp_dir().join("a b/ü.md");
        let uri = path_to_uri(&path).unwrap();
        assert!(uri.as_str().starts_with("file:///"), "{}", uri.as_str());
        assert!(uri.as_str().contains("a%20b"), "{}", uri.as_str());
        assert_eq!(uri_to_path(&uri).unwrap(), path);
        let dotted: Uri = "file:///x/./y/../z.md".parse().unwrap();
        assert_eq!(uri_to_path(&dotted).unwrap(), Path::new("/x/z.md"));
        let untitled: Uri = "untitled:Untitled-1".parse().unwrap();
        assert_eq!(uri_to_path(&untitled), None);
    }
}
