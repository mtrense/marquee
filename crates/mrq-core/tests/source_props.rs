//! Property tests for `mrq_core::source`.

use mrq_core::source::{
    ByteRange, EditError, Position, PositionEncoding, SourceFile, TextEdit, apply_edits,
};
use proptest::prelude::*;
use proptest::sample::Index;

/// Arbitrary Unicode text with a bias towards line endings (LF, CRLF, lone CR) and
/// multi-byte characters.
fn text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => any::<char>().prop_map(String::from),
        1 => Just("\n".to_owned()),
        1 => Just("\r\n".to_owned()),
        1 => Just("\r".to_owned()),
        1 => Just("é".to_owned()),
        1 => Just("€".to_owned()),
        1 => Just("🦀".to_owned()),
        1 => "[a-z ]{0,4}",
    ];
    prop::collection::vec(piece, 0..40).prop_map(|pieces| pieces.concat())
}

fn boundaries(text: &str) -> Vec<usize> {
    (0..=text.len())
        .filter(|&i| text.is_char_boundary(i))
        .collect()
}

/// Raw edit material: two boundary picks, a replacement and a sort key used to shuffle.
fn raw_edits() -> impl Strategy<Value = Vec<(Index, Index, String, u32)>> {
    prop::collection::vec(
        (
            any::<Index>(),
            any::<Index>(),
            "(\\PC|\r\n|\n){0,3}",
            any::<u32>(),
        ),
        0..8,
    )
}

fn to_edits(text: &str, raw: &[(Index, Index, String, u32)]) -> Vec<TextEdit> {
    let bounds = boundaries(text);
    raw.iter()
        .map(|(a, b, replacement, _)| {
            let (a, b) = (*a.get(&bounds), *b.get(&bounds));
            TextEdit {
                range: ByteRange::new(a.min(b), a.max(b)),
                replacement: replacement.clone(),
            }
        })
        .collect()
}

/// Reference definition of a conflict between two valid edits.
fn conflicts(a: ByteRange, b: ByteRange) -> bool {
    let share_bytes = a.start < b.end && b.start < a.end;
    let insert_inside =
        |x: ByteRange, y: ByteRange| x.is_empty() && y.start < x.start && x.start < y.end;
    share_bytes || insert_inside(a, b) || insert_inside(b, a)
}

/// Keeps a conflict-free subset of `edits` (greedy over start order) and shuffles it by
/// the raw sort keys.
fn non_overlapping(edits: Vec<TextEdit>, keys: &[u32]) -> Vec<TextEdit> {
    let mut sorted: Vec<(TextEdit, u32)> = edits.into_iter().zip(keys.iter().copied()).collect();
    sorted.sort_by_key(|(e, _)| (e.range.start, e.range.end));
    let mut kept: Vec<(TextEdit, u32)> = Vec::new();
    for (edit, key) in sorted {
        if kept.iter().all(|(k, _)| !conflicts(k.range, edit.range)) {
            kept.push((edit, key));
        }
    }
    kept.sort_by_key(|(_, key)| *key);
    kept.into_iter().map(|(e, _)| e).collect()
}

