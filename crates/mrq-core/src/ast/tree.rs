//! The [`Ast`] arena, [`NodeId`]s, [`Node`] handles and structural edits.

use std::fmt;

use super::{NodeKind, NodeType, Span};

/// Identifier of a node within one [`Ast`].
///
/// Ids are indices into the arena: they are cheap to copy, stay valid for the lifetime of
/// the `Ast` (nodes are never freed, detaching only unlinks them), and are assigned in
/// creation order, so for a freshly converted tree they follow document order. An id is
/// meaningless for any other `Ast`; passing a foreign id is a logic error and may panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// The arena index of this id.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Why a structural edit of an [`Ast`] was rejected. Nothing is changed in that case.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    /// The node to insert still has a parent; [`Ast::detach`] it first.
    #[error("node {node} already has a parent")]
    Attached {
        /// The node to insert.
        node: NodeId,
    },
    /// The root cannot be detached, replaced or inserted anywhere.
    #[error("node {node} is the root")]
    Root {
        /// The root.
        node: NodeId,
    },
    /// The node to insert is `parent` itself or one of its ancestors.
    #[error("inserting {node} under {parent} would create a cycle")]
    Cycle {
        /// The node to insert.
        node: NodeId,
        /// The intended parent.
        parent: NodeId,
    },
    /// A sibling-relative operation was given a node without a parent.
    #[error("node {node} has no parent")]
    NoParent {
        /// The parentless node.
        node: NodeId,
    },
    /// A child index past the end of the children.
    #[error("index {index} is out of bounds for {len} children")]
    IndexOutOfBounds {
        /// The requested index.
        index: usize,
        /// Number of children.
        len: usize,
    },
}

#[derive(Debug, Clone)]
struct NodeData {
    kind: NodeKind,
    span: Span,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

/// An owned Markdown syntax tree: an arena of nodes, each with a [`NodeKind`], a byte
/// [`Span`] into the source, a parent link and an ordered list of children.
///
/// The root is created with the tree and can never be moved. Other nodes are created
/// detached with [`create`](Self::create) and linked with [`append_child`](Self::append_child),
/// [`insert_child`](Self::insert_child), [`insert_before`](Self::insert_before),
/// [`insert_after`](Self::insert_after) or [`replace`](Self::replace). Detached nodes stay
/// in the arena (so ids remain valid) but are invisible to traversals from the root.
///
/// Accessors take a [`NodeId`] and panic if it does not belong to this tree. Read-only code
/// usually works with [`Node`] handles from [`node`](Self::node) instead.
#[derive(Debug, Clone)]
pub struct Ast {
    nodes: Vec<NodeData>,
}

impl Ast {
    /// A tree whose root is a [`NodeKind::Document`] spanning `span`.
    pub fn new(span: Span) -> Self {
        Self::with_root(NodeKind::Document, span)
    }

    /// A tree with an arbitrary root node.
    pub fn with_root(kind: NodeKind, span: Span) -> Self {
        let root = NodeData {
            kind,
            span,
            parent: None,
            children: Vec::new(),
        };
        Self { nodes: vec![root] }
    }

    /// The root node's id.
    pub const fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Number of nodes in the arena, including detached ones.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Whether `id` belongs to this arena.
    pub fn contains(&self, id: NodeId) -> bool {
        id.index() < self.nodes.len()
    }

    /// A read-only handle to `id`.
    pub fn node(&self, id: NodeId) -> Node<'_> {
        assert!(self.contains(id), "node {id} does not belong to this tree");
        Node { ast: self, id }
    }

    /// The kind of `id`.
    pub fn kind(&self, id: NodeId) -> &NodeKind {
        &self.data(id).kind
    }

    /// Mutable access to the kind of `id`, e.g. to fill in heading attributes.
    pub fn kind_mut(&mut self, id: NodeId) -> &mut NodeKind {
        &mut self.data_mut(id).kind
    }

