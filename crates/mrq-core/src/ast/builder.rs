//! [`Builder`]: concise construction of trees, mainly for tests.

use super::{
    Alignment, Ast, CodeBlock, Fence, Heading, Link, List, ListItem, ListKind, NodeId, NodeKind,
    OrderedDelimiter, Span, Table, TableRow, TaskState,
};

/// Builds an [`Ast`] with nested closures that mirror the tree's shape.
///
/// Every method adds a node as the last child of the node whose closure is running and
/// returns its id. Spans are optional: set them with [`set_span`](Self::set_span). A node
/// without an explicit span gets the smallest range covering its children's spans when
/// the tree is finished, or the empty range at 0 if no child has one.
///
/// ```
/// use mrq_core::ast::{Builder, NodeType};
///
/// let src = "# Hi\n";
/// let ast = Builder::document(|b| {
///     let h = b.heading(1, |b| {
///         let t = b.text("Hi");
///         b.set_span(t, 2..4);
///     });
///     b.set_span(h, 0..5);
/// });
/// let heading = ast.node(ast.children(ast.root())[0]);
/// assert_eq!(heading.node_type(), NodeType::Heading);
/// assert_eq!(heading.source(src), "# Hi\n");
/// assert_eq!(ast.span(ast.root()), (0..5).into());
/// ```
#[derive(Debug)]
pub struct Builder {
    ast: Ast,
    /// Open nodes; the last one receives new children.
    stack: Vec<NodeId>,
    /// Per arena index: whether the span is known (set explicitly or derived from children).
    known: Vec<bool>,
}

impl Builder {
    /// Builds a tree with a [`NodeKind::Document`] root.
    pub fn document(f: impl FnOnce(&mut Self)) -> Ast {
        Self::with_root(NodeKind::Document, f)
    }

    /// Builds a tree with an arbitrary root kind.
    pub fn with_root(kind: NodeKind, f: impl FnOnce(&mut Self)) -> Ast {
        let ast = Ast::with_root(kind, Span::default());
        let mut builder = Self {
            stack: vec![ast.root()],
            known: vec![false],
            ast,
        };
        f(&mut builder);
        builder.finish()
    }

    /// The id of the node whose children are being built.
    pub fn current(&self) -> NodeId {
        *self
            .stack
            .last()
            .expect("builder stack holds at least the root")
    }

    /// Sets the span of `id` explicitly.
    pub fn set_span(&mut self, id: NodeId, span: impl Into<Span>) {
        self.ast.set_span(id, span.into());
        self.known[id.index()] = true;
    }

    /// Adds a node and builds its children with `f`.
    pub fn node(&mut self, kind: NodeKind, f: impl FnOnce(&mut Self)) -> NodeId {
        let id = self.leaf(kind);
        self.stack.push(id);
        f(self);
        self.stack.pop();
        id
    }

    /// Adds a childless node.
    pub fn leaf(&mut self, kind: NodeKind) -> NodeId {
        let id = self.ast.create(kind, Span::default());
        self.known.push(false);
        self.ast
            .append_child(self.current(), id)
            .expect("a fresh node can always be appended");
        id
    }

    /// Fills in implicit spans bottom-up and returns the tree.
    fn finish(mut self) -> Ast {
        let order: Vec<NodeId> = self.ast.descendants(self.ast.root()).collect();
        // Reverse pre-order visits every child before its parent.
        for &id in order.iter().rev() {
            if self.known[id.index()] {
                continue;
            }
            let known = self
                .ast
                .children(id)
                .iter()
                .filter(|child| self.known[child.index()])
                .map(|&child| self.ast.span(child));
            let cover = known.reduce(|a, b| Span::new(a.start.min(b.start), a.end.max(b.end)));
            if let Some(span) = cover {
                self.ast.set_span(id, span);
                self.known[id.index()] = true;
            }
        }
        self.ast
    }

