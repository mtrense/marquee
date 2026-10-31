//! Helpers shared by flavor passes that cut text nodes apart: escape detection,
//! container prefixes ([`repeated_prefix`], also used by the printer), aligning a text
//! node's source with its literal ([`Pieces`]), and splicing new inline nodes into a
//! parent's children ([`splice`]).

use crate::ast::{Ast, NodeId, NodeKind, Span};

/// Whether the byte at `at` is escaped by an odd number of backslashes before it.
pub(super) fn escaped(bytes: &[u8], at: usize) -> bool {
    bytes[..at]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

/// How many leading bytes of `line` repeat `prefix`: `>` and tabs as they are, a space
/// for anything else.
pub(crate) fn repeated_prefix(prefix: &str, line: &str) -> usize {
    prefix
        .bytes()
        .zip(line.bytes())
        .take_while(|&(p, l)| match p {
            b'>' | b'\t' => l == p,
            _ => l == b' ',
        })
        .count()
}

/// Cuts a text node into pieces with their own literals.
pub(super) struct Pieces {
    literal: String,
    /// Offset of the node's source.
    base: usize,
    /// For each byte offset of the node's source, the matching literal offset, if the
    /// source could be aligned with the literal and the offset is between two escapes,
    /// entities or characters.
    map: Option<Vec<Option<usize>>>,
}

impl Pieces {
    pub(super) fn new(source: &str, literal: String, base: usize) -> Self {
        Self {
            map: align(source, &literal),
            literal,
            base,
        }
    }

    /// A detached text node for `src[start..end]`. Its literal is cut from the node's
    /// literal; if that fails, it is the source with backslash escapes resolved.
    pub(super) fn create(&self, ast: &mut Ast, src: &str, start: usize, end: usize) -> NodeId {
        let cut = |offset: usize| self.map.as_ref().and_then(|map| map[offset - self.base]);
        let literal = match (cut(start), cut(end)) {
            (Some(a), Some(b)) => self.literal[a..b].to_owned(),
            _ => unescape(&src[start..end]),
        };
        ast.create(NodeKind::Text(literal), Span::new(start, end))
    }
}

/// Aligns a text node's `source` with its `literal`, which differs by backslash escapes
/// and entity references: for each byte offset of `source`, the literal offset if the
/// offset lies between two escapes, entities or characters. `None` if they don't align.
fn align(source: &str, literal: &str) -> Option<Vec<Option<usize>>> {
    if source == literal {
        return Some((0..=source.len()).map(Some).collect());
    }
    let mut map = vec![None; source.len() + 1];
    let (mut i, mut j) = (0, 0);
    while i < source.len() {
        map[i] = Some(j);
        let rest = &source[i..];
        let lit = &literal[j..];
        if let Some(c) = rest
            .strip_prefix('\\')
            .and_then(|r| r.chars().next())
            .filter(char::is_ascii_punctuation)
            && lit.starts_with(c)
        {
            i += 2;
            j += 1;
            continue;
        }
        if let Some((len, decoded)) = entity(rest)
            && !lit.starts_with(&rest[..len])
        {
            // A numeric reference must decode to the literal; a named one is taken to
            // decode to one character (true of all but a few rare names).
            let c = lit.chars().next()?;
            if decoded.is_some_and(|d| d != c) {
                return None;
            }
            i += len;
            j += c.len_utf8();
            continue;
        }
        let c = rest.chars().next()?;
        if !lit.starts_with(c) {
            return None;
        }
        i += c.len_utf8();
        j += c.len_utf8();
    }
    map[i] = Some(j);
    (j == literal.len()).then_some(map)
}

/// The length of the entity reference `&…;` that `text` starts with, and its character
/// if it is numeric.
fn entity(text: &str) -> Option<(usize, Option<char>)> {
    let body = text.strip_prefix('&')?;
    let end = body.find(';')?;
    let name = &body[..end];
    let len = end + 2;
    if let Some(number) = name.strip_prefix('#') {
        let (digits, radix, max) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16, 6),
            None => (number, 10, 7),
        };
        if digits.is_empty() || digits.len() > max {
            return None;
        }
        let value = u32::from_str_radix(digits, radix).ok()?;
        let c = char::from_u32(value)
            .filter(|&c| c != '\0')
            .unwrap_or(char::REPLACEMENT_CHARACTER);
        return Some((len, Some(c)));
    }
    let valid = name.len() <= 32
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then_some((len, None))
}

/// `text` with backslash escapes of ASCII punctuation resolved.
pub(super) fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(next) if c == '\\' && next.is_ascii_punctuation() => {}
            _ => out.push(c),
        }
    }
    out
}
/// Replaces ranges of `parent`'s children by new nodes: each `(span, node)` in `matches`
/// (in source order, without overlap, starting and ending within text children) takes
/// the place of everything it covers. Text children it touches are cut, keeping the
/// pieces before and after it as text nodes with their own spans and literals; other
/// children it covers are dropped.
pub(super) fn splice(ast: &mut Ast, src: &str, parent: NodeId, matches: &[(Span, NodeId)]) {
    let segments = ast.take_children(parent);
    let mut children = Vec::new();
    for child in segments {
        let s = ast.span(child);
        let NodeKind::Text(literal) = ast.kind(child) else {
            if !matches
                .iter()
                .any(|(m, _)| m.start <= s.start && s.end <= m.end)
            {
                children.push(child);
            }
            continue;
        };
        let touching: Vec<_> = matches
            .iter()
            .filter(|(m, _)| m.start < s.end && m.end > s.start)
            .collect();
        if touching.is_empty() {
            children.push(child);
            continue;
        }
        let pieces = Pieces::new(&src[s.start..s.end], literal.clone(), s.start);
        let mut cursor = s.start;
        for &&(m, node) in &touching {
            if m.start > cursor {
                children.push(pieces.create(ast, src, cursor, m.start));
            }
            if m.start >= s.start {
                children.push(node);
            }
            cursor = cursor.max(m.end);
        }
        if cursor < s.end {
            children.push(pieces.create(ast, src, cursor, s.end));
        }
    }
    for child in children {
        ast.append_child(parent, child)
            .expect("children were taken or freshly created");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_aligns_with_literal() {
        let map = align("a\\*b&amp;c&#35;d&bogus;", "a*b&c#d&bogus;").unwrap();
        assert_eq!(map[1], Some(1));
        assert_eq!(map[2], None);
        assert_eq!(map[3], Some(2));
        assert_eq!(map[4], Some(3));
        assert_eq!(map[9], Some(4));
        assert_eq!(map[15], Some(6));
        assert_eq!(map.last(), Some(&Some(14)));
        assert!(align("ab", "ac").is_none());
        assert!(align("&#35;", "x").is_none());
        assert_eq!(unescape("a\\*\\b"), "a*\\b");
    }

    #[test]
    fn entities_are_recognized() {
        assert_eq!(entity("&amp;x"), Some((5, None)));
        assert_eq!(entity("&#x41;"), Some((6, Some('A'))));
        assert_eq!(entity("&#0;"), Some((4, Some(char::REPLACEMENT_CHARACTER))));
        assert_eq!(entity("&#;"), None);
        assert_eq!(entity("&a b;"), None);
        assert_eq!(entity("&x"), None);
    }

    #[test]
    fn escapes_count_backslashes() {
        assert!(escaped(b"\\#", 1));
        assert!(!escaped(b"\\\\#", 2));
        assert!(!escaped(b"#", 0));
    }
}
