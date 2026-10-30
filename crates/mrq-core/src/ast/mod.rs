//! mrq's owned Markdown syntax tree.
//!
//! An [`Ast`] is an arena of nodes addressed by [`NodeId`]. Every node has a
//! [`NodeKind`] (what it is, plus kind-specific data), a byte [`Span`] into the original
//! source, a parent and ordered children. The parser converts comrak's tree into an `Ast`;
//! flavor passes then rewrite parts of it (replacing a paragraph by an admonition, splitting
//! text into tags, …) through the structural edits on [`Ast`]. Nothing else in mrq sees
//! parser-specific types.
//!
//! Reading:
//!
//! - [`Node`] is a `Copy` handle (`&Ast` + id) with navigation (`parent`, `children`,
//!   siblings, `ancestors`) and [`Node::text`] for plain-text extraction.
//! - [`Ast::descendants`] iterates a subtree in document (pre-)order; [`Ast::traverse`]
//!   yields [`Event::Enter`]/[`Event::Leave`] pairs, which suits printers.
//! - [`Visitor`] with [`Ast::walk`] gives callback-style traversal with pruning
//!   ([`Walk::SkipChildren`]) and early exit ([`Walk::Stop`]).
//! - [`Ast::dump`] renders a subtree as an indented outline for snapshot tests.
//!
//! Editing: collect the ids to change first (ids stay valid across edits), then mutate.
//!
//! ```
//! use mrq_core::ast::{Builder, NodeKind, NodeType};
//!
//! let mut ast = Builder::document(|b| {
//!     b.heading(1, |b| { b.text("Title"); });
//!     b.paragraph(|b| { b.text("Hello "); b.strong(|b| { b.text("world"); }); });
//! });
//! let strong: Vec<_> = ast
//!     .descendants(ast.root())
//!     .filter(|&id| ast.kind(id).node_type() == NodeType::Strong)
//!     .collect();
//! for id in strong {
//!     ast.set_kind(id, NodeKind::Emphasis);
//! }
//! assert_eq!(ast.node(ast.root()).text(), "Title\nHello world");
//! ```
//!
//! # Spans
//!
//! A [`Span`] is a [`ByteRange`] into the full source text the tree was parsed from
//! (frontmatter included, offsets not shifted), so `&src[span]` is the node's source text
//! ([`Node::source`]). Nodes synthesized by passes should get the span of the source they
//! stand for; [`Builder`] trees for tests may leave spans empty.
//!
//! # Stability
//!
//! [`NodeKind`], [`NodeType`] and the payload enums are `#[non_exhaustive]`: new syntax
//! adds variants without a breaking change. The JSON projection ([`json`]), not this
//! module, is the versioned external contract.
//!
//! [`ByteRange`]: crate::source::ByteRange

mod builder;
mod dump;
pub mod json;
mod kind;
mod text;
mod tree;
mod walk;

pub use builder::Builder;
pub use kind::{
    AbbreviationDefinition, Admonition, Alignment, Callout, CodeBlock, Comment, CommentSyntax,
    Directive, DirectiveAttribute, Fence, Fold, FootnoteDefinition, FootnoteReference, FrontMatter,
    FrontMatterFormat, Heading, HeadingAttrs, Link, LinkDefinition, LinkForm, List, ListItem,
    ListKind, Math, NodeKind, NodeType, OrderedDelimiter, ReferenceStyle, Table, TableRow, Tag,
    TaskState, WikiLink,
};
pub use tree::{Ancestors, Ast, Node, NodeId, TreeError};
pub use walk::{Descendants, Event, Traverse, Visitor, Walk};

/// A byte range into the source text; the same type as [`crate::source::ByteRange`], so
/// spans feed straight into diagnostics and [`TextEdit`](crate::source::TextEdit)s.
pub type Span = crate::source::ByteRange;