    /// Replaces the kind of `id` in place (children and span unchanged) and returns the
    /// old kind.
    pub fn set_kind(&mut self, id: NodeId, kind: NodeKind) -> NodeKind {
        std::mem::replace(self.kind_mut(id), kind)
    }

    /// The source span of `id`.
    pub fn span(&self, id: NodeId) -> Span {
        self.data(id).span
    }

    /// Sets the source span of `id`.
    pub fn set_span(&mut self, id: NodeId, span: Span) {
        self.data_mut(id).span = span;
    }

    /// The parent of `id`, `None` for the root and detached nodes.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.data(id).parent
    }

    /// The children of `id`, in order.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.data(id).children
    }

    /// Position of `id` among its parent's children.
    pub fn index_in_parent(&self, id: NodeId) -> Option<usize> {
        let parent = self.parent(id)?;
        self.children(parent).iter().position(|&child| child == id)
    }

    /// The sibling after `id`.
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let index = self.index_in_parent(id)?;
        self.children(self.parent(id)?).get(index + 1).copied()
    }

    /// The sibling before `id`.
    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        let index = self.index_in_parent(id)?.checked_sub(1)?;
        self.children(self.parent(id)?).get(index).copied()
    }

    /// Whether `id` is the root or reachable from it.
    pub fn is_attached(&self, id: NodeId) -> bool {
        id == self.root() || self.ancestors(id).last() == Some(self.root())
    }

    /// The ancestors of `id`, nearest first, excluding `id` itself.
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            ast: self,
            next: self.parent(id),
        }
    }

    /// Creates a detached node and returns its id.
    ///
    /// # Panics
    ///
    /// If the arena already holds `u32::MAX` nodes.
    pub fn create(&mut self, kind: NodeKind, span: Span) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("too many AST nodes"));
        self.nodes.push(NodeData {
            kind,
            span,
            parent: None,
            children: Vec::new(),
        });
        id
    }

    /// Appends the detached `child` as the last child of `parent`.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), TreeError> {
        let len = self.children(parent).len();
        self.insert_child(parent, len, child)
    }

    /// Inserts the detached `child` at `index` among the children of `parent`
    /// (`index == len` appends).
    pub fn insert_child(
        &mut self,
        parent: NodeId,
        index: usize,
        child: NodeId,
    ) -> Result<(), TreeError> {
        self.check_insertable(parent, child)?;
        let len = self.children(parent).len();
        if index > len {
            return Err(TreeError::IndexOutOfBounds { index, len });
        }
        self.data_mut(parent).children.insert(index, child);
        self.data_mut(child).parent = Some(parent);
        Ok(())
    }

    /// Inserts the detached `node` right before `sibling`.
    pub fn insert_before(&mut self, sibling: NodeId, node: NodeId) -> Result<(), TreeError> {
        let (parent, index) = self.slot(sibling)?;
        self.insert_child(parent, index, node)
    }

    /// Inserts the detached `node` right after `sibling`.
    pub fn insert_after(&mut self, sibling: NodeId, node: NodeId) -> Result<(), TreeError> {
        let (parent, index) = self.slot(sibling)?;
        self.insert_child(parent, index + 1, node)
    }

    /// Unlinks `id` (with its subtree) from its parent. Detaching a detached node is a
    /// no-op.
    pub fn detach(&mut self, id: NodeId) -> Result<(), TreeError> {
        if id == self.root() {
            return Err(TreeError::Root { node: id });
        }
        if let Some(parent) = self.data_mut(id).parent.take() {
            self.data_mut(parent).children.retain(|&child| child != id);
        }
        Ok(())
    }

    /// Puts the detached `new` in the place of `old` and detaches `old`.
    pub fn replace(&mut self, old: NodeId, new: NodeId) -> Result<(), TreeError> {
        let (parent, index) = self.slot(old)?;
        self.check_insertable(parent, new)?;
        self.data_mut(parent).children[index] = new;
        self.data_mut(new).parent = Some(parent);
        self.data_mut(old).parent = None;
        Ok(())
    }

    /// Detaches all children of `id` and returns them in order, e.g. to move them under a
    /// new wrapper node.
    pub fn take_children(&mut self, id: NodeId) -> Vec<NodeId> {
        let children = std::mem::take(&mut self.data_mut(id).children);
        for &child in &children {
            self.data_mut(child).parent = None;
        }
        children
    }

    /// Deep-copies the subtree of `other` rooted at `node` into this arena, detached, and
    /// returns the id of the copy. `map_span` translates each span into this tree's source,
    /// e.g. from a re-parsed admonition body back to the enclosing document.
    pub fn import(
        &mut self,
        other: &Ast,
        node: NodeId,
        mut map_span: impl FnMut(Span) -> Span,
    ) -> NodeId {
        let copy = self.create(other.kind(node).clone(), map_span(other.span(node)));
        // Explicit stack of (source node, copy) pairs whose children still need copying.
        let mut pending = vec![(node, copy)];
        while let Some((source, target)) = pending.pop() {
            for &child in other.children(source) {
                let child_copy =
                    self.create(other.kind(child).clone(), map_span(other.span(child)));
                self.data_mut(child_copy).parent = Some(target);
                self.data_mut(target).children.push(child_copy);
                pending.push((child, child_copy));
            }
        }
        copy
    }

    /// Whether the subtree at `a` in `self` and at `b` in `other` have equal kinds, spans
    /// and shape. Ids are not compared, so trees built in different orders can be equal.
    pub fn subtree_eq(&self, a: NodeId, other: &Ast, b: NodeId) -> bool {
        let mut pending = vec![(a, b)];
        while let Some((x, y)) = pending.pop() {
            let (dx, dy) = (self.data(x), other.data(y));
            if dx.kind != dy.kind || dx.span != dy.span || dx.children.len() != dy.children.len() {
                return false;
            }
            pending.extend(dx.children.iter().copied().zip(dy.children.iter().copied()));
        }
        true
    }

    fn data(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.index()]
    }

    fn data_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id.index()]
    }

    /// The parent of `id` and its index there.
    fn slot(&self, id: NodeId) -> Result<(NodeId, usize), TreeError> {
        if id == self.root() {
            return Err(TreeError::Root { node: id });
        }
        let parent = self.parent(id).ok_or(TreeError::NoParent { node: id })?;
        let index = self
            .index_in_parent(id)
            .expect("child is listed by its parent");
        Ok((parent, index))
    }

    /// Checks that `child` may become a child of `parent`.
    fn check_insertable(&self, parent: NodeId, child: NodeId) -> Result<(), TreeError> {
        if child == self.root() {
            return Err(TreeError::Root { node: child });
        }
        if self.parent(child).is_some() {
            return Err(TreeError::Attached { node: child });
        }
        if parent == child || self.ancestors(parent).any(|id| id == child) {
            return Err(TreeError::Cycle {
                node: child,
                parent,
            });
        }
        Ok(())
    }
}

