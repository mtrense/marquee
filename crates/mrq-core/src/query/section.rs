//! Sections: a heading plus the blocks after it up to the next heading of the same or a
//! higher level ([`Section`], [`sections`]).

use std::fmt;

use crate::ast::{Ast, Node, NodeId, NodeKind, Span};

/// A heading and its content: the sibling blocks after it, up to (excluding) the next
/// sibling heading of the same or a higher level (a smaller or equal number), or the end
/// of the container.
///
/// Sections nest: the content of `# A` includes `## B` and everything in B's section.
/// They never leave their container: a heading inside a block quote or list item starts
/// a section that ends with that container. Like [`Node`], a section is a `Copy` handle
/// into an [`Ast`].
#[derive(Clone, Copy)]
pub struct Section<'a> {
    ast: &'a Ast,
    heading: NodeId,
    level: u8,
    /// Index range of the content in the heading's parent's children.
    content: (usize, usize),
    span: Span,
}

impl<'a> Section<'a> {
    /// The heading node.
    pub fn heading(self) -> Node<'a> {
        self.ast.node(self.heading)
    }

    /// The heading level, 1–6.
    pub const fn level(self) -> u8 {
        self.level
    }

    /// From the start of the heading to the end of the last content block (the heading's
    /// end if there is no content).
    pub const fn span(self) -> Span {
        self.span
    }

    /// The source text of the whole section: `&src[self.span()]`.
    ///
    /// # Panics
    ///
    /// If the span does not fit `src` (i.e. `src` is not the text the tree was built from).
    pub fn source(self, src: &str) -> &str {
        &src[self.span.start..self.span.end]
    }

    /// The content blocks after the heading, subsection headings included.
    pub fn content(self) -> impl DoubleEndedIterator<Item = Node<'a>> + ExactSizeIterator {
        let ast = self.ast;
        let (start, end) = self.content;
        self.siblings()[start..end]
            .iter()
            .map(move |&id| ast.node(id))
    }

    /// The heading followed by the content blocks.
    pub fn nodes(self) -> impl DoubleEndedIterator<Item = Node<'a>> + ExactSizeIterator {
        let ast = self.ast;
        let (start, end) = self.content;
        self.siblings()[start - 1..end]
            .iter()
            .map(move |&id| ast.node(id))
    }

    /// The plain text of the heading and every content block, one block per line; see
    /// [`Ast::text`].
    pub fn text(self) -> String {
        let texts: Vec<String> = self
            .nodes()
            .map(Node::text)
            .filter(|text| !text.is_empty())
            .collect();
        texts.join("\n")
    }

    fn siblings(self) -> &'a [NodeId] {
        let parent = self
            .ast
            .parent(self.heading)
            .expect("a section heading has a parent");
        self.ast.children(parent)
    }
}

impl PartialEq for Section<'_> {
    /// Sections are equal when they have the same heading in the same tree.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.ast, other.ast) && self.heading == other.heading
    }
}

impl Eq for Section<'_> {}

impl fmt::Debug for Section<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "section h{} {} {}..{}",
            self.level, self.heading, self.span.start, self.span.end
        )
    }
}

/// Every section of the tree, in document order of their headings.
///
/// ```
/// use mrq_core::{Document, query};
///
/// let doc = Document::parse("# A\n\ntext\n\n## B\n\n# C\n");
/// let sections = query::sections(doc.ast());
/// let outline: Vec<_> = sections
///     .iter()
///     .map(|s| (s.level(), s.heading().text(), s.source(doc.text())))
///     .collect();
/// assert_eq!(
///     outline,
///     [
///         (1, "A".to_owned(), "# A\n\ntext\n\n## B"),
///         (2, "B".to_owned(), "## B"),
///         (1, "C".to_owned(), "# C"),
///     ]
/// );
/// ```
pub fn sections(ast: &Ast) -> Vec<Section<'_>> {
    SectionIndex::new(ast).sections
}

/// The sections of a tree with their nesting, for the engine.
pub(super) struct SectionIndex<'a> {
    /// In document order.
    pub sections: Vec<Section<'a>>,
    /// Per section: the enclosing section's index.
    pub parents: Vec<Option<usize>>,
    /// Per node index: the section whose heading it is.
    by_heading: Vec<Option<usize>>,
    /// Per node index: the innermost section that has the node as its heading or as a
    /// direct content block.
    owners: Vec<Option<usize>>,
}

impl<'a> SectionIndex<'a> {
    pub fn new(ast: &'a Ast) -> Self {
        // Sections are found container by container, keyed by heading, then sorted.
        let mut found: Vec<Section<'a>> = Vec::new();
        let mut parents = Vec::new();
        let mut owners = vec![None; ast.node_count()];
        for container in ast.descendants(ast.root()) {
            let children = ast.children(container);
            // Indices into `found` of the open sections, outermost first.
            let mut open: Vec<usize> = Vec::new();
            for (index, &child) in children.iter().enumerate() {
                if let NodeKind::Heading(heading) = ast.kind(child) {
                    while let Some(&s) = open.last()
                        && found[s].level >= heading.level
                    {
                        open.pop();
                        close(ast, &mut found[s], index);
                    }
                    parents.push(open.last().map(|&s| found[s].heading));
                    open.push(found.len());
                    found.push(Section {
                        ast,
                        heading: child,
                        level: heading.level,
                        content: (index + 1, index + 1),
                        span: ast.span(child),
                    });
                }
                owners[child.index()] = open.last().map(|&s| found[s].heading);
            }
            for s in open {
                close(ast, &mut found[s], children.len());
            }
        }

        let mut order = vec![0; ast.node_count()];
        for (position, id) in ast.descendants(ast.root()).enumerate() {
            order[id.index()] = position;
        }
        let mut sorted: Vec<(Section<'a>, Option<NodeId>)> =
            found.into_iter().zip(parents).collect();
        sorted.sort_by_key(|(section, _)| order[section.heading.index()]);
        let mut by_heading = vec![None; ast.node_count()];
        for (index, (section, _)) in sorted.iter().enumerate() {
            by_heading[section.heading.index()] = Some(index);
        }
        let lookup = |heading: Option<NodeId>| heading.and_then(|h| by_heading[h.index()]);
        let parents = sorted.iter().map(|&(_, parent)| lookup(parent)).collect();
        let owners = owners.into_iter().map(lookup).collect();
        Self {
            sections: sorted.into_iter().map(|(section, _)| section).collect(),
            parents,
            by_heading,
            owners,
        }
    }

    /// The index of the section whose heading is `id`.
    pub fn of_heading(&self, id: NodeId) -> Option<usize> {
        self.by_heading[id.index()]
    }

    /// The index of the innermost section that has `id` as its heading or as a direct
    /// content block.
    pub fn owner(&self, id: NodeId) -> Option<usize> {
        self.owners[id.index()]
    }
}

/// Ends `section`'s content before the sibling at `end`.
fn close(ast: &Ast, section: &mut Section<'_>, end: usize) {
    section.content.1 = end;
    if end > section.content.0 {
        let siblings = section.siblings();
        section.span.end = ast.span(siblings[end - 1]).end;
    }
}
