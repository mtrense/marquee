//! `mrq:` directives: `<!-- mrq:include … -->`, `<!-- mrq:toc … -->` and
//! `<!-- mrq:end -->` comments, and the regions they delimit.
//!
//! Directives are flavor-independent: [`flavor::parse`](crate::flavor::parse) recognizes
//! them after the flavor's passes, so comments in nested bodies (mkdocs admonitions) are
//! covered too. Only block-level HTML [`Comment`](crate::ast::NodeKind::Comment)s whose
//! text starts with `mrq:` (after whitespace) are directives; text in code blocks is never
//! a comment, and other comments stay comments.
//!
//! # Syntax
//!
//! ```markdown
//! <!-- mrq:include path="chapters/install.md" shift=1 dedent -->
//! <!-- mrq:end -->
//! ```
//!
//! After the name come whitespace-separated attributes, possibly over several lines:
//! `name=value` (up to the next whitespace), `name="value"` or `name='value'` (in which
//! `\"`, `\'` and `\\` stand for the escaped character and other backslashes are kept),
//! or a bare `name`, whose value is `true`. Names are ASCII letters, digits, `_` and `-`.
//! Attribute names are not checked here (the commands using them do that); each
//! [`DirectiveAttribute`] spans its `name=value` in the
//! source.
//!
//! A well-formed directive comment becomes an
//! [`IncludeDirective`](crate::ast::NodeKind::IncludeDirective),
//! [`TocDirective`](crate::ast::NodeKind::TocDirective) or
//! [`DirectiveEnd`](crate::ast::NodeKind::DirectiveEnd) node in place. These are flat
//! siblings; the pairing lives in [`DirectiveRegion`]s
//! ([`Document::directives`](crate::Document::directives)) and is not part of the
//! [JSON projection](crate::ast::json).
//!
//! # Regions
//!
//! An `mrq:end` closes the nearest preceding unclosed start marker *among its siblings*:
//! start and end must sit in the same container (document, list item, blockquote, …).
//! Regions do not nest. A start marker that no end closes is a *bare* region
//! ([`DirectiveRegion::is_bare`]); that is allowed (`interpolate --in-place` and
//! `toc --insert` insert the end marker), also when another start follows it.
//!
//! # Diagnostics
//!
//! All have the code [`DIRECTIVE_ERROR_CODE`] and a span:
//!
//! - errors: an unknown directive name (`mrq:incldue`) or malformed attributes (the
//!   comment stays a comment), a duplicate attribute (the first one wins), an `mrq:end`
//!   with no start, and an `mrq:end` whose nearest unclosed start sits in another
//!   container ("nested incorrectly", labelled at the start; that start forms no region,
//!   so tools leave it alone);
//! - warnings: a directive comment inside a paragraph, heading or table cell (it stays a
//!   comment), and attributes on `mrq:end` (ignored).

mod syntax;

use std::collections::HashMap;
use std::path::Path;

use self::syntax::{Name, RawAttribute, Syntax};
use crate::ast::{Ast, CommentSyntax, Directive, DirectiveAttribute, NodeId, NodeKind, Span};
use crate::diag::{Diagnostic, Label, Severity};

/// Diagnostic code of malformed and unpaired directives.
pub const DIRECTIVE_ERROR_CODE: &str = "directive";

/// The start marker of a [`DirectiveRegion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectiveKind {
    /// `<!-- mrq:include … -->`
    Include,
    /// `<!-- mrq:toc … -->`
    Toc,
}

/// A start marker and the `mrq:end` that closes it, if any. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DirectiveRegion {
    pub kind: DirectiveKind,
    /// The [`IncludeDirective`](NodeKind::IncludeDirective) or
    /// [`TocDirective`](NodeKind::TocDirective) node.
    pub start: NodeId,
    /// The [`DirectiveEnd`](NodeKind::DirectiveEnd) sibling; `None` for a bare start.
    pub end: Option<NodeId>,
    /// The source bytes between the markers (newlines and container prefixes such as
    /// `> ` included): the range a refresh replaces. Empty, at the end of the start
    /// marker, for a bare start.
    pub content: Span,
}

impl DirectiveRegion {
    /// Whether no `mrq:end` closes the region.
    pub const fn is_bare(&self) -> bool {
        self.end.is_none()
    }

    /// From the start of the start marker to the end of the end marker (of the start
    /// marker, if bare).
    pub fn span(&self, ast: &Ast) -> Span {
        let start = ast.span(self.start);
        Span::new(
            start.start,
            self.end.map_or(start.end, |end| ast.span(end).end),
        )
    }

