//! The `definition_lists` pass: every definition of a term under the term's item.
//!
//! comrak turns
//!
//! ```text
//! Term
//! :   First
//! :   Second
//! ```
//!
//! into two definition items, the second without a term. The AST has one
//! [`DefinitionItem`](NodeKind::DefinitionItem) per term with all of its details, so the
//! pass moves the children of an item that does not start with a term into the item
//! before it, which then spans them too.

use crate::ast::{Ast, NodeId, NodeKind, Span};
use crate::flavor::{Pass, PassContext};

pub(super) struct DefinitionLists;

impl Pass for DefinitionLists {
    fn name(&self) -> &'static str {
        "definition_lists"
    }

    fn run(&self, ast: &mut Ast, _cx: &mut PassContext<'_>) {
        let items: Vec<NodeId> = ast
            .descendants(ast.root())
            .filter(|&id| ast.kind(id) == &NodeKind::DefinitionItem)
            .collect();
        for item in items {
            let starts_with_term = ast
                .children(item)
                .first()
                .is_some_and(|&first| ast.kind(first) == &NodeKind::DefinitionTerm);
            let Some(previous) = ast.prev_sibling(item).filter(|_| !starts_with_term) else {
                continue;
            };
            for child in ast.take_children(item) {
                ast.append_child(previous, child)
                    .expect("taken children are detached");
            }
            let span = ast.span(previous);
            ast.set_span(previous, Span::new(span.start, ast.span(item).end));
            ast.detach(item).expect("an item is not the root");
        }
    }
}
