//! Link reference definitions recovered from the source: [`reference_definitions`].

use crate::Document;
use crate::ast::{NodeKind, Span};

/// A link reference definition `[label]: destination "title"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceDefinition {
    /// The label as written, without brackets (not case-folded).
    pub label: String,
    /// The destination as written, without `<…>` (not unescaped).
    pub url: String,
    /// From the opening `[` to the end of the destination; a title is not included.
    pub span: Span,
}

/// Every link reference definition of `document`, in document order.
///
/// The parser resolves reference links but drops the definitions, so they are recovered
/// from the source: a line, after any container prefix (`>`, list markers and
/// indentation), that starts with `[label]:` outside code blocks, HTML blocks, headings
/// and the like, and outside paragraph text (a definition cannot interrupt a paragraph).
/// The destination may be on the next line; labels may not span lines. With footnotes
/// enabled, `[^label]:` is a footnote definition instead.
pub fn reference_definitions(document: &Document) -> Vec<ReferenceDefinition> {
    let text = document.text();
    let footnotes = document.flavor().parse_options().footnotes;
    let opaque = opaque_ranges(document);
    let mut next_opaque = 0;
    let mut defs = Vec::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let at = line_start + container_prefix_len(line);
        line_start += line.len();
        while opaque.get(next_opaque).is_some_and(|range| range.end <= at) {
            next_opaque += 1;
        }
        if opaque
            .get(next_opaque)
            .is_some_and(|range| range.start <= at)
        {
            continue;
        }
        if let Some(def) = parse_definition(text, at)
            && !(footnotes && def.label.starts_with('^'))
        {
            defs.push(def);
        }
    }
    defs
}

/// The sorted, merged ranges where a definition cannot start: every block that is not a
/// container (code, HTML, headings, tables, …), and the text of every paragraph.
fn opaque_ranges(document: &Document) -> Vec<Span> {
    let ast = document.ast();
    let mut ranges: Vec<Span> = ast
        .descendants(ast.root())
        .filter(|&node| ast.parent(node).is_some_and(|p| is_container(ast.kind(p))))
        .filter_map(|node| match ast.kind(node) {
            kind if is_container(kind) => None,
            NodeKind::Paragraph => ast
                .children(node)
                .first()
                .map(|&first| Span::new(ast.span(first).start, ast.span(node).end)),
            _ => Some(ast.span(node)),
        })
        .collect();
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Span> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// Whether nodes of `kind` hold blocks, so a definition may sit directly inside.
const fn is_container(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Document
            | NodeKind::BlockQuote
            | NodeKind::List(_)
            | NodeKind::ListItem(_)
            | NodeKind::FootnoteDefinition(_)
            | NodeKind::DefinitionList
            | NodeKind::DefinitionItem
            | NodeKind::DefinitionTerm
            | NodeKind::DefinitionDetails
            | NodeKind::Admonition(_)
            | NodeKind::Callout(_)
    )
}

