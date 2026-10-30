//! Finding the frontmatter block: delimiter lines at the very start of the source.

use crate::ast::{FrontMatterFormat, Span};

/// The byte layout of a frontmatter block in its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Block {
    pub format: FrontMatterFormat,
    /// Opening delimiter through closing delimiter, without the final line ending.
    pub span: Span,
    /// Between the delimiter lines: after the opening line's ending, up to the closing line.
    pub content: Span,
    /// After the closing line's ending (or the end of the source).
    pub body_start: usize,
}

/// Finds a frontmatter block at the start of `src`, after an optional byte order mark.
///
/// The opening line is `---` (YAML) or `+++` (TOML), optionally followed by spaces or
/// tabs, and must end with `\n` or `\r\n`. The block ends at the next line made of the same
/// delimiter; without one there is no frontmatter.
pub(super) fn detect(src: &str) -> Option<Block> {
    let start = if src.starts_with('\u{feff}') { 3 } else { 0 };
    let (format, delimiter) = match src.get(start..start + 3)? {
        "---" => (FrontMatterFormat::Yaml, "---"),
        "+++" => (FrontMatterFormat::Toml, "+++"),
        _ => return None,
    };
    let (open_end, content_start) = line_bounds(src, start);
    if content_start == open_end || !is_delimiter(&src[start..open_end], delimiter) {
        return None;
    }
    let mut pos = content_start;
    while pos < src.len() {
        let (end, next) = line_bounds(src, pos);
        if is_delimiter(&src[pos..end], delimiter) {
            return Some(Block {
                format,
                span: Span::new(start, end),
                content: Span::new(content_start, pos),
                body_start: next,
            });
        }
        pos = next;
    }
    None
}

/// End of the line starting at `pos` (before `\n` or `\r\n`) and the start of the next
/// one; both are `src.len()` on a last line without line ending.
fn line_bounds(src: &str, pos: usize) -> (usize, usize) {
    match src[pos..].find('\n') {
        Some(offset) => {
            let newline = pos + offset;
            let end = if src[..newline].ends_with('\r') {
                newline - 1
            } else {
                newline
            };
            (end, newline + 1)
        }
        None => (src.len(), src.len()),
    }
}

fn is_delimiter(line: &str, delimiter: &str) -> bool {
    line.strip_prefix(delimiter)
        .is_some_and(|rest| rest.trim_matches([' ', '\t']).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_bounds_handle_lf_crlf_and_eof() {
        assert_eq!(line_bounds("ab\ncd", 0), (2, 3));
        assert_eq!(line_bounds("ab\r\ncd", 0), (2, 4));
        assert_eq!(line_bounds("ab\ncd", 3), (5, 5));
        assert_eq!(line_bounds("\n", 0), (0, 1));
    }

    #[test]
    fn delimiter_lines() {
        assert!(is_delimiter("---", "---"));
        assert!(is_delimiter("--- \t", "---"));
        assert!(!is_delimiter("--- x", "---"));
        assert!(!is_delimiter("----", "---"));
        assert!(!is_delimiter(" ---", "---"));
        assert!(!is_delimiter("+++", "---"));
    }
}