/// Iterator over the ancestors of a node, nearest first. See [`Ast::ancestors`].
#[derive(Debug, Clone)]
pub struct Ancestors<'a> {
    ast: &'a Ast,
    next: Option<NodeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        self.next = self.ast.parent(current);
        Some(current)
    }
}

/// A read-only handle to a node: an [`Ast`] reference plus a [`NodeId`].
///
/// Navigation methods return further handles, so queries can walk the tree without
/// threading the `Ast` through every call.
#[derive(Clone, Copy)]
pub struct Node<'a> {
    ast: &'a Ast,
    id: NodeId,
}

impl<'a> Node<'a> {
    /// The node's id.
    pub const fn id(self) -> NodeId {
        self.id
    }

    /// The tree the node belongs to.
    pub const fn ast(self) -> &'a Ast {
        self.ast
    }

    /// The node's kind.
    pub fn kind(self) -> &'a NodeKind {
        self.ast.kind(self.id)
    }

    /// Shorthand for `self.kind().node_type()`.
    pub fn node_type(self) -> NodeType {
        self.kind().node_type()
    }

    /// The node's source span.
    pub fn span(self) -> Span {
        self.ast.span(self.id)
    }

    /// The source text of the node: `&src[self.span()]`.
    ///
    /// # Panics
    ///
    /// If the span does not fit `src` (i.e. `src` is not the text the tree was built from).
    pub fn source(self, src: &str) -> &str {
        let span = self.span();
        &src[span.start..span.end]
    }

    /// The parent node.
    pub fn parent(self) -> Option<Self> {
        self.ast.parent(self.id).map(|id| self.ast.node(id))
    }

    /// The children, in order.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Node<'a>> + ExactSizeIterator {
        let ast = self.ast;
        ast.children(self.id).iter().map(move |&id| ast.node(id))
    }

    /// The first child.
    pub fn first_child(self) -> Option<Self> {
        self.ast
            .children(self.id)
            .first()
            .map(|&id| self.ast.node(id))
    }

    /// The last child.
    pub fn last_child(self) -> Option<Self> {
        self.ast
            .children(self.id)
            .last()
            .map(|&id| self.ast.node(id))
    }

    /// The sibling after this node.
    pub fn next_sibling(self) -> Option<Self> {
        self.ast.next_sibling(self.id).map(|id| self.ast.node(id))
    }

    /// The sibling before this node.
    pub fn prev_sibling(self) -> Option<Self> {
        self.ast.prev_sibling(self.id).map(|id| self.ast.node(id))
    }

    /// The ancestors, nearest first.
    pub fn ancestors(self) -> impl Iterator<Item = Node<'a>> {
        let ast = self.ast;
        ast.ancestors(self.id).map(move |id| ast.node(id))
    }
}

