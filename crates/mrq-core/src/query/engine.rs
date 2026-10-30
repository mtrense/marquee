//! Evaluation of a [`QueryPlan`] over a document: the [`Match`]es.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;

use super::plan::{Attribute, Predicate, QueryPlan, Selector};
use super::section::{Section, SectionIndex};
use crate::Document;
use crate::ast::{Ast, FrontMatterFormat, ListItem, ListKind, Node, NodeId, NodeKind, Span};

/// One result of a query: a node or a [`Section`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Match<'a> {
    /// A node of the tree.
    Node(Node<'a>),
    /// A heading with its content.
    Section(Section<'a>),
}

impl<'a> Match<'a> {
    /// The matched node, if this is not a section.
    pub const fn node(&self) -> Option<Node<'a>> {
        match self {
            Self::Node(node) => Some(*node),
            Self::Section(_) => None,
        }
    }

    /// The matched section, if this is one.
    pub const fn section(&self) -> Option<Section<'a>> {
        match self {
            Self::Node(_) => None,
            Self::Section(section) => Some(*section),
        }
    }

    /// Whether this is a section.
    pub const fn is_section(&self) -> bool {
        matches!(self, Self::Section(_))
    }

    /// The source span; a section spans its heading and all its content.
    pub fn span(&self) -> Span {
        match self {
            Self::Node(node) => node.span(),
            Self::Section(section) => section.span(),
        }
    }

    /// The source text, `&src[self.span()]`; whole sections are printed in full.
    ///
    /// # Panics
    ///
    /// If the span does not fit `src` (i.e. `src` is not the text the tree was built from).
    pub fn source<'s>(&self, src: &'s str) -> &'s str {
        let span = self.span();
        &src[span.start..span.end]
    }

    /// The plain text; see [`Node::text`] and [`Section::text`].
    pub fn text(&self) -> String {
        match self {
            Self::Node(node) => node.text(),
            Self::Section(section) => section.text(),
        }
    }
}

impl fmt::Debug for Match<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(node) => node.fmt(f),
            Self::Section(section) => section.fmt(f),
        }
    }
}

impl QueryPlan {
    /// The plan's matches in `doc`, in document order without duplicates.
    pub fn evaluate<'a>(&self, doc: &'a Document) -> Vec<Match<'a>> {
        self.evaluate_ast(doc.ast())
    }

    /// The plan's matches in a bare tree; see [`evaluate`](Self::evaluate).
    pub fn evaluate_ast<'a>(&self, ast: &'a Ast) -> Vec<Match<'a>> {
        let engine = Engine::new(ast);
        engine
            .eval(self, Item::Node(ast.root()), false)
            .into_iter()
            .map(|item| match item {
                Item::Node(id) => Match::Node(ast.node(id)),
                Item::Section(s) => Match::Section(engine.sections.sections[s]),
            })
            .collect()
    }
}

/// An intermediate result: a node id or an index into the section index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Item {
    Node(NodeId),
    Section(usize),
}

struct Engine<'a> {
    ast: &'a Ast,
    sections: SectionIndex<'a>,
    /// Per node index: its position in document (pre-)order.
    order: Vec<usize>,
    /// The whole-document results of the plans of [`Predicate::Is`], by address.
    members: RefCell<HashMap<*const QueryPlan, HashSet<Item>>>,
}

impl<'a> Engine<'a> {
    fn new(ast: &'a Ast) -> Self {
        let mut order = vec![0; ast.node_count()];
        for (position, id) in ast.descendants(ast.root()).enumerate() {
            order[id.index()] = position;
        }
        Self {
            ast,
            sections: SectionIndex::new(ast),
            order,
            members: RefCell::default(),
        }
    }

    /// The results of `plan` inside `context`; `strict` leaves the context itself out.
    fn eval(&self, plan: &QueryPlan, context: Item, strict: bool) -> Vec<Item> {
        let mut items = match plan {
            QueryPlan::Select(selector) => self
                .nodes_in(context, strict)
                .into_iter()
                .filter(|&id| self.selects(selector, Item::Node(id)))
                .map(Item::Node)
                .collect(),
            QueryPlan::Section(heading) => self
                .sections_in(context, strict)
                .into_iter()
                .filter(|&s| {
                    heading.matches(&self.ast.text(self.sections.sections[s].heading().id()))
                })
                .map(Item::Section)
                .collect(),
            QueryPlan::Filter(plan, predicates) => self
                .eval(plan, context, strict)
                .into_iter()
                .filter(|&item| predicates.iter().all(|p| self.satisfies(p, item)))
                .collect(),
            QueryPlan::Pipe(outer, inner) => self.nested(outer, inner, context, strict, false),
            QueryPlan::Descendant(outer, inner) => self.nested(outer, inner, context, strict, true),
            QueryPlan::Child(outer, inner) => self
                .eval(outer, context, strict)
                .into_iter()
                .flat_map(|parent| {
                    self.eval(inner, parent, true)
                        .into_iter()
                        .filter(move |&item| self.parent(item) == Some(parent))
                })
                .collect(),
            QueryPlan::Union(plans) => plans
                .iter()
                .flat_map(|plan| self.eval(plan, context, strict))
                .collect(),
            QueryPlan::Scope => vec![context],
        };
        items.sort_by_key(|&item| self.order_key(item));
        items.dedup();
        items
    }