proptest! {
    #[test]
    fn offset_position_round_trip(text in text()) {
        let file = SourceFile::new(None, text.clone());
        for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            let mut previous: Option<Position> = None;
            for offset in boundaries(&text) {
                let pos = file.offset_to_position(offset, encoding).unwrap();
                prop_assert_eq!(file.position_to_offset(pos, encoding), Ok(offset));
                prop_assert!(pos.line < file.line_count());
                prop_assert_eq!(pos.line, text[..offset].matches('\n').count());
                if let Some(prev) = previous {
                    prop_assert!(prev < pos, "positions must increase with offsets");
                }
                previous = Some(pos);
            }
            prop_assert!(file.offset_to_position(text.len() + 1, encoding).is_err());
        }
    }

    #[test]
    fn utf16_columns_match_encode_utf16(text in text()) {
        let file = SourceFile::new(None, text.clone());
        for offset in boundaries(&text) {
            let pos = file.offset_to_position(offset, PositionEncoding::Utf16).unwrap();
            let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
            prop_assert_eq!(pos.col, text[line_start..offset].encode_utf16().count());
        }
    }

    #[test]
    fn line_ranges_cover_lines(text in text()) {
        let file = SourceFile::new(None, text.clone());
        let lines: Vec<&str> = text.split('\n').collect();
        prop_assert_eq!(file.line_count(), lines.len());
        for (i, line) in lines.iter().enumerate() {
            let range = file.line_range(i).unwrap();
            prop_assert_eq!(&text[range.start..range.end], line.strip_suffix('\r').unwrap_or(line));
        }
        prop_assert_eq!(file.line_range(lines.len()), None);
    }

    #[test]
    fn non_overlapping_edits_apply(text in text(), raw in raw_edits()) {
        let keys: Vec<u32> = raw.iter().map(|r| r.3).collect();
        let edits = non_overlapping(to_edits(&text, &raw), &keys);
        let out = apply_edits(&text, &edits).unwrap();

        // Length arithmetic.
        let removed: usize = edits.iter().map(|e| e.range.len()).sum();
        let added: usize = edits.iter().map(|e| e.replacement.len()).sum();
        prop_assert_eq!(out.len(), text.len() - removed + added);

        // Untouched bytes and replacements appear in order at the expected output offsets.
        // A stable sort by (start, end) keeps same-offset inserts in input order.
        let mut sorted = edits.clone();
        sorted.sort_by_key(|e| (e.range.start, e.range.end));
        let (mut cursor, mut at) = (0, 0);
        for edit in &sorted {
            let gap = &text[cursor..edit.range.start];
            prop_assert_eq!(&out[at..at + gap.len()], gap);
            at += gap.len();
            prop_assert_eq!(&out[at..at + edit.replacement.len()], edit.replacement.as_str());
            at += edit.replacement.len();
            cursor = edit.range.end;
        }
        prop_assert_eq!(&out[at..], &text[cursor..]);
    }

    #[test]
    fn conflicts_are_detected_exactly(text in text(), raw in raw_edits()) {
        let edits = to_edits(&text, &raw);
        let any_conflict = edits.iter().enumerate().any(|(i, a)| {
            edits[i + 1..].iter().any(|b| conflicts(a.range, b.range))
        });
        match apply_edits(&text, &edits) {
            Ok(_) => prop_assert!(!any_conflict),
            Err(EditError::Overlap { first, first_range, second, second_range }) => {
                prop_assert!(any_conflict);
                prop_assert_eq!(edits[first].range, first_range);
                prop_assert_eq!(edits[second].range, second_range);
                prop_assert!(conflicts(first_range, second_range));
            }
            Err(other) => prop_assert!(false, "unexpected error {other}"),
        }
    }

    #[test]
    fn edited_source_file_is_reindexed(text in text(), raw in raw_edits()) {
        let keys: Vec<u32> = raw.iter().map(|r| r.3).collect();
        let edits = non_overlapping(to_edits(&text, &raw), &keys);
        let edited = SourceFile::new(None, text.clone()).apply_edits(&edits).unwrap();
        let fresh = SourceFile::new(None, apply_edits(&text, &edits).unwrap());
        prop_assert_eq!(edited, fresh);
    }

    #[test]
    fn out_of_bounds_edits_are_rejected(text in text(), extra in 1usize..4, replacement in "\\PC{0,2}") {
        let edit = TextEdit { range: ByteRange::new(0, text.len() + extra), replacement };
        let is_out_of_bounds = matches!(apply_edits(&text, &[edit]), Err(EditError::OutOfBounds { index: 0, .. }));
        prop_assert!(is_out_of_bounds);
    }
}
