//! [`Ast::dump`]: an indented, line-per-node outline of a subtree for snapshot tests.

use std::fmt::{Debug, Write as _};

use super::{Ast, Event, NodeId, NodeKind};

impl Ast {
    /// Renders the subtree at `id` with one line per node, indented two spaces per level:
    /// the type name, the span, then the payload (if any) in `Debug` form without its
    /// struct name.
    ///
    /// ```text
    /// document 0..12
    ///   heading 0..5 { level: 1, setext: false, attrs: None }
    ///     text 2..4 "Hi"
    /// ```
    ///
    /// The format is for humans and snapshots, not a stable contract; the JSON projection
    /// is.
    pub fn dump(&self, id: NodeId) -> String {
        let mut out = String::new();
        let mut depth = 0usize;
        for event in self.traverse(id) {
            match event {
                Event::Enter(node) => {
                    let span = self.span(node);
                    let kind = self.kind(node);
                    let _ = write!(
                        out,
                        "{:indent$}{} {}..{}",
                        "",
                        kind.node_type(),
                        span.start,
                        span.end,
                        indent = depth * 2
                    );
                    if let Some(payload) = payload(kind) {
                        let debug = format!("{payload:?}");
                        out.push(' ');
                        out.push_str(strip_struct_name(&debug));
                    }
                    out.push('\n');
                    depth += 1;
                }
                Event::Leave(_) => depth -= 1,
            }
        }
        out
    }
}

/// The payload of `kind`, if it has one.
fn payload(kind: &NodeKind) -> Option<&dyn Debug> {
    Some(match kind {
        NodeKind::FrontMatter(p) => p,
        NodeKind::Heading(p) => p,
        NodeKind::List(p) => p,
        NodeKind::ListItem(p) => p,
        NodeKind::CodeBlock(p) => p,
        NodeKind::Table(p) => p,
        NodeKind::TableRow(p) => p,
        NodeKind::FootnoteDefinition(p) => p,
        NodeKind::LinkDefinition(p) => p,
        NodeKind::Admonition(p) => p,
        NodeKind::Callout(p) => p,
        NodeKind::AbbreviationDefinition(p) => p,
        NodeKind::IncludeDirective(p) | NodeKind::TocDirective(p) => p,
        NodeKind::Link(p) | NodeKind::Image(p) => p,
        NodeKind::FootnoteReference(p) => p,
        NodeKind::WikiLink(p) | NodeKind::Embed(p) => p,
        NodeKind::Tag(p) => p,
        NodeKind::Math(p) => p,
        NodeKind::Comment(p) => p,
        NodeKind::HtmlBlock(s)
        | NodeKind::Text(s)
        | NodeKind::Code(s)
        | NodeKind::HtmlInline(s) => s,
        NodeKind::Document
        | NodeKind::Paragraph
        | NodeKind::ThematicBreak
        | NodeKind::BlockQuote
        | NodeKind::TableCell
        | NodeKind::DefinitionList
        | NodeKind::DefinitionItem
        | NodeKind::DefinitionTerm
        | NodeKind::DefinitionDetails
        | NodeKind::DirectiveEnd
        | NodeKind::SoftBreak
        | NodeKind::LineBreak
        | NodeKind::Emphasis
        | NodeKind::Strong
        | NodeKind::Strikethrough
        | NodeKind::Highlight
        | NodeKind::Superscript
        | NodeKind::Subscript => return None,
    })
}

/// `Heading { level: 1 }` → `{ level: 1 }`; other `Debug` output is returned unchanged.
fn strip_struct_name(debug: &str) -> &str {
    match debug.split_once(' ') {
        Some((name, rest))
            if rest.starts_with('{') && name.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            rest
        }
        _ => debug,
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{Builder, Link};

    #[test]
    fn dump_shows_types_spans_and_payloads() {
        let ast = Builder::document(|b| {
            let h = b.heading(1, |b| {
                let t = b.text("Hi");
                b.set_span(t, 2..4);
            });
            b.set_span(h, 0..5);
            b.paragraph(|b| {
                b.link(Link::inline("u"), |b| {
                    let t = b.text("x");
                    b.set_span(t, 7..8);
                });
            });
            b.thematic_break();
        });
        let expected = "\
document 0..8
  heading 0..5 { level: 1, setext: false, attrs: None }
    text 2..4 \"Hi\"
  paragraph 7..8
    link 7..8 { url: \"u\", title: None, form: Inline }
      text 7..8 \"x\"
  thematic_break 0..0
";
        assert_eq!(ast.dump(ast.root()), expected);
    }
}
