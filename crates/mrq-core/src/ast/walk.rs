//! Traversal: enter/leave event iteration, pre-order descendants and the [`Visitor`] walker.

use super::{Ast, Node, NodeId};

/// One step of a depth-first traversal: a node is entered before its children and left
/// after them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    /// Before the node's children.
    Enter(NodeId),
    /// After the node's children.
    Leave(NodeId),
}

/// Depth-first iterator of [`Event`]s over a subtree. See [`Ast::traverse`].
///
/// The tree is borrowed, so it cannot change during iteration; collect ids first to edit.
#[derive(Debug, Clone)]
pub struct Traverse<'a> {
    ast: &'a Ast,
    /// The subtree root, until it has been entered.
    pending: Option<NodeId>,
    /// Open nodes with the index of the next child to enter.
    stack: Vec<(NodeId, usize)>,
}

impl Traverse<'_> {
    /// Skips the children of the node just entered: the next event is its `Leave`.
    ///
    /// Has no effect if the last event was a `Leave`.
    pub fn skip_children(&mut self) {
        if let Some((id, next)) = self.stack.last_mut() {
            *next = self.ast.children(*id).len();
        }
    }
}

impl Iterator for Traverse<'_> {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        if let Some(root) = self.pending.take() {
            self.stack.push((root, 0));
            return Some(Event::Enter(root));
        }
        let (id, next) = self.stack.last_mut()?;
        let id = *id;
        if let Some(&child) = self.ast.children(id).get(*next) {
            *next += 1;
            self.stack.push((child, 0));
            Some(Event::Enter(child))
        } else {
            self.stack.pop();
            Some(Event::Leave(id))
        }
    }
}

/// Pre-order (document order) iterator over a subtree, starting with its root. See
/// [`Ast::descendants`].
#[derive(Debug, Clone)]
pub struct Descendants<'a>(Traverse<'a>);

impl Iterator for Descendants<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        self.0.find_map(|event| match event {
            Event::Enter(id) => Some(id),
            Event::Leave(_) => None,
        })
    }
}

/// What a [`Visitor`] wants the walk to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Walk {
    /// Keep going.
    #[default]
    Continue,
    /// From [`Visitor::enter`]: do not visit this node's children (its `leave` still
    /// runs). From [`Visitor::leave`]: same as `Continue`.
    SkipChildren,
    /// End the walk immediately; no further callbacks run.
    Stop,
}

/// Callbacks for [`Ast::walk`]. Both default to [`Walk::Continue`].
pub trait Visitor {
    /// Called before the node's children.
    fn enter(&mut self, node: Node<'_>) -> Walk {
        let _ = node;
        Walk::Continue
    }

    /// Called after the node's children.
    fn leave(&mut self, node: Node<'_>) -> Walk {
        let _ = node;
        Walk::Continue
    }
}

impl Ast {
    /// Depth-first [`Event`]s over the subtree rooted at `id`.
    pub fn traverse(&self, id: NodeId) -> Traverse<'_> {
        Traverse {
            ast: self,
            pending: Some(id),
            stack: Vec::new(),
        }
    }

    /// The subtree rooted at `id` in pre-order (document order), `id` first.
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants(self.traverse(id))
    }

    /// Walks the subtree rooted at `id`, calling `visitor` on every node. Returns
    /// [`Walk::Stop`] if the visitor stopped the walk, otherwise [`Walk::Continue`].
    pub fn walk(&self, id: NodeId, visitor: &mut impl Visitor) -> Walk {
        let mut events = self.traverse(id);
        while let Some(event) = events.next() {
            let walk = match event {
                Event::Enter(id) => visitor.enter(self.node(id)),
                Event::Leave(id) => visitor.leave(self.node(id)),
            };
            match walk {
                Walk::Stop => return Walk::Stop,
                Walk::SkipChildren if matches!(event, Event::Enter(_)) => events.skip_children(),
                Walk::SkipChildren | Walk::Continue => {}
            }
        }
        Walk::Continue
    }
}

impl<'a> Node<'a> {
    /// This node and its descendants in pre-order.
    pub fn descendants(self) -> impl Iterator<Item = Node<'a>> {
        let ast = self.ast();
        ast.descendants(self.id()).map(move |id| ast.node(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Builder, NodeType};

    /// `document > [paragraph > [text, emphasis > [text]], thematic_break]`
    fn sample() -> Ast {
        Builder::document(|b| {
            b.paragraph(|b| {
                b.text("a");
                b.emphasis(|b| {
                    b.text("b");
                });
            });
            b.thematic_break();
        })
    }

    fn types(ast: &Ast, ids: impl IntoIterator<Item = NodeId>) -> Vec<&'static str> {
        ids.into_iter()
            .map(|id| ast.kind(id).node_type().as_str())
            .collect()
    }

