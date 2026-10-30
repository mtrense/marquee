//! The `block_comments` pass: `%%` comments that span blocks.
//!
//! comrak splits a comment like
//!
//! ```text
//! %%
//! Hidden paragraph.
//!
//! Another one.
//! %%
//! ```
//!
//! into separate paragraphs, so the inline pass can't see it. This pass looks at the
//! children of every block container: a paragraph whose only unescaped `%%` (in its text)
//! is at its very start opens a comment, and the next sibling paragraph with any `%%` must
//! have exactly one, at its very end, to close it. The opener, the closer and every block
//! between them (code blocks, lists, …) are replaced by one block-level
//! [`Comment`](crate::ast::NodeKind::Comment) spanning `%%…%%`, whose literal is the
//! verbatim source in between (container markers such as `> ` included).
//!
//! Limits: a comment that opens or closes mid-paragraph, or whose closer is nested
//! deeper (e.g. in a list) or at a different level than its opener, is not recognized
//! (its `%%` stay text). A comment within one paragraph is the inline pass's job.

use super::inlines::percent_delimiters;
use crate::ast::{Ast, Comment, CommentSyntax, NodeId, NodeKind, Span};
use crate::flavor::{Pass, PassContext};

pub(super) struct BlockComments;

impl Pass for BlockComments {
    fn name(&self) -> &'static str {
        "block_comments"
    }

    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
        let containers: Vec<NodeId> = ast
            .descendants(ast.root())
            .filter(|&id| {
                ast.children(id)
                    .iter()
                    .any(|&child| ast.kind(child) == &NodeKind::Paragraph)
            })
            .collect();
        for container in containers {
            join(ast, cx.src(), container);
        }
    }
}

/// Replaces each run of children of `container` from an opening to a closing paragraph
/// by one comment.
fn join(ast: &mut Ast, src: &str, container: NodeId) {
    let children = ast.children(container).to_vec();
    let mut i = 0;
    while i < children.len() {
        let open = children[i];
        let opens = delimiters(ast, src, open).is_some_and(|d| d == [ast.span(open).start]);
        let close = opens
            .then(|| {
                children[i + 1..]
                    .iter()
                    .enumerate()
                    .find_map(|(n, &id)| delimiters(ast, src, id).map(|d| (i + 1 + n, id, d)))
            })
            .flatten()
            .filter(|&(_, id, ref d)| d[..] == [ast.span(id).end - 2]);
        let Some((last, close, _)) = close else {
            i += 1;
            continue;
        };
        let span = Span::new(ast.span(open).start, ast.span(close).end);
        let comment = ast.create(
            NodeKind::Comment(Comment {
                syntax: CommentSyntax::Percent,
                literal: src[span.start + 2..span.end - 2].to_owned(),
            }),
            span,
        );
        ast.insert_before(open, comment)
            .expect("an attached child has a parent");
        for &id in &children[i..=last] {
            ast.detach(id).expect("a child is not the root");
        }
        i = last + 1;
    }
}

/// For a paragraph with `%%` delimiters in its text, their source offsets.
fn delimiters(ast: &Ast, src: &str, id: NodeId) -> Option<Vec<usize>> {
    if ast.kind(id) != &NodeKind::Paragraph {
        return None;
    }
    let found: Vec<usize> = ast
        .descendants(id)
        .filter(|&node| matches!(ast.kind(node), NodeKind::Text(_)))
        .flat_map(|node| {
            let span = ast.span(node);
            percent_delimiters(&src[span.start..span.end])
                .into_iter()
                .map(move |at| span.start + at)
        })
        .collect();
    (!found.is_empty()).then_some(found)
}