    /// `inner` within each result of `outer`.
    fn nested(
        &self,
        outer: &QueryPlan,
        inner: &QueryPlan,
        context: Item,
        strict: bool,
        inner_strict: bool,
    ) -> Vec<Item> {
        self.eval(outer, context, strict)
            .into_iter()
            .flat_map(|item| self.eval(inner, item, inner_strict))
            .collect()
    }

    /// Document order, a section right before its heading.
    fn order_key(&self, item: Item) -> (usize, bool) {
        match item {
            Item::Node(id) => (self.order[id.index()], true),
            Item::Section(s) => (self.order[self.heading(s).index()], false),
        }
    }

    fn heading(&self, section: usize) -> NodeId {
        self.sections.sections[section].heading().id()
    }

    /// The nodes inside `context`, in document order.
    fn nodes_in(&self, context: Item, strict: bool) -> Vec<NodeId> {
        match context {
            Item::Node(id) => self.ast.descendants(id).skip(usize::from(strict)).collect(),
            Item::Section(s) => self.sections.sections[s]
                .nodes()
                .flat_map(|node| self.ast.descendants(node.id()))
                .collect(),
        }
    }

    /// The sections inside `context`, in document order: those whose heading is inside
    /// it, except the section of a context heading.
    fn sections_in(&self, context: Item, strict: bool) -> Vec<usize> {
        self.nodes_in(context, false)
            .into_iter()
            .filter_map(|id| self.sections.of_heading(id))
            .filter(|&s| match context {
                Item::Node(id) => self.heading(s) != id,
                Item::Section(own) => !(strict && s == own),
            })
            .collect()
    }

    /// The parent in the combined tree of nodes and sections; see [`QueryPlan::Child`].
    fn parent(&self, item: Item) -> Option<Item> {
        match item {
            Item::Node(id) => self
                .sections
                .owner(id)
                .map(Item::Section)
                .or_else(|| self.ast.parent(id).map(Item::Node)),
            Item::Section(s) => self.sections.parents[s]
                .map(Item::Section)
                .or_else(|| self.ast.parent(self.heading(s)).map(Item::Node)),
        }
    }

    fn selects(&self, selector: &Selector, item: Item) -> bool {
        let type_ok = match (selector.node_type, item) {
            (None, _) => true,
            (Some(ty), Item::Node(id)) => self.ast.kind(id).node_type() == ty,
            (Some(_), Item::Section(_)) => false,
        };
        type_ok && selector.predicates.iter().all(|p| self.satisfies(p, item))
    }

    fn satisfies(&self, predicate: &Predicate, item: Item) -> bool {
        // Kind-level predicates look at a section's heading.
        let node = match item {
            Item::Node(id) => id,
            Item::Section(s) => self.heading(s),
        };
        let kind = self.ast.kind(node);
        match predicate {
            Predicate::Text(matcher) => matcher.matches(&match item {
                Item::Node(id) => self.ast.text(id),
                Item::Section(s) => self.sections.sections[s].text(),
            }),
            Predicate::Attr(attr, matcher) => attribute_values(*attr, kind)
                .iter()
                .any(|value| matcher.matches(value)),
            Predicate::Level(level) => {
                matches!(kind, NodeKind::Heading(heading) if heading.level == *level)
            }
            Predicate::Task(state) => match kind {
                NodeKind::ListItem(ListItem { task: Some(task) }) => {
                    state.is_none_or(|state| state == *task)
                }
                _ => false,
            },
            Predicate::Ordered(ordered) => {
                let list = match kind {
                    NodeKind::ListItem(_) => self.ast.parent(node).map(|p| self.ast.kind(p)),
                    _ => Some(kind),
                };
                matches!(list, Some(NodeKind::List(list))
                    if matches!(list.kind, ListKind::Ordered { .. }) == *ordered)
            }
            Predicate::Heading(matcher) => match item {
                Item::Section(_) => matcher.matches(&self.ast.text(node)),
                Item::Node(id) => {
                    matches!(kind, NodeKind::Heading(_)) && matcher.matches(&self.ast.text(id))
                }
            },
            Predicate::Has(plan) => !self.eval(plan, item, true).is_empty(),
            Predicate::Is(plan) => self.is_member(plan, item),
            Predicate::Not(selector) => !self.selects(selector, item),
            Predicate::NthChild { a, b } => self
                .ast
                .index_in_parent(node)
                .and_then(|index| i64::try_from(index + 1).ok())
                .is_some_and(|n| nth(*a, *b, n)),
        }
    }
}