    #[test]
    fn traverse_enters_and_leaves_in_depth_first_order() {
        let ast = sample();
        let events: Vec<String> = ast
            .traverse(ast.root())
            .map(|event| match event {
                Event::Enter(id) => format!("+{}", ast.kind(id).node_type()),
                Event::Leave(id) => format!("-{}", ast.kind(id).node_type()),
            })
            .collect();
        assert_eq!(
            events,
            [
                "+document",
                "+paragraph",
                "+text",
                "-text",
                "+emphasis",
                "+text",
                "-text",
                "-emphasis",
                "-paragraph",
                "+thematic_break",
                "-thematic_break",
                "-document",
            ]
        );
    }

    #[test]
    fn traverse_can_skip_children() {
        let ast = sample();
        let mut iter = ast.traverse(ast.root());
        let mut entered = Vec::new();
        while let Some(event) = iter.next() {
            if let Event::Enter(id) = event {
                entered.push(id);
                if ast.kind(id).node_type() == NodeType::Paragraph {
                    iter.skip_children();
                }
            }
        }
        assert_eq!(
            types(&ast, entered),
            ["document", "paragraph", "thematic_break"]
        );
    }

    #[test]
    fn descendants_are_in_pre_order_and_scoped_to_the_subtree() {
        let ast = sample();
        assert_eq!(
            types(&ast, ast.descendants(ast.root())),
            [
                "document",
                "paragraph",
                "text",
                "emphasis",
                "text",
                "thematic_break"
            ]
        );
        let para = ast.children(ast.root())[0];
        assert_eq!(
            types(&ast, ast.descendants(para)),
            ["paragraph", "text", "emphasis", "text"]
        );
        let leaf = ast.children(ast.root())[1];
        assert_eq!(ast.node(leaf).descendants().count(), 1);
    }

    #[test]
    fn detached_nodes_are_not_traversed() {
        let mut ast = sample();
        let para = ast.children(ast.root())[0];
        ast.detach(para).unwrap();
        assert_eq!(
            types(&ast, ast.descendants(ast.root())),
            ["document", "thematic_break"]
        );
    }

    #[derive(Default)]
    struct Recorder {
        log: Vec<String>,
        skip: Option<NodeType>,
        stop: Option<NodeType>,
    }

    impl Visitor for Recorder {
        fn enter(&mut self, node: Node<'_>) -> Walk {
            self.log.push(format!("+{}", node.node_type()));
            if Some(node.node_type()) == self.stop {
                Walk::Stop
            } else if Some(node.node_type()) == self.skip {
                Walk::SkipChildren
            } else {
                Walk::Continue
            }
        }

        fn leave(&mut self, node: Node<'_>) -> Walk {
            self.log.push(format!("-{}", node.node_type()));
            Walk::Continue
        }
    }

    #[test]
    fn walk_visits_every_node() {
        let ast = sample();
        let mut rec = Recorder::default();
        assert_eq!(ast.walk(ast.root(), &mut rec), Walk::Continue);
        assert_eq!(rec.log.len(), 12);
        assert_eq!(rec.log.first().map(String::as_str), Some("+document"));
        assert_eq!(rec.log.last().map(String::as_str), Some("-document"));
    }

    #[test]
    fn walk_skip_children_still_leaves_the_node() {
        let ast = sample();
        let mut rec = Recorder {
            skip: Some(NodeType::Emphasis),
            ..Recorder::default()
        };
        ast.walk(ast.root(), &mut rec);
        let at = rec.log.iter().position(|s| s == "+emphasis").unwrap();
        assert_eq!(rec.log[at + 1], "-emphasis");
    }

    #[test]
    fn walk_stop_ends_immediately() {
        let ast = sample();
        let mut rec = Recorder {
            stop: Some(NodeType::Emphasis),
            ..Recorder::default()
        };
        assert_eq!(ast.walk(ast.root(), &mut rec), Walk::Stop);
        assert_eq!(
            rec.log,
            ["+document", "+paragraph", "+text", "-text", "+emphasis"]
        );
    }

    #[test]
    fn stop_from_leave_ends_the_walk() {
        struct StopOnLeave(usize);
        impl Visitor for StopOnLeave {
            fn leave(&mut self, _: Node<'_>) -> Walk {
                self.0 += 1;
                Walk::Stop
            }
        }
        let ast = sample();
        let mut v = StopOnLeave(0);
        assert_eq!(ast.walk(ast.root(), &mut v), Walk::Stop);
        assert_eq!(v.0, 1);
    }
}
