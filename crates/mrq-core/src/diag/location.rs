//! Byte range to line/column resolution shared by the emitters.

use std::path::Path;

use serde::Serialize;

use crate::source::{ByteRange, PositionEncoding, SourceFile};

/// A 1-based line and 1-based column, the column counted in UTF-16 code units
/// (the SARIF default `columnKind` and the LSP default encoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct LineCol {
    pub line: usize,
    pub column: usize,
}

/// The source file whose path equals `path`, if it was provided.
pub(crate) fn find_source<'a>(sources: &'a [SourceFile], path: &Path) -> Option<&'a SourceFile> {
    sources.iter().find(|source| source.path() == Some(path))
}

/// Start and end of `range` in `source`; `None` without a source or for an invalid range.
pub(crate) fn resolve(source: Option<&SourceFile>, range: ByteRange) -> Option<(LineCol, LineCol)> {
    let source = source?;
    let at = |offset| {
        source
            .offset_to_position(offset, PositionEncoding::Utf16)
            .ok()
            .map(|p| LineCol {
                line: p.line + 1,
                column: p.col + 1,
            })
    };
    if range.start > range.end {
        return None;
    }
    Some((at(range.start)?, at(range.end)?))
}

/// `path` with `/` separators, for output formats that want portable paths.
pub(crate) fn display_path(path: &Path) -> String {
    crate::Slashed::slashed(path)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn lc(line: usize, column: usize) -> LineCol {
        LineCol { line, column }
    }

    #[test]
    fn finds_source_by_path() {
        let sources = [
            SourceFile::new(None, "x"),
            SourceFile::new(Some(PathBuf::from("a.md")), "a"),
        ];
        assert_eq!(
            find_source(&sources, Path::new("a.md")).map(SourceFile::text),
            Some("a")
        );
        assert!(find_source(&sources, Path::new("b.md")).is_none());
    }

    #[test]
    fn resolves_one_based_utf16_columns() {
        let file = SourceFile::new(None, "ab\nä🦀c");
        assert_eq!(
            resolve(Some(&file), ByteRange::new(1, 2)),
            Some((lc(1, 2), lc(1, 3)))
        );
        // "ä" is 2 bytes / 1 unit, "🦀" is 4 bytes / 2 units.
        assert_eq!(
            resolve(Some(&file), ByteRange::new(5, 9)),
            Some((lc(2, 2), lc(2, 4)))
        );
    }

    #[test]
    fn invalid_ranges_do_not_resolve() {
        let file = SourceFile::new(None, "é");
        assert_eq!(resolve(None, ByteRange::new(0, 0)), None);
        assert_eq!(resolve(Some(&file), ByteRange::new(0, 1)), None);
        assert_eq!(resolve(Some(&file), ByteRange::new(0, 9)), None);
        assert_eq!(resolve(Some(&file), ByteRange::new(2, 0)), None);
    }

    #[test]
    fn display_path_is_unchanged_on_unix_style_input() {
        assert_eq!(display_path(Path::new("docs/a b.md")), "docs/a b.md");
    }
}