    /// Adds a [`NodeKind::Paragraph`].
    pub fn paragraph(&mut self, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::Paragraph, f)
    }

    /// Adds an ATX [`NodeKind::Heading`] of `level`.
    pub fn heading(&mut self, level: u8, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::Heading(Heading::atx(level)), f)
    }

    /// Adds a [`NodeKind::BlockQuote`].
    pub fn block_quote(&mut self, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::BlockQuote, f)
    }

    /// Adds a tight bullet [`NodeKind::List`] with `marker`.
    pub fn bullet_list(&mut self, marker: char, f: impl FnOnce(&mut Self)) -> NodeId {
        let kind = ListKind::Bullet { marker };
        self.node(NodeKind::List(List { kind, tight: true }), f)
    }

    /// Adds a tight ordered [`NodeKind::List`] (`1.` style) starting at `start`.
    pub fn ordered_list(&mut self, start: u64, f: impl FnOnce(&mut Self)) -> NodeId {
        let kind = ListKind::Ordered {
            start,
            delimiter: OrderedDelimiter::Period,
        };
        self.node(NodeKind::List(List { kind, tight: true }), f)
    }

    /// Adds a plain [`NodeKind::ListItem`].
    pub fn item(&mut self, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::ListItem(ListItem::default()), f)
    }

    /// Adds a task [`NodeKind::ListItem`].
    pub fn task(&mut self, state: TaskState, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::ListItem(ListItem { task: Some(state) }), f)
    }

    /// Adds a ```` ``` ````-fenced [`NodeKind::CodeBlock`] with info string `info`.
    pub fn code_block(&mut self, info: &str, literal: &str) -> NodeId {
        self.leaf(NodeKind::CodeBlock(CodeBlock {
            fence: Some(Fence { char: '`', len: 3 }),
            info: info.to_owned(),
            literal: literal.to_owned(),
        }))
    }

    /// Adds a [`NodeKind::HtmlBlock`].
    pub fn html_block(&mut self, html: &str) -> NodeId {
        self.leaf(NodeKind::HtmlBlock(html.to_owned()))
    }

    /// Adds a [`NodeKind::ThematicBreak`].
    pub fn thematic_break(&mut self) -> NodeId {
        self.leaf(NodeKind::ThematicBreak)
    }

    /// Adds a [`NodeKind::Table`] with column `alignments`.
    pub fn table(&mut self, alignments: &[Alignment], f: impl FnOnce(&mut Self)) -> NodeId {
        let table = Table {
            alignments: alignments.to_vec(),
        };
        self.node(NodeKind::Table(table), f)
    }

    /// Adds a [`NodeKind::TableRow`].
    pub fn row(&mut self, header: bool, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::TableRow(TableRow { header }), f)
    }

    /// Adds a [`NodeKind::TableCell`].
    pub fn cell(&mut self, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::TableCell, f)
    }

    /// Adds a [`NodeKind::Text`].
    pub fn text(&mut self, text: &str) -> NodeId {
        self.leaf(NodeKind::Text(text.to_owned()))
    }

    /// Adds a [`NodeKind::Code`] span.
    pub fn code(&mut self, code: &str) -> NodeId {
        self.leaf(NodeKind::Code(code.to_owned()))
    }

    /// Adds a [`NodeKind::HtmlInline`].
    pub fn html_inline(&mut self, html: &str) -> NodeId {
        self.leaf(NodeKind::HtmlInline(html.to_owned()))
    }

    /// Adds a [`NodeKind::SoftBreak`].
    pub fn soft_break(&mut self) -> NodeId {
        self.leaf(NodeKind::SoftBreak)
    }

    /// Adds a [`NodeKind::LineBreak`].
    pub fn line_break(&mut self) -> NodeId {
        self.leaf(NodeKind::LineBreak)
    }

    /// Adds a [`NodeKind::Emphasis`].
    pub fn emphasis(&mut self, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::Emphasis, f)
    }

    /// Adds a [`NodeKind::Strong`].
    pub fn strong(&mut self, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::Strong, f)
    }

    /// Adds a [`NodeKind::Link`].
    pub fn link(&mut self, link: Link, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::Link(link), f)
    }

    /// Adds a [`NodeKind::Image`].
    pub fn image(&mut self, link: Link, f: impl FnOnce(&mut Self)) -> NodeId {
        self.node(NodeKind::Image(link), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::NodeType;

    #[test]
    fn builds_nested_structure_in_order() {
        let mut ids = Vec::new();
        let ast = Builder::document(|b| {
            ids.push(b.current());
            let mut inner = None;
            ids.push(b.block_quote(|b| {
                inner = Some(b.current());
                b.paragraph(|b| {
                    b.text("x");
                });
            }));
            ids.extend(inner);
            b.ordered_list(3, |b| {
                b.task(TaskState::Done, |_| {});
            });
        });
        assert_eq!(ids[0], ast.root());
        assert_eq!(ids[1], ids[2]);
        let types: Vec<_> = ast
            .descendants(ast.root())
            .map(|id| ast.kind(id).node_type())
            .collect();
        assert_eq!(
            types,
            [
                NodeType::Document,
                NodeType::BlockQuote,
                NodeType::Paragraph,
                NodeType::Text,
                NodeType::List,
                NodeType::ListItem,
            ]
        );
        assert!(ast.descendants(ast.root()).all(|id| ast.is_attached(id)));
    }

    #[test]
    fn unspecified_spans_cover_their_children() {
        let ast = Builder::document(|b| {
            b.paragraph(|b| {
                let t = b.text("a");
                b.set_span(t, 4..5);
                b.emphasis(|b| {
                    let t = b.text("b");
                    b.set_span(t, 6..9);
                });
                b.soft_break();
            });
            b.thematic_break();
        });
        let para = ast.children(ast.root())[0];
        assert_eq!(ast.span(para), Span::new(4, 9));
        assert_eq!(ast.span(ast.children(para)[1]), Span::new(6, 9));
        assert_eq!(ast.span(ast.children(para)[2]), Span::empty(0));
        assert_eq!(ast.span(ast.root()), Span::new(4, 9));
    }

    #[test]
    fn explicit_spans_win_over_children() {
        let ast = Builder::document(|b| {
            let root = b.current();
            b.set_span(root, 0..20);
            b.paragraph(|b| {
                let t = b.text("a");
                b.set_span(t, 4..5);
            });
        });
        assert_eq!(ast.span(ast.root()), Span::new(0, 20));
    }

    #[test]
    fn with_root_and_leaf_helpers() {
        let ast = Builder::with_root(NodeKind::Paragraph, |b| {
            b.code("c");
            b.html_inline("<b>");
            b.line_break();
            b.link(Link::inline("u"), |_| {});
            b.image(Link::inline("i"), |_| {});
            b.strong(|_| {});
        });
        assert_eq!(ast.kind(ast.root()), &NodeKind::Paragraph);
        assert_eq!(ast.children(ast.root()).len(), 6);
    }
}
