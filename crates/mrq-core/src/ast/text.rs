//! Plain-text extraction: [`Ast::text`] / [`Node::text`].

use super::{Admonition, Ast, Callout, Event, Node, NodeId, NodeKind};

impl Ast {
    /// The plain text of the subtree rooted at `id`: what a reader sees, without markup.
    ///
    /// - Text, code spans, code blocks and math contribute their literal content; images
    ///   their alt text (children); a tag `#name`.
    /// - A soft break becomes a space, a hard break a newline.
    /// - A wikilink contributes its alias, else `target > anchor` (as Obsidian shows it),
    ///   else whichever of the two is present. An embed contributes its alias only.
    /// - Admonition and callout titles come first, on their own line.
    /// - Successive blocks are separated by a newline, table cells in a row by a tab.
    /// - Raw HTML, comments, frontmatter, link/abbreviation definitions, directives,
    ///   footnote references and thematic breaks contribute nothing.
    ///
    /// Heading text (for slugs and the outline) is `ast.text(heading)`.
    pub fn text(&self, id: NodeId) -> String {
        let mut out = TextOut::default();
        let mut events = self.traverse(id);
        while let Some(event) = events.next() {
            let Event::Enter(node) = event else { continue };
            let kind = self.kind(node);
            if node != id && kind.is_block() {
                let cell_after_cell =
                    matches!(kind, NodeKind::TableCell) && self.prev_sibling(node).is_some();
                out.separate(if cell_after_cell { '\t' } else { '\n' });
            }
            match kind {
                NodeKind::Text(text) | NodeKind::Code(text) => out.push(text),
                NodeKind::Math(math) => out.push(&math.literal),
                NodeKind::CodeBlock(code) => {
                    out.push(code.literal.strip_suffix('\n').unwrap_or(&code.literal));
                }
                NodeKind::SoftBreak => out.push(" "),
                NodeKind::LineBreak => out.push("\n"),
                NodeKind::Tag(tag) => {
                    out.push("#");
                    out.push(&tag.name);
                }
                NodeKind::WikiLink(link) => match (&link.alias, &link.anchor) {
                    (Some(alias), _) => out.push(alias),
                    (None, Some(anchor)) if link.target.is_empty() => out.push(anchor),
                    (None, Some(anchor)) => {
                        out.push(&link.target);
                        out.push(" > ");
                        out.push(anchor);
                    }
                    (None, None) => out.push(&link.target),
                },
                NodeKind::Embed(link) => out.push(link.alias.as_deref().unwrap_or_default()),
                NodeKind::Admonition(Admonition { title, .. })
                | NodeKind::Callout(Callout { title, .. }) => {
                    out.push(title.as_deref().unwrap_or_default());
                }
                NodeKind::HtmlBlock(_)
                | NodeKind::HtmlInline(_)
                | NodeKind::Comment(_)
                | NodeKind::FrontMatter(_)
                | NodeKind::LinkDefinition(_)
                | NodeKind::AbbreviationDefinition(_)
                | NodeKind::IncludeDirective(_)
                | NodeKind::TocDirective(_)
                | NodeKind::DirectiveEnd
                | NodeKind::FootnoteReference(_)
                | NodeKind::ThematicBreak => events.skip_children(),
                _ => {}
            }
        }
        out.text
    }
}

/// Output buffer that inserts a pending block separator lazily, so blocks without text
/// leave no stray separators.
#[derive(Default)]
struct TextOut {
    text: String,
    pending: Option<char>,
}

impl TextOut {
    /// Requests a separator before the next text. A newline wins over a tab.
    fn separate(&mut self, sep: char) {
        if self.pending != Some('\n') {
            self.pending = Some(sep);
        }
    }

    fn push(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if let Some(sep) = self.pending.take()
            && !self.text.is_empty()
            && !(sep == '\n' && self.text.ends_with('\n'))
        {
            self.text.push(sep);
        }
        self.text.push_str(s);
    }
}

