//! MD036 `no-emphasis-as-heading`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{NodeId, NodeKind};
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{ellipsify, message, raw_html};

/// Emphasis used instead of a heading.
///
/// A top-level paragraph (or one-line HTML block, which markdownlint reads as inline
/// content) that is only emphasized or strong plain text (inline HTML and whitespace
/// aside), one line without nested formatting, escapes or entities, is reported unless
/// the text ends with a character from `punctuation`. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md036;

/// Options of MD036 `no-emphasis-as-heading`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md036Options {
    /// Punctuation characters.
    pub punctuation: String,
}

impl Default for Md036Options {
    fn default() -> Self {
        Self {
            punctuation: ".,;:!?。，；：！？".to_owned(),
        }
    }
}

impl Rule for Md036 {
    type Options = Md036Options;

    const META: RuleMeta = RuleMeta {
        id: "MD036",
        aliases: &["no-emphasis-as-heading"],
        tags: &["headings", "emphasis"],
        description: "Emphasis used instead of a heading",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md036Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let text = doc.text();
        let mut diagnostics = Vec::new();
        for &block in ast.children(ast.root()) {
            let found = match ast.kind(block) {
                NodeKind::Paragraph => emphasis_text(doc, block),
                NodeKind::HtmlBlock(_) => html_block_emphasis(doc, block),
                _ => None,
            };
            let Some(range) = found else {
                continue;
            };
            let content = &text[range.start..range.end];
            if content
                .chars()
                .last()
                .is_some_and(|last| options.punctuation.contains(last))
            {
                continue;
            }
            let shown = ellipsify(content, false, false);
            diagnostics.push(Self::META.diagnostic(
                doc,
                range,
                message(&Self::META, None, Some(&shown)),
            ));
        }
        diagnostics
    }
}

/// The text of `paragraph` if it is only emphasis or strong around plain text (inline
/// HTML and whitespace aside).
fn emphasis_text(doc: &Document, paragraph: NodeId) -> Option<ByteRange> {
    let ast = doc.ast();
    let mut meaningful = ast.children(paragraph).iter().filter(|&&child| {
        !matches!(
            ast.kind(child),
            NodeKind::HtmlInline(_) | NodeKind::Comment(_)
        ) && !matches!(ast.kind(child), NodeKind::Text(text) if text.trim().is_empty())
    });
    let (Some(&emphasis), None) = (meaningful.next(), meaningful.next()) else {
        return None;
    };
    if !matches!(ast.kind(emphasis), NodeKind::Emphasis | NodeKind::Strong) {
        return None;
    }
    // One text child, literally as in the source (no escapes or entities).
    let [inner] = ast.children(emphasis) else {
        return None;
    };
    let NodeKind::Text(content) = ast.kind(*inner) else {
        return None;
    };
    let span = ast.span(*inner);
    (doc.text()[span.start..span.end] == *content).then(|| ByteRange::new(span.start, span.end))
}

/// markdownlint parses an HTML block's text again as inline content, so a one-line
/// block like `<!-- note --> **Text**` is a paragraph of HTML and emphasis: the text
/// between the HTML, parsed alone, must pass [`emphasis_text`].
fn html_block_emphasis(doc: &Document, block: NodeId) -> Option<ByteRange> {
    let span = doc.ast().span(block);
    let line = &doc.text()[span.start..span.end];
    let line = line.trim_end_matches(['\r', '\n']);
    if line.contains(['\r', '\n']) {
        return None;
    }
    // The one stretch of non-HTML text.
    let mut found = None;
    let mut pos = 0;
    let mut stretch = 0;
    while pos < line.len() {
        if let Some(len) = line[pos..]
            .starts_with('<')
            .then(|| raw_html(&line[pos..]))
            .flatten()
        {
            if !line[stretch..pos].trim().is_empty() {
                if found.is_some() {
                    return None;
                }
                found = Some(ByteRange::new(stretch, pos));
            }
            pos += len;
            stretch = pos;
        } else {
            pos += line[pos..].chars().next().map_or(1, char::len_utf8);
        }
    }
    if !line[stretch..].trim().is_empty() {
        if found.is_some() {
            return None;
        }
        found = Some(ByteRange::new(stretch, line.len()));
    }
    let found = found?;
    let segment = &line[found.start..found.end];
    let offset = span.start + found.start + (segment.len() - segment.trim_start().len());
    let inner = Document::parse(segment.trim().to_owned());
    let ast = inner.ast();
    let [paragraph] = ast.children(ast.root()) else {
        return None;
    };
    if !matches!(ast.kind(*paragraph), NodeKind::Paragraph) {
        return None;
    }
    let range = emphasis_text(&inner, *paragraph)?;
    Some(ByteRange::new(offset + range.start, offset + range.end))
}