/// The length of `line`'s container prefix: blanks, `>` markers and list markers.
fn container_prefix_len(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut i = 0;
    loop {
        while matches!(bytes.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        let blank_after = |at: usize| matches!(bytes.get(at), Some(b' ' | b'\t'));
        match bytes.get(i) {
            Some(b'>') => i += 1,
            Some(b'-' | b'*' | b'+') if blank_after(i + 1) => i += 1,
            Some(b'0'..=b'9') => {
                let digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
                let marker = i + digits;
                if digits <= 9
                    && matches!(bytes.get(marker), Some(b'.' | b')'))
                    && blank_after(marker + 1)
                {
                    i = marker + 1;
                } else {
                    return i;
                }
            }
            _ => return i,
        }
    }
}

/// The definition starting at byte `at` of `text`, if there is one.
fn parse_definition(text: &str, at: usize) -> Option<ReferenceDefinition> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut i = at + 1;
    loop {
        match bytes.get(i)? {
            b'\\' => i += 2,
            b'[' | b'\n' | b'\r' => return None,
            b']' => break,
            _ => i += 1,
        }
    }
    let label = &text[at + 1..i];
    if label.trim().is_empty() || bytes.get(i + 1) != Some(&b':') {
        return None;
    }
    let blank = |b: Option<&u8>| matches!(b, Some(b' ' | b'\t'));
    let mut i = i + 2;
    while blank(bytes.get(i)) {
        i += 1;
    }
    let same_line = !matches!(bytes.get(i), Some(b'\n' | b'\r') | None);
    if !same_line {
        i += if bytes.get(i) == Some(&b'\r') { 2 } else { 1 };
        let next_line = text.get(i..)?.split('\n').next().unwrap_or_default();
        i += container_prefix_len(next_line);
    }
    let (url, end) = if bytes.get(i) == Some(&b'<') {
        // `<…>`, possibly empty, on one line.
        let close = i + text[i..].find(['>', '\n'])?;
        if bytes[close] != b'>' {
            return None;
        }
        (&text[i + 1..close], close + 1)
    } else {
        let len = text[i..]
            .find([' ', '\t', '\n', '\r'])
            .unwrap_or(text.len() - i);
        if len == 0 {
            return None;
        }
        (&text[i..i + len], i + len)
    };
    // On the definition's own line, only blanks or a title may follow.
    if same_line {
        let mut j = end;
        while blank(bytes.get(j)) {
            j += 1;
        }
        let ends_line = matches!(bytes.get(j), Some(b'\n' | b'\r') | None);
        let titled = j > end && matches!(bytes.get(j), Some(b'"' | b'\'' | b'('));
        if !ends_line && !titled {
            return None;
        }
    }
    Some(ReferenceDefinition {
        label: label.to_owned(),
        url: url.to_owned(),
        span: Span::new(at, end),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (label, url, source of the span) of every definition.
    fn defs(text: &str) -> Vec<(String, String, String)> {
        let doc = Document::parse(text);
        reference_definitions(&doc)
            .into_iter()
            .map(|def| {
                let source = doc.text()[def.span.start..def.span.end].to_owned();
                (def.label, def.url, source)
            })
            .collect()
    }

    fn def(label: &str, url: &str, source: &str) -> (String, String, String) {
        (label.into(), url.into(), source.into())
    }

    #[test]
    fn top_level_definitions() {
        assert_eq!(
            defs("Text [a].\n\n[a]: a.md\n[B c]: <b c.md> \"Title\"\n"),
            [
                def("a", "a.md", "[a]: a.md"),
                def("B c", "b c.md", "[B c]: <b c.md>"),
            ]
        );
    }

    #[test]
    fn definitions_before_paragraph_text_and_in_containers() {
        assert_eq!(
            defs("> [q]: q.md\n> text\n\n- [l]: l.md\n  [m]: m.md 't'\n\n[d]: d.md\ntext\n"),
            [
                def("q", "q.md", "[q]: q.md"),
                def("l", "l.md", "[l]: l.md"),
                def("m", "m.md", "[m]: m.md"),
                def("d", "d.md", "[d]: d.md"),
            ]
        );
    }

    #[test]
    fn destination_on_the_next_line() {
        assert_eq!(
            defs("[multi]:\n  multi.md\n  \"title\"\n"),
            [def("multi", "multi.md", "[multi]:\n  multi.md")]
        );
    }

    #[test]
    fn look_alikes_are_not_definitions() {
        let text = "```\n[code]: x\n```\n\n    [indented]: x\n\ntext\n[lazy]: x\n\n\
                    # [h]: x\n\n<div>\n[html]: x\n</div>\n\n[^note]: footnote\n\n[a] [b]\n";
        assert_eq!(defs(text), []);
    }

    #[test]
    fn footnote_like_labels_are_definitions_without_footnotes() {
        let doc = crate::document::DocumentOptions::new()
            .flavor("commonmark")
            .parse("[^x]: x.md\n")
            .unwrap();
        let labels: Vec<_> = reference_definitions(&doc)
            .into_iter()
            .map(|def| def.label)
            .collect();
        assert_eq!(labels, ["^x"]);
    }
}