    /// The siblings between the markers; empty for a bare start.
    pub fn content_nodes<'a>(&self, ast: &'a Ast) -> &'a [NodeId] {
        let (Some(end), Some(parent)) = (self.end, ast.parent(self.start)) else {
            return &[];
        };
        let index = |id| ast.index_in_parent(id).expect("a marker is attached");
        &ast.children(parent)[index(self.start) + 1..index(end)]
    }

    /// The start marker's attributes.
    pub fn directive<'a>(&self, ast: &'a Ast) -> &'a Directive {
        match ast.kind(self.start) {
            NodeKind::IncludeDirective(directive) | NodeKind::TocDirective(directive) => directive,
            other => panic!("a region starts at a directive, not {other:?}"),
        }
    }
}

/// Turns the directive comments of `ast` (the tree of `src`) into directive nodes and
/// pairs them into regions, in document order.
pub(crate) fn recognize(
    ast: &mut Ast,
    src: &str,
    file: &Path,
) -> (Vec<DirectiveRegion>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let comments: Vec<NodeId> = ast
        .descendants(ast.root())
        .filter(
            |&id| matches!(ast.kind(id), NodeKind::Comment(c) if c.syntax == CommentSyntax::Html),
        )
        .collect();
    for id in comments {
        let NodeKind::Comment(comment) = ast.kind(id) else {
            unreachable!()
        };
        let Some(parsed) = syntax::parse(&comment.literal) else {
            continue;
        };
        let span = ast.span(id);
        let offsets = Offsets::new(src, span, &comment.literal);
        let report = |severity, range, message: String| {
            Diagnostic::new(DIRECTIVE_ERROR_CODE, severity, file, range, message)
        };
        if !is_block_level(ast, id) {
            diagnostics.push(
                report(
                    Severity::Warning,
                    span,
                    "a directive must be a block of its own; this one is ignored".into(),
                )
                .with_help("put the directive comment on its own lines"),
            );
            continue;
        }
        match parsed {
            Err(err) => diagnostics.push(report(
                Severity::Error,
                offsets.span(err.range),
                err.message,
            )),
            Ok(Syntax { name, attributes }) => {
                let attributes: Vec<DirectiveAttribute> = attributes
                    .into_iter()
                    .map(|RawAttribute { name, value, range }| DirectiveAttribute {
                        name,
                        value,
                        span: offsets.span(range),
                    })
                    .collect();
                let kind = match name {
                    Name::Include => NodeKind::IncludeDirective(Directive { attributes }),
                    Name::Toc => NodeKind::TocDirective(Directive { attributes }),
                    Name::End => {
                        if let (Some(first), Some(last)) = (attributes.first(), attributes.last()) {
                            diagnostics.push(report(
                                Severity::Warning,
                                Span::new(first.span.start, last.span.end),
                                "`mrq:end` takes no attributes; they are ignored".into(),
                            ));
                        }
                        NodeKind::DirectiveEnd
                    }
                };
                if let NodeKind::IncludeDirective(d) | NodeKind::TocDirective(d) = &kind {
                    diagnostics.extend(duplicates(d, |range, message| {
                        report(Severity::Error, range, message)
                    }));
                }
                ast.set_kind(id, kind);
            }
        }
    }
    let regions = pair(ast, file, &mut diagnostics);
    (regions, diagnostics)
}

/// Whether the comment `id` stands among blocks rather than inside inline content.
fn is_block_level(ast: &Ast, id: NodeId) -> bool {
    ast.parent(id).is_some_and(|parent| {
        let kind = ast.kind(parent);
        !kind.is_inline()
            && !matches!(
                kind,
                NodeKind::Paragraph | NodeKind::Heading(_) | NodeKind::TableCell
            )
    })
}

/// An error for every attribute whose name appeared before, labelled at the first one.
fn duplicates(
    directive: &Directive,
    report: impl Fn(Span, String) -> Diagnostic,
) -> Vec<Diagnostic> {
    let mut first: HashMap<&str, Span> = HashMap::new();
    let mut out = Vec::new();
    for attr in &directive.attributes {
        if let Some(&earlier) = first.get(attr.name.as_str()) {
            out.push(
                report(attr.span, format!("duplicate attribute `{}`", attr.name))
                    .with_label(Label::new(earlier, "first given here; this value is used")),
            );
        } else {
            first.insert(&attr.name, attr.span);
        }
    }
    out
}