impl Engine<'_> {
    /// Whether `item` is a result of `plan` over the whole document; the results are
    /// computed once per plan.
    fn is_member(&self, plan: &QueryPlan, item: Item) -> bool {
        let key: *const QueryPlan = plan;
        if let Some(members) = self.members.borrow().get(&key) {
            return members.contains(&item);
        }
        // Not holding the borrow: the plan may contain `Is` predicates of its own.
        let members: HashSet<Item> = self
            .eval(plan, Item::Node(self.ast.root()), false)
            .into_iter()
            .collect();
        let found = members.contains(&item);
        self.members.borrow_mut().insert(key, members);
        found
    }
}

/// Whether `n = a·k + b` for some `k ≥ 0`.
fn nth(a: i64, b: i64, n: i64) -> bool {
    let diff = n - b;
    if a == 0 {
        diff == 0
    } else {
        diff % a == 0 && diff / a >= 0
    }
}

/// The values of `attr` on a node of this kind; empty when it has none.
fn attribute_values(attr: Attribute, kind: &NodeKind) -> Vec<String> {
    let one = |value: &str| vec![value.to_owned()];
    let opt = |value: Option<&str>| value.map(str::to_owned).into_iter().collect();
    match (attr, kind) {
        (Attribute::Lang, NodeKind::CodeBlock(code)) => opt(code.lang()),
        (Attribute::Info, NodeKind::CodeBlock(code)) => one(&code.info),
        (Attribute::Url, NodeKind::Link(link) | NodeKind::Image(link)) => one(&link.url),
        (Attribute::Url, NodeKind::LinkDefinition(def)) => one(&def.url),
        (Attribute::Title, NodeKind::Link(link) | NodeKind::Image(link)) => {
            opt(link.title.as_deref())
        }
        (Attribute::Title, NodeKind::LinkDefinition(def)) => opt(def.title.as_deref()),
        (Attribute::Title, NodeKind::Admonition(admonition)) => opt(admonition.title.as_deref()),
        (Attribute::Title, NodeKind::Callout(callout)) => opt(callout.title.as_deref()),
        (Attribute::Level, NodeKind::Heading(heading)) => one(&heading.level.to_string()),
        (Attribute::Kind, NodeKind::Admonition(admonition)) => one(&admonition.kind),
        (Attribute::Kind, NodeKind::Callout(callout)) => one(&callout.kind),
        (Attribute::Target, NodeKind::WikiLink(link) | NodeKind::Embed(link)) => one(&link.target),
        (Attribute::Anchor, NodeKind::WikiLink(link) | NodeKind::Embed(link)) => {
            opt(link.anchor.as_deref())
        }
        (Attribute::Alias, NodeKind::WikiLink(link) | NodeKind::Embed(link)) => {
            opt(link.alias.as_deref())
        }
        (Attribute::Label, NodeKind::FootnoteDefinition(def)) => one(&def.label),
        (Attribute::Label, NodeKind::FootnoteReference(reference)) => one(&reference.label),
        (Attribute::Label, NodeKind::LinkDefinition(def)) => one(&def.label),
        (Attribute::Name, NodeKind::Tag(tag)) => one(&tag.name),
        (Attribute::Id, NodeKind::Heading(heading)) => {
            opt(heading.attrs.as_ref().and_then(|attrs| attrs.id.as_deref()))
        }
        (Attribute::Class, NodeKind::Heading(heading)) => heading
            .attrs
            .as_ref()
            .map(|attrs| attrs.classes.clone())
            .unwrap_or_default(),
        (Attribute::Class, NodeKind::Admonition(admonition)) => admonition.classes.clone(),
        (Attribute::Format, NodeKind::FrontMatter(fm)) => one(match fm.format {
            FrontMatterFormat::Yaml => "yaml",
            FrontMatterFormat::Toml => "toml",
        }),
        (Attribute::Raw, NodeKind::HtmlBlock(html) | NodeKind::HtmlInline(html)) => one(html),
        (Attribute::Raw, NodeKind::Comment(comment)) => one(&comment.literal),
        (Attribute::Raw, NodeKind::FrontMatter(fm)) => one(&fm.literal),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::nth;

    #[test]
    fn nth_child_formula() {
        let hits = |a, b| (1..=8).filter(|&n| nth(a, b, n)).collect::<Vec<_>>();
        assert_eq!(hits(2, 1), [1, 3, 5, 7]);
        assert_eq!(hits(0, 4), [4]);
        assert_eq!(hits(3, -1), [2, 5, 8]);
        assert_eq!(hits(-2, 5), [1, 3, 5]);
        assert_eq!(hits(1, 6), [6, 7, 8]);
    }
}