impl PartialEq for Node<'_> {
    /// Handles are equal when they refer to the same id of the same tree.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.ast, other.ast) && self.id == other.id
    }
}

impl Eq for Node<'_> {}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let span = self.span();
        write!(
            f,
            "{} {} {}..{}",
            self.id,
            self.node_type(),
            span.start,
            span.end
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Heading;

    fn span(start: usize, end: usize) -> Span {
        Span::new(start, end)
    }

    fn text(s: &str) -> NodeKind {
        NodeKind::Text(s.into())
    }

    /// `document > [paragraph > [text "a", text "b"], thematic_break]`
    fn sample() -> (Ast, NodeId, NodeId, NodeId, NodeId) {
        let mut ast = Ast::new(span(0, 10));
        let para = ast.create(NodeKind::Paragraph, span(0, 2));
        let a = ast.create(text("a"), span(0, 1));
        let b = ast.create(text("b"), span(1, 2));
        let hr = ast.create(NodeKind::ThematicBreak, span(4, 7));
        ast.append_child(ast.root(), para).unwrap();
        ast.append_child(para, a).unwrap();
        ast.append_child(para, b).unwrap();
        ast.append_child(ast.root(), hr).unwrap();
        (ast, para, a, b, hr)
    }

    #[test]
    fn new_tree_has_a_document_root() {
        let ast = Ast::new(span(0, 5));
        assert_eq!(ast.kind(ast.root()), &NodeKind::Document);
        assert_eq!(ast.span(ast.root()), span(0, 5));
        assert_eq!(ast.parent(ast.root()), None);
        assert_eq!(ast.children(ast.root()), []);
        assert_eq!(ast.node_count(), 1);
        assert!(ast.is_attached(ast.root()));
    }

    #[test]
    fn with_root_uses_the_given_kind() {
        let ast = Ast::with_root(NodeKind::BlockQuote, span(1, 2));
        assert_eq!(ast.kind(ast.root()), &NodeKind::BlockQuote);
    }

    #[test]
    fn created_nodes_are_detached_until_linked() {
        let mut ast = Ast::new(span(0, 1));
        let id = ast.create(NodeKind::Paragraph, span(0, 1));
        assert!(ast.contains(id));
        assert!(!ast.is_attached(id));
        assert_eq!(ast.parent(id), None);
        ast.append_child(ast.root(), id).unwrap();
        assert!(ast.is_attached(id));
        assert_eq!(ast.parent(id), Some(ast.root()));
    }

    #[test]
    fn navigation_between_parent_children_and_siblings() {
        let (ast, para, a, b, hr) = sample();
        assert_eq!(ast.children(ast.root()), &[para, hr]);
        assert_eq!(ast.children(para), &[a, b]);
        assert_eq!(ast.parent(a), Some(para));
        assert_eq!(ast.index_in_parent(b), Some(1));
        assert_eq!(ast.index_in_parent(ast.root()), None);
        assert_eq!(ast.next_sibling(a), Some(b));
        assert_eq!(ast.next_sibling(b), None);
        assert_eq!(ast.prev_sibling(b), Some(a));
        assert_eq!(ast.prev_sibling(a), None);
        assert_eq!(ast.next_sibling(ast.root()), None);
        assert_eq!(ast.ancestors(b).collect::<Vec<_>>(), [para, ast.root()]);
    }

    #[test]
    fn node_handles_navigate() {
        let (ast, para, a, b, hr) = sample();
        let root = ast.node(ast.root());
        assert_eq!(
            root.children().map(Node::id).collect::<Vec<_>>(),
            [para, hr]
        );
        let pa = ast.node(para);
        assert_eq!(pa.first_child().map(Node::id), Some(a));
        assert_eq!(pa.last_child().map(Node::id), Some(b));
        assert_eq!(pa.next_sibling().map(Node::id), Some(hr));
        assert_eq!(ast.node(hr).prev_sibling(), Some(pa));
        assert_eq!(ast.node(a).parent(), Some(pa));
        assert_eq!(ast.node(b).ancestors().count(), 2);
        assert_eq!(ast.node(a).node_type(), NodeType::Text);
        assert_eq!(ast.node(hr).source("0123456789"), "456");
        assert_eq!(format!("{:?}", ast.node(hr)), "#4 thematic_break 4..7");
    }

    #[test]
    fn kind_and_span_can_be_changed_in_place() {
        let (mut ast, para, ..) = sample();
        let old = ast.set_kind(para, NodeKind::Heading(Heading::atx(1)));
        assert_eq!(old, NodeKind::Paragraph);
        if let NodeKind::Heading(h) = ast.kind_mut(para) {
            h.level = 2;
        }
        assert_eq!(ast.kind(para), &NodeKind::Heading(Heading::atx(2)));
        ast.set_span(para, span(0, 3));
        assert_eq!(ast.span(para), span(0, 3));
    }

    #[test]
    fn insert_child_at_index_and_relative_to_siblings() {
        let (mut ast, para, a, b, _) = sample();
        let first = ast.create(text("x"), span(0, 0));
        let before = ast.create(text("y"), span(0, 0));
        let after = ast.create(text("z"), span(0, 0));
        ast.insert_child(para, 0, first).unwrap();
        ast.insert_before(b, before).unwrap();
        ast.insert_after(b, after).unwrap();
        assert_eq!(ast.children(para), &[first, a, before, b, after]);
        let last = ast.create(text("w"), span(0, 0));
        assert_eq!(
            ast.insert_child(para, 6, last),
            Err(TreeError::IndexOutOfBounds { index: 6, len: 5 })
        );
        ast.insert_child(para, 5, last).unwrap();
        assert_eq!(ast.children(para).last(), Some(&last));
    }

    #[test]
    fn detach_unlinks_a_subtree_and_keeps_it_in_the_arena() {
        let (mut ast, para, a, b, hr) = sample();
        ast.detach(para).unwrap();
        assert_eq!(ast.children(ast.root()), &[hr]);
        assert_eq!(ast.parent(para), None);
        assert!(!ast.is_attached(a));
        assert_eq!(ast.children(para), &[a, b]);
        ast.detach(para).unwrap();
        assert_eq!(
            ast.detach(ast.root()),
            Err(TreeError::Root { node: ast.root() })
        );
    }

    #[test]
    fn replace_swaps_a_node_for_a_detached_one() {
        let (mut ast, para, _, _, hr) = sample();
        let quote = ast.create(NodeKind::BlockQuote, span(0, 2));
        ast.replace(para, quote).unwrap();
        assert_eq!(ast.children(ast.root()), &[quote, hr]);
        assert_eq!(ast.parent(para), None);
        assert_eq!(ast.parent(quote), Some(ast.root()));
        // `para` is detached now, so it can be moved under the quote.
        ast.append_child(quote, para).unwrap();
        assert_eq!(ast.ancestors(para).collect::<Vec<_>>(), [quote, ast.root()]);
    }

    #[test]
    fn take_children_detaches_them_in_order() {
        let (mut ast, para, a, b, _) = sample();
        assert_eq!(ast.take_children(para), [a, b]);
        assert_eq!(ast.children(para), []);
        assert_eq!(ast.parent(a), None);
    }

    #[test]
    fn inserting_an_attached_node_is_rejected() {
        let (mut ast, para, a, _, hr) = sample();
        assert_eq!(
            ast.append_child(hr, a),
            Err(TreeError::Attached { node: a })
        );
        assert_eq!(
            ast.insert_before(hr, para),
            Err(TreeError::Attached { node: para })
        );
        assert_eq!(ast.replace(hr, a), Err(TreeError::Attached { node: a }));
        assert_eq!(ast.children(para).len(), 2);
    }

    #[test]
    fn root_cannot_be_inserted_or_replaced() {
        let (mut ast, para, ..) = sample();
        let root = ast.root();
        assert_eq!(
            ast.append_child(para, root),
            Err(TreeError::Root { node: root })
        );
        let x = ast.create(NodeKind::Paragraph, span(0, 0));
        assert_eq!(ast.replace(root, x), Err(TreeError::Root { node: root }));
    }

    #[test]
    fn cycles_are_rejected() {
        let (mut ast, para, a, ..) = sample();
        ast.detach(para).unwrap();
        assert_eq!(
            ast.append_child(a, para),
            Err(TreeError::Cycle {
                node: para,
                parent: a
            })
        );
        assert_eq!(
            ast.append_child(para, para),
            Err(TreeError::Cycle {
                node: para,
                parent: para
            })
        );
    }

    #[test]
    fn sibling_operations_need_a_parent() {
        let mut ast = Ast::new(span(0, 0));
        let lone = ast.create(NodeKind::Paragraph, span(0, 0));
        let x = ast.create(NodeKind::Paragraph, span(0, 0));
        assert_eq!(
            ast.insert_after(lone, x),
            Err(TreeError::NoParent { node: lone })
        );
        assert_eq!(
            ast.replace(lone, x),
            Err(TreeError::NoParent { node: lone })
        );
    }

    #[test]
    fn import_copies_a_subtree_mapping_spans() {
        let (other, para, ..) = sample();
        let mut ast = Ast::new(span(0, 100));
        let copy = ast.import(&other, para, |s| Span::new(s.start + 10, s.end + 10));
        assert!(!ast.is_attached(copy));
        assert_eq!(ast.span(copy), span(10, 12));
        let kids: Vec<_> = ast.children(copy).to_vec();
        assert_eq!(kids.len(), 2);
        assert_eq!(ast.kind(kids[1]), &text("b"));
        assert_eq!(ast.span(kids[1]), span(11, 12));
        assert_eq!(ast.parent(kids[0]), Some(copy));
    }

    #[test]
    fn subtree_eq_ignores_ids() {
        let (one, para, ..) = sample();
        let mut two = Ast::new(span(0, 10));
        let copy = two.import(&one, para, |s| s);
        assert!(two.subtree_eq(copy, &one, para));
        two.set_span(two.children(copy)[0], span(0, 0));
        assert!(!two.subtree_eq(copy, &one, para));
        assert!(!one.subtree_eq(one.root(), &one, para));
    }

    #[test]
    fn errors_display() {
        let id = NodeId(3);
        assert_eq!(
            TreeError::Attached { node: id }.to_string(),
            "node #3 already has a parent"
        );
    }
}
