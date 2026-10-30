//! [`AnchorIndex`]: a document's link targets.

use std::collections::HashMap;

use crate::ast::NodeId;

/// The anchors a link's `#fragment` can target in one document: one per heading, in
/// document order, as given by [`heading_anchors`](crate::flavor::heading_anchors) with
/// the document flavor's slugger.
///
/// Only headings are indexed. Footnote definitions have no anchor in Markdown itself (the
/// renderer names them), and HTML `id`/`name` attributes would need an HTML scan.
///
/// Lookups compare exactly, or case-insensitively (by Unicode lowercase) for flavors
/// with [`case_insensitive_anchors`](crate::flavor::Flavor::case_insensitive_anchors).
/// Explicit ids may repeat; a lookup finds the first heading with the anchor.
#[derive(Debug, Clone)]
pub struct AnchorIndex {
    anchors: Vec<(NodeId, String)>,
    /// Lookup key → index of the first anchor with it.
    first: HashMap<String, usize>,
    case_insensitive: bool,
}

impl AnchorIndex {
    /// Indexes `anchors` (heading, anchor) in document order.
    pub fn new(anchors: Vec<(NodeId, String)>, case_insensitive: bool) -> Self {
        let mut index = Self {
            anchors: Vec::new(),
            first: HashMap::with_capacity(anchors.len()),
            case_insensitive,
        };
        for (i, (_, anchor)) in anchors.iter().enumerate() {
            index.first.entry(index.key(anchor)).or_insert(i);
        }
        index.anchors = anchors;
        index
    }

    fn key(&self, anchor: &str) -> String {
        if self.case_insensitive {
            anchor.to_lowercase()
        } else {
            anchor.to_owned()
        }
    }

    /// The heading `anchor` (a fragment, without `#`) targets.
    pub fn get(&self, anchor: &str) -> Option<NodeId> {
        self.first
            .get(&self.key(anchor))
            .map(|&i| self.anchors[i].0)
    }

    /// Whether some heading has `anchor`.
    pub fn contains(&self, anchor: &str) -> bool {
        self.get(anchor).is_some()
    }

    /// The anchor of `heading`, as the slugger made it.
    pub fn anchor_of(&self, heading: NodeId) -> Option<&str> {
        self.anchors
            .iter()
            .find(|(id, _)| *id == heading)
            .map(|(_, anchor)| anchor.as_str())
    }

    /// Every (heading, anchor) in document order, duplicates included.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (NodeId, &str)> {
        self.anchors
            .iter()
            .map(|(id, anchor)| (*id, anchor.as_str()))
    }

    /// Number of headings.
    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    /// Whether lookups ignore case.
    pub const fn is_case_insensitive(&self) -> bool {
        self.case_insensitive
    }
}
