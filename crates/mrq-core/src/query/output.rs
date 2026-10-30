//! JSON results of a query ([`JsonOutput`]) and the JSON shape of a [`Section`].

use serde_json::{Value, json};

use super::engine::Match;
use super::section::{Section, sections};
use crate::Document;
use crate::ast::json::{position, project_node, type_name};
use crate::ast::{Node, Span};

/// One JSON result of a query: a jq output, or a [`Match`] projected to JSON.
///
/// Projected nodes have the shape of the [JSON projection](crate::ast::json). Sections
/// are objects of type `section`:
///
/// ```json
/// { "type": "section", "depth": 2, "heading": { "type": "heading", … },
///   "children": [ … ], "position": { "start": { … }, "end": { … } } }
/// ```
///
/// `depth` is the heading level, `children` the content blocks after the heading
/// (subsection headings included) and `position` spans the heading and the content, as
/// [`Section::span`]. A jq program may output anything, so [`span`](Self::span) and
/// [`to_match`](Self::to_match) recover the source only when the value still is a node
/// or section of the document.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonOutput {
    value: Value,
}

impl JsonOutput {
    /// An output holding `value`.
    pub const fn new(value: Value) -> Self {
        Self { value }
    }

    /// The JSON projection of a match of `doc`: the node, or the section object.
    pub fn from_match(doc: &Document, found: &Match<'_>) -> Self {
        match found {
            Match::Node(node) => Self::new(node_json(doc, *node)),
            Match::Section(section) => Self::new(section_json(doc, *section)),
        }
    }

    /// The JSON value.
    pub const fn value(&self) -> &Value {
        &self.value
    }

    /// The JSON value, by value.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// The byte range of `position.start.offset`..`position.end.offset`, when the value is
    /// an object with such a position (a node or section, also one a program edited).
    pub fn span(&self) -> Option<Span> {
        let offset = |pointer| {
            self.value
                .pointer(pointer)
                .and_then(Value::as_u64)
                .and_then(|offset| usize::try_from(offset).ok())
        };
        let (start, end) = (
            offset("/position/start/offset")?,
            offset("/position/end/offset")?,
        );
        (start <= end).then(|| Span::new(start, end))
    }

    /// The node or section of `doc` this value is the projection of: a node with the same
    /// `id`, `type` and span, or a section whose heading has the `id` of `heading.id`
    /// and the same span. `None` for other values.
    pub fn to_match<'a>(&self, doc: &'a Document) -> Option<Match<'a>> {
        let span = self.span()?;
        let id = |value: &Value| value.get("id")?.as_u64();
        if self.value.get("type")? == "section" {
            let heading = id(self.value.get("heading")?)?;
            return sections(doc.ast())
                .into_iter()
                .find(|s| s.heading().id().index() as u64 == heading && s.span() == span)
                .map(Match::Section);
        }
        let node = node_by_index(doc, id(&self.value)?)?;
        (node.span() == span && self.value["type"] == type_name(node.node_type()))
            .then_some(Match::Node(node))
    }
}

/// The node of `doc`'s tree whose [`NodeId::index`](crate::ast::NodeId::index) is
/// `index`, if it is attached.
fn node_by_index(doc: &Document, index: u64) -> Option<Node<'_>> {
    let ast = doc.ast();
    ast.descendants(ast.root())
        .find(|id| id.index() as u64 == index)
        .map(|id| ast.node(id))
}

/// The projection of `node`.
pub(super) fn node_json(doc: &Document, node: Node<'_>) -> Value {
    serde_json::to_value(project_node(doc, node.id())).expect("the projection serializes")
}

/// The `section` object of `section`; see [`JsonOutput`].
pub(super) fn section_json(doc: &Document, section: Section<'_>) -> Value {
    let children: Vec<Value> = section.content().map(|node| node_json(doc, node)).collect();
    json!({
        "type": "section",
        "depth": section.level(),
        "heading": node_json(doc, section.heading()),
        "children": children,
        "position": position(doc.source(), section.span()),
    })
}