/// Pairs the directive nodes of `ast` (see the [module docs](self)), reporting broken
/// end markers.
fn pair(ast: &Ast, file: &Path, diagnostics: &mut Vec<Diagnostic>) -> Vec<DirectiveRegion> {
    // The open start of each container, and every start not closed yet, in order.
    let mut open: HashMap<NodeId, NodeId> = HashMap::new();
    let mut unclosed: Vec<NodeId> = Vec::new();
    let mut ends: HashMap<NodeId, Option<NodeId>> = HashMap::new();
    let mut starts = Vec::new();
    for id in ast.descendants(ast.root()) {
        let parent = ast.parent(id).unwrap_or(id);
        match ast.kind(id) {
            NodeKind::IncludeDirective(_) | NodeKind::TocDirective(_) => {
                open.insert(parent, id);
                unclosed.push(id);
                starts.push(id);
                ends.insert(id, None);
            }
            NodeKind::DirectiveEnd => {
                if let Some(start) = open.remove(&parent) {
                    unclosed.retain(|&s| s != start);
                    ends.insert(start, Some(id));
                } else if let Some(start) = unclosed
                    .last()
                    .copied()
                    .filter(|&start| ast.parent(start) != Some(parent))
                {
                    unclosed.pop();
                    open.retain(|_, &mut s| s != start);
                    ends.remove(&start);
                    diagnostics.push(
                        Diagnostic::new(
                            DIRECTIVE_ERROR_CODE,
                            Severity::Error,
                            file,
                            ast.span(id),
                            format!(
                                "`mrq:end` is nested differently than its `{}`; both \
                                 must be in the same container",
                                marker_name(ast.kind(start))
                            ),
                        )
                        .with_label(Label::new(ast.span(start), "the region starts here")),
                    );
                } else {
                    diagnostics.push(Diagnostic::new(
                        DIRECTIVE_ERROR_CODE,
                        Severity::Error,
                        file,
                        ast.span(id),
                        "`mrq:end` without a preceding `mrq:include` or `mrq:toc`",
                    ));
                }
            }
            _ => {}
        }
    }
    starts
        .into_iter()
        .filter_map(|start| {
            let end = *ends.get(&start)?;
            let kind = match ast.kind(start) {
                NodeKind::IncludeDirective(_) => DirectiveKind::Include,
                _ => DirectiveKind::Toc,
            };
            let after = ast.span(start).end;
            let content = Span::new(after, end.map_or(after, |end| ast.span(end).start));
            Some(DirectiveRegion {
                kind,
                start,
                end,
                content,
            })
        })
        .collect()
}

/// `mrq:include` or `mrq:toc`.
fn marker_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::IncludeDirective(_) => "mrq:include",
        _ => "mrq:toc",
    }
}

/// Maps offsets in a comment's literal to the source.
///
/// The literal is the text between `<!--` and `-->` with container prefixes (`> `,
/// list or admonition indentation) removed from its continuation lines, so each literal
/// line is aligned with the end of its source line.
struct Offsets {
    /// `(literal offset, source offset)` of each line start.
    lines: Vec<(usize, usize)>,
    /// The end of the comment in the source.
    limit: usize,
}

impl Offsets {
    fn new(src: &str, span: Span, literal: &str) -> Self {
        let first = span.start + "<!--".len();
        let Some(inner) = src
            .get(span.start..span.end)
            .and_then(|s| s.strip_prefix("<!--"))
            .and_then(|s| s.trim_end().strip_suffix("-->"))
        else {
            return Self {
                lines: vec![(0, first)],
                limit: span.end,
            };
        };
        let mut lines = Vec::new();
        let (mut lit_at, mut src_at) = (0, first);
        for (n, (lit, source)) in literal.split('\n').zip(inner.split('\n')).enumerate() {
            let (lit_line, src_line) = (lit.trim_end_matches('\r'), source.trim_end_matches('\r'));
            let shift = if n > 0 && src_line.ends_with(lit_line) {
                src_line.len() - lit_line.len()
            } else {
                0
            };
            lines.push((lit_at, src_at + shift));
            lit_at += lit.len() + 1;
            src_at += source.len() + 1;
        }
        Self {
            lines,
            limit: span.end,
        }
    }

    fn offset(&self, at: usize) -> usize {
        let &(lit, src) = self
            .lines
            .iter()
            .rev()
            .find(|&&(lit, _)| lit <= at)
            .expect("the first line starts at 0");
        (src + at - lit).min(self.limit)
    }

    fn span(&self, range: std::ops::Range<usize>) -> Span {
        Span::new(self.offset(range.start), self.offset(range.end))
    }
}