impl Node<'_> {
    /// The plain text of this subtree; see [`Ast::text`].
    pub fn text(self) -> String {
        self.ast().text(self.id())
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{
        Admonition, Ast, Builder, Callout, Comment, CommentSyntax, FootnoteReference, Link, Math,
        NodeKind, Tag, WikiLink,
    };

    fn text_of(f: impl FnOnce(&mut Builder)) -> String {
        let ast: Ast = Builder::document(f);
        ast.text(ast.root())
    }

    fn wiki(target: &str, anchor: Option<&str>, alias: Option<&str>) -> WikiLink {
        WikiLink {
            target: target.into(),
            anchor: anchor.map(Into::into),
            alias: alias.map(Into::into),
        }
    }

    #[test]
    fn inline_markup_is_dropped() {
        let text = text_of(|b| {
            b.paragraph(|b| {
                b.text("a ");
                b.strong(|b| {
                    b.text("b");
                });
                b.text(" ");
                b.code("c()");
                b.soft_break();
                b.link(Link::inline("u"), |b| {
                    b.text("d");
                });
                b.line_break();
                b.image(Link::inline("i.png"), |b| {
                    b.text("alt");
                });
            });
        });
        assert_eq!(text, "a b c() d\nalt");
    }

    #[test]
    fn blocks_are_separated_by_newlines() {
        let text = text_of(|b| {
            b.heading(1, |b| {
                b.text("Title");
            });
            b.paragraph(|b| {
                b.text("one");
            });
            b.bullet_list('-', |b| {
                b.item(|b| {
                    b.paragraph(|b| {
                        b.text("two");
                    });
                });
                b.item(|b| {
                    b.paragraph(|b| {
                        b.text("three");
                    });
                });
            });
            b.thematic_break();
            b.code_block("sh", "ls\n");
            b.paragraph(|b| {
                b.text("four");
            });
        });
        assert_eq!(text, "Title\none\ntwo\nthree\nls\nfour");
    }

    #[test]
    fn invisible_nodes_contribute_nothing() {
        let text = text_of(|b| {
            b.html_block("<div>\n");
            b.paragraph(|b| {
                b.text("x");
                b.html_inline("<br>");
                b.leaf(NodeKind::FootnoteReference(FootnoteReference {
                    label: "1".into(),
                }));
                b.leaf(NodeKind::Comment(Comment {
                    syntax: CommentSyntax::Percent,
                    literal: "hidden".into(),
                }));
            });
            b.leaf(NodeKind::DirectiveEnd);
        });
        assert_eq!(text, "x");
    }

    #[test]
    fn extension_inlines() {
        let text = text_of(|b| {
            b.paragraph(|b| {
                b.leaf(NodeKind::WikiLink(wiki("Page", None, Some("alias"))));
                b.text(" ");
                b.leaf(NodeKind::WikiLink(wiki("Page", Some("Head"), None)));
                b.text(" ");
                b.leaf(NodeKind::WikiLink(wiki("", Some("Local"), None)));
                b.text(" ");
                b.leaf(NodeKind::Embed(wiki("img.png", None, None)));
                b.leaf(NodeKind::Tag(Tag {
                    name: "todo".into(),
                }));
                b.text(" ");
                b.leaf(NodeKind::Math(Math {
                    display: false,
                    literal: "x^2".into(),
                }));
            });
        });
        assert_eq!(text, "alias Page > Head Local #todo x^2");
    }

    #[test]
    fn titles_and_table_cells() {
        let text = text_of(|b| {
            b.node(
                NodeKind::Callout(Callout {
                    kind: "note".into(),
                    title: Some("Heads up".into()),
                    fold: None,
                }),
                |b| {
                    b.paragraph(|b| {
                        b.text("body");
                    });
                },
            );
            b.node(
                NodeKind::Admonition(Admonition {
                    kind: "tip".into(),
                    classes: vec![],
                    title: None,
                    fold: None,
                }),
                |b| {
                    b.paragraph(|b| {
                        b.text("tip");
                    });
                },
            );
            b.table(&[], |b| {
                b.row(true, |b| {
                    b.cell(|b| {
                        b.text("a");
                    });
                    b.cell(|b| {
                        b.text("b");
                    });
                });
                b.row(false, |b| {
                    b.cell(|b| {
                        b.text("1");
                    });
                    b.cell(|b| {
                        b.text("2");
                    });
                });
            });
        });
        assert_eq!(text, "Heads up\nbody\ntip\na\tb\n1\t2");
    }

    #[test]
    fn text_of_a_single_node() {
        let ast = Builder::document(|b| {
            b.heading(2, |b| {
                b.text("Install ");
                b.code("mrq");
            });
        });
        let heading = ast.node(ast.children(ast.root())[0]);
        assert_eq!(heading.text(), "Install mrq");
    }
}
