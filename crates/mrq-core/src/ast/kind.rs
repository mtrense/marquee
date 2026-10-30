//! Node kinds: [`NodeKind`] with its payload structs, and the payload-free [`NodeType`].

use std::fmt;

use super::Span;

/// What a node is, plus the data specific to that kind.
///
/// Covers `CommonMark`, GFM and every flavor extension mrq knows about. Structure (children,
/// parent) and the source [`Span`] live in the [`Ast`](super::Ast), not here, so a kind can
/// be swapped in place by a flavor pass without touching the tree.
///
/// # Children
///
/// The doc comment of each variant says what its children are. "Inline" children are
/// [`NodeType::is_inline`] nodes; "block" children are block nodes. Leaves have none.
/// [`Math`](Self::Math) and [`Comment`](Self::Comment) are inline but may also appear as
/// a direct child of a block container when a flavor pass recognizes them at block level.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeKind {
    // --- Blocks -----------------------------------------------------------------------
    /// The root of a document. Children: blocks.
    Document,
    /// YAML or TOML frontmatter at the start of a document. Leaf.
    FrontMatter(FrontMatter),
    /// Children: inlines.
    Paragraph,
    /// ATX or setext heading. Children: inlines.
    Heading(Heading),
    /// `---`, `***` or `___`. Leaf.
    ThematicBreak,
    /// `> …`. Children: blocks.
    BlockQuote,
    /// Bullet or ordered list. Children: [`ListItem`](Self::ListItem)s.
    List(List),
    /// A list item, possibly a GFM task. Children: blocks.
    ListItem(ListItem),
    /// Fenced or indented code block. Leaf.
    CodeBlock(CodeBlock),
    /// Raw HTML block (`CommonMark` types 1–7, except a lone comment, see
    /// [`Comment`](Self::Comment)). Leaf; the payload is the literal HTML.
    HtmlBlock(String),
    /// GFM table. Children: [`TableRow`](Self::TableRow)s, the header row first.
    Table(Table),
    /// Children: [`TableCell`](Self::TableCell)s.
    TableRow(TableRow),
    /// Children: inlines.
    TableCell,
    /// `[^label]: …`. Children: blocks.
    FootnoteDefinition(FootnoteDefinition),
    /// Link reference definition `[label]: url "title"`. Leaf.
    LinkDefinition(LinkDefinition),
    /// mkdocs/PHP-Markdown-Extra definition list. Children:
    /// [`DefinitionItem`](Self::DefinitionItem)s.
    DefinitionList,
    /// One term with its details. Children: a [`DefinitionTerm`](Self::DefinitionTerm)
    /// followed by one or more [`DefinitionDetails`](Self::DefinitionDetails).
    DefinitionItem,
    /// Children: blocks (usually one paragraph).
    DefinitionTerm,
    /// `: …`. Children: blocks.
    DefinitionDetails,
    /// mkdocs admonition `!!! note "Title"` / collapsible `??? note`. Children: blocks
    /// (the indented body).
    Admonition(Admonition),
    /// GFM alert `> [!NOTE]` or Obsidian callout `> [!note]+ Title`. Children: blocks (the
    /// quote content without the marker line).
    Callout(Callout),
    /// abbreviation definition `*[HTML]: Hyper Text Markup Language` (mkdocs). Leaf.
    AbbreviationDefinition(AbbreviationDefinition),
    /// `<!-- mrq:include … -->`. Leaf; the included region is delimited by a following
    /// [`DirectiveEnd`](Self::DirectiveEnd) sibling.
    IncludeDirective(Directive),
    /// `<!-- mrq:toc … -->`. Leaf; region as for [`IncludeDirective`](Self::IncludeDirective).
    TocDirective(Directive),
    /// `<!-- mrq:end -->`. Leaf.
    DirectiveEnd,

    // --- Inlines ----------------------------------------------------------------------
    /// Literal text (entities and escapes already resolved). Leaf.
    Text(String),
    /// A line ending inside a paragraph that is not a hard break. Leaf.
    SoftBreak,
    /// Hard line break (two trailing spaces or a backslash). Leaf.
    LineBreak,
    /// Code span; the payload is its content after `CommonMark` normalization. Leaf.
    Code(String),
    /// `*…*` / `_…_`. Children: inlines.
    Emphasis,
    /// `**…**` / `__…__`. Children: inlines.
    Strong,
    /// GFM `~~…~~`. Children: inlines.
    Strikethrough,
    /// `==…==` (Obsidian, mkdocs `mark`). Children: inlines.
    Highlight,
    /// `^…^`. Children: inlines.
    Superscript,
    /// `~…~`. Children: inlines.
    Subscript,
    /// Children: inlines (the link text).
    Link(Link),
    /// Children: inlines (the alt text).
    Image(Link),
    /// Raw inline HTML (except a comment, see [`Comment`](Self::Comment)). Leaf.
    HtmlInline(String),
    /// `[^label]`. Leaf.
    FootnoteReference(FootnoteReference),
    /// Obsidian `[[target#anchor|alias]]`. Leaf.
    WikiLink(WikiLink),
    /// Obsidian `![[target#anchor|alias]]`. Leaf.
    Embed(WikiLink),
    /// Obsidian `#tag`. Leaf.
    Tag(Tag),
    /// `$…$` or `$$…$$`. Leaf.
    Math(Math),
    /// HTML `<!-- … -->` or Obsidian `%%…%%` comment. Leaf.
    Comment(Comment),
}

impl NodeKind {
    /// The payload-free type of this kind.
    pub const fn node_type(&self) -> NodeType {
        match self {
            Self::Document => NodeType::Document,
            Self::FrontMatter(_) => NodeType::FrontMatter,
            Self::Paragraph => NodeType::Paragraph,
            Self::Heading(_) => NodeType::Heading,
            Self::ThematicBreak => NodeType::ThematicBreak,
            Self::BlockQuote => NodeType::BlockQuote,
            Self::List(_) => NodeType::List,
            Self::ListItem(_) => NodeType::ListItem,
            Self::CodeBlock(_) => NodeType::CodeBlock,
            Self::HtmlBlock(_) => NodeType::HtmlBlock,
            Self::Table(_) => NodeType::Table,
            Self::TableRow(_) => NodeType::TableRow,
            Self::TableCell => NodeType::TableCell,
            Self::FootnoteDefinition(_) => NodeType::FootnoteDefinition,
            Self::LinkDefinition(_) => NodeType::LinkDefinition,
            Self::DefinitionList => NodeType::DefinitionList,
            Self::DefinitionItem => NodeType::DefinitionItem,
            Self::DefinitionTerm => NodeType::DefinitionTerm,
            Self::DefinitionDetails => NodeType::DefinitionDetails,
            Self::Admonition(_) => NodeType::Admonition,
            Self::Callout(_) => NodeType::Callout,
            Self::AbbreviationDefinition(_) => NodeType::AbbreviationDefinition,
            Self::IncludeDirective(_) => NodeType::IncludeDirective,
            Self::TocDirective(_) => NodeType::TocDirective,
            Self::DirectiveEnd => NodeType::DirectiveEnd,
            Self::Text(_) => NodeType::Text,
            Self::SoftBreak => NodeType::SoftBreak,
            Self::LineBreak => NodeType::LineBreak,
            Self::Code(_) => NodeType::Code,
            Self::Emphasis => NodeType::Emphasis,
            Self::Strong => NodeType::Strong,
            Self::Strikethrough => NodeType::Strikethrough,
            Self::Highlight => NodeType::Highlight,
            Self::Superscript => NodeType::Superscript,
            Self::Subscript => NodeType::Subscript,
            Self::Link(_) => NodeType::Link,
            Self::Image(_) => NodeType::Image,
            Self::HtmlInline(_) => NodeType::HtmlInline,
            Self::FootnoteReference(_) => NodeType::FootnoteReference,
            Self::WikiLink(_) => NodeType::WikiLink,
            Self::Embed(_) => NodeType::Embed,
            Self::Tag(_) => NodeType::Tag,
            Self::Math(_) => NodeType::Math,
            Self::Comment(_) => NodeType::Comment,
        }
    }

    /// Applies `map` to every span in the payload: a heading's
    /// [`HeadingAttrs::span`] and each [`DirectiveAttribute::span`]. The node's own span
    /// lives in the [`Ast`](super::Ast); [`Ast::import`](super::Ast::import) maps both.
    pub fn map_spans(&mut self, mut map: impl FnMut(Span) -> Span) {
        match self {
            Self::Heading(Heading {
                attrs: Some(attrs), ..
            }) => attrs.span = map(attrs.span),
            Self::IncludeDirective(directive) | Self::TocDirective(directive) => {
                for attr in &mut directive.attributes {
                    attr.span = map(attr.span);
                }
            }
            _ => {}
        }
    }

    /// Shorthand for `self.node_type().is_block()`.
    pub const fn is_block(&self) -> bool {
        self.node_type().is_block()
    }

    /// Shorthand for `self.node_type().is_inline()`.
    pub const fn is_inline(&self) -> bool {
        self.node_type().is_inline()
    }
}

macro_rules! node_types {
    ($( $(#[$doc:meta])* $variant:ident = $name:literal, $block:literal; )*) => {
        /// The type of a node without its payload: cheap to copy, compare and hash.
        ///
        /// Used wherever only the kind matters, e.g. query plans (`Select(type, …)`) and
        /// CSS type selectors. [`NodeType::as_str`] gives a stable `snake_case` name.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum NodeType {
            $( $(#[$doc])* $variant, )*
        }

        impl NodeType {
            /// Every node type, in declaration order (blocks first).
            pub const ALL: &'static [NodeType] = &[$(NodeType::$variant),*];

            /// Stable `snake_case` name, e.g. `"code_block"`.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $name, )*
                }
            }

            /// Whether nodes of this type are block-level (containers or leaves that
            /// occupy whole lines). Every type is either block or inline.
            pub const fn is_block(self) -> bool {
                match self {
                    $( Self::$variant => $block, )*
                }
            }

            /// Whether nodes of this type are inline (`!self.is_block()`).
            pub const fn is_inline(self) -> bool {
                !self.is_block()
            }

            /// Looks a type up by its [`as_str`](Self::as_str) name.
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $( $name => Some(Self::$variant), )*
                    _ => None,
                }
            }
        }
    };
}

node_types! {
    /// [`NodeKind::Document`]
    Document = "document", true;
    /// [`NodeKind::FrontMatter`]
    FrontMatter = "front_matter", true;
    /// [`NodeKind::Paragraph`]
    Paragraph = "paragraph", true;
    /// [`NodeKind::Heading`]
    Heading = "heading", true;
    /// [`NodeKind::ThematicBreak`]
    ThematicBreak = "thematic_break", true;
    /// [`NodeKind::BlockQuote`]
    BlockQuote = "block_quote", true;
    /// [`NodeKind::List`]
    List = "list", true;
    /// [`NodeKind::ListItem`]
    ListItem = "list_item", true;
    /// [`NodeKind::CodeBlock`]
    CodeBlock = "code_block", true;
    /// [`NodeKind::HtmlBlock`]
    HtmlBlock = "html_block", true;
    /// [`NodeKind::Table`]
    Table = "table", true;
    /// [`NodeKind::TableRow`]
    TableRow = "table_row", true;
    /// [`NodeKind::TableCell`]
    TableCell = "table_cell", true;
    /// [`NodeKind::FootnoteDefinition`]
    FootnoteDefinition = "footnote_definition", true;
    /// [`NodeKind::LinkDefinition`]
    LinkDefinition = "link_definition", true;
    /// [`NodeKind::DefinitionList`]
    DefinitionList = "definition_list", true;
    /// [`NodeKind::DefinitionItem`]
    DefinitionItem = "definition_item", true;
    /// [`NodeKind::DefinitionTerm`]
    DefinitionTerm = "definition_term", true;
    /// [`NodeKind::DefinitionDetails`]
    DefinitionDetails = "definition_details", true;
    /// [`NodeKind::Admonition`]
    Admonition = "admonition", true;
    /// [`NodeKind::Callout`]
    Callout = "callout", true;
    /// [`NodeKind::AbbreviationDefinition`]
    AbbreviationDefinition = "abbreviation_definition", true;
    /// [`NodeKind::IncludeDirective`]
    IncludeDirective = "include_directive", true;
    /// [`NodeKind::TocDirective`]
    TocDirective = "toc_directive", true;
    /// [`NodeKind::DirectiveEnd`]
    DirectiveEnd = "directive_end", true;
    /// [`NodeKind::Text`]
    Text = "text", false;
    /// [`NodeKind::SoftBreak`]
    SoftBreak = "soft_break", false;
    /// [`NodeKind::LineBreak`]
    LineBreak = "line_break", false;
    /// [`NodeKind::Code`]
    Code = "code", false;
    /// [`NodeKind::Emphasis`]
    Emphasis = "emphasis", false;
    /// [`NodeKind::Strong`]
    Strong = "strong", false;
    /// [`NodeKind::Strikethrough`]
    Strikethrough = "strikethrough", false;
    /// [`NodeKind::Highlight`]
    Highlight = "highlight", false;
    /// [`NodeKind::Superscript`]
    Superscript = "superscript", false;
    /// [`NodeKind::Subscript`]
    Subscript = "subscript", false;
    /// [`NodeKind::Link`]
    Link = "link", false;
    /// [`NodeKind::Image`]
    Image = "image", false;
    /// [`NodeKind::HtmlInline`]
    HtmlInline = "html_inline", false;
    /// [`NodeKind::FootnoteReference`]
    FootnoteReference = "footnote_reference", false;
    /// [`NodeKind::WikiLink`]
    WikiLink = "wiki_link", false;
    /// [`NodeKind::Embed`]
    Embed = "embed", false;
    /// [`NodeKind::Tag`]
    Tag = "tag", false;
    /// [`NodeKind::Math`]
    Math = "math", false;
    /// [`NodeKind::Comment`]
    Comment = "comment", false;
}

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Syntax of a [`FrontMatter`] block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FrontMatterFormat {
    /// `---` delimited YAML.
    Yaml,
    /// `+++` delimited TOML.
    Toml,
}

/// Payload of [`NodeKind::FrontMatter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatter {
    /// YAML or TOML.
    pub format: FrontMatterFormat,
    /// The content between the delimiter lines, verbatim.
    pub literal: String,
}

/// Payload of [`NodeKind::Heading`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1–6.
    pub level: u8,
    /// Whether the heading is underlined (`===` / `---`) rather than `#`-prefixed.
    pub setext: bool,
    /// `{#id .class key=value}` attributes (mkdocs `attr_list`), set by a flavor pass.
    pub attrs: Option<HeadingAttrs>,
}

impl Heading {
    /// An ATX heading of `level` without attributes.
    pub const fn atx(level: u8) -> Self {
        Self {
            level,
            setext: false,
            attrs: None,
        }
    }
}

/// Attribute list of a heading, `{#id .class key=value}`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeadingAttrs {
    /// `#id`; overrides the slugger.
    pub id: Option<String>,
    /// `.class` entries in source order.
    pub classes: Vec<String>,
    /// `key=value` entries in source order.
    pub attributes: Vec<(String, String)>,
    /// Bytes of the `{…}` block, which is not part of any child [`NodeKind::Text`].
    pub span: Span,
}

/// Payload of [`NodeKind::List`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    /// Bullet or ordered, with its marker.
    pub kind: ListKind,
    /// Tight lists render items without `<p>` wrappers.
    pub tight: bool,
}

/// Marker style of a [`List`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// `-`, `*` or `+`.
    Bullet {
        /// The marker character.
        marker: char,
    },
    /// `1.` or `1)`.
    Ordered {
        /// Number of the first item.
        start: u64,
        /// `.` or `)`.
        delimiter: OrderedDelimiter,
    },
}

/// Delimiter after the number of an ordered list item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderedDelimiter {
    /// `1.`
    Period,
    /// `1)`
    Paren,
}

/// Payload of [`NodeKind::ListItem`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListItem {
    /// `Some` for a GFM task list item `[ ]` / `[x]`.
    pub task: Option<TaskState>,
}

/// State of a task list item, from the character between the brackets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// `[ ]`
    Open,
    /// `[x]` or `[X]`
    Done,
    /// Any other character (Obsidian custom states such as `[-]`, `[?]`).
    Other(char),
}

impl TaskState {
    /// Classifies the character between the brackets.
    pub const fn from_marker(marker: char) -> Self {
        match marker {
            ' ' => Self::Open,
            'x' | 'X' => Self::Done,
            other => Self::Other(other),
        }
    }
}

/// Payload of [`NodeKind::CodeBlock`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    /// The fence, or `None` for an indented code block.
    pub fence: Option<Fence>,
    /// The info string after the opening fence (trimmed), empty if none.
    pub info: String,
    /// The code, including its final newline.
    pub literal: String,
}

impl CodeBlock {
    /// The language: the first word of the info string, if any.
    pub fn lang(&self) -> Option<&str> {
        self.info.split_whitespace().next()
    }
}

/// Opening fence of a fenced code block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fence {
    /// `` ` `` or `~`.
    pub char: char,
    /// Number of fence characters (at least 3).
    pub len: usize,
}

/// Payload of [`NodeKind::Table`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    /// One entry per column, from the delimiter row.
    pub alignments: Vec<Alignment>,
}

/// Column alignment of a [`Table`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
    /// `---`
    #[default]
    None,
    /// `:--`
    Left,
    /// `:-:`
    Center,
    /// `--:`
    Right,
}

/// Payload of [`NodeKind::TableRow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableRow {
    /// Whether this is the header row.
    pub header: bool,
}

/// Payload of [`NodeKind::FootnoteDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootnoteDefinition {
    /// The label as written, without `[^` and `]`.
    pub label: String,
}

/// Payload of [`NodeKind::FootnoteReference`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootnoteReference {
    /// The label as written, without `[^` and `]`.
    pub label: String,
}

/// Payload of [`NodeKind::LinkDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkDefinition {
    /// The label as written, without brackets (not case-folded).
    pub label: String,
    /// The destination, unescaped.
    pub url: String,
    /// The title, unescaped.
    pub title: Option<String>,
}

/// Payload shared by [`NodeKind::Link`] and [`NodeKind::Image`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The destination, unescaped (resolved through the definition for references).
    pub url: String,
    /// The title, unescaped.
    pub title: Option<String>,
    /// How the destination was written.
    pub form: LinkForm,
}

impl Link {
    /// An inline link `[…](url)` without title.
    pub fn inline(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: None,
            form: LinkForm::Inline,
        }
    }
}

/// How the destination of a [`Link`] was written.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkForm {
    /// `[text](url "title")`
    Inline,
    /// `[text][label]`, `[label][]` or `[label]`.
    Reference {
        /// The label as written (not case-folded).
        label: String,
        /// Full, collapsed or shortcut.
        style: ReferenceStyle,
    },
    /// `<https://…>` or `<user@host>`.
    Autolink,
    /// A GFM extended autolink: a bare `www.…`, `https://…` or e-mail in text.
    Bare,
}

/// Style of a reference link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceStyle {
    /// `[text][label]`
    Full,
    /// `[label][]`
    Collapsed,
    /// `[label]`
    Shortcut,
}

/// Payload shared by [`NodeKind::WikiLink`] and [`NodeKind::Embed`]: `[[target#anchor|alias]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// The page or file name, possibly empty for a same-page link `[[#heading]]`.
    pub target: String,
    /// Text after `#`: a heading, or `^block-id` for a block reference.
    pub anchor: Option<String>,
    /// Text after `|`: the display text (or a size such as `100x200` for embeds).
    pub alias: Option<String>,
}

/// Payload of [`NodeKind::Tag`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// The tag without its leading `#`, e.g. `project/active`.
    pub name: String,
}

/// Payload of [`NodeKind::Math`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Math {
    /// `$$…$$` (display) rather than `$…$` (inline).
    pub display: bool,
    /// The TeX source between the delimiters.
    pub literal: String,
}

/// Payload of [`NodeKind::Comment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// HTML or Obsidian syntax.
    pub syntax: CommentSyntax,
    /// The text between the delimiters, verbatim.
    pub literal: String,
}

/// Delimiters of a [`Comment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommentSyntax {
    /// `<!-- … -->`
    Html,
    /// Obsidian `%% … %%`
    Percent,
}

/// When an [`Admonition`] or [`Callout`] is collapsible, whether it starts open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    /// mkdocs `???+`, Obsidian `[!note]+`.
    Open,
    /// mkdocs `???`, Obsidian `[!note]-`.
    Closed,
}

/// Payload of [`NodeKind::Admonition`]: `!!! kind "title"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admonition {
    /// The type keyword, e.g. `note`, `warning` (as written).
    pub kind: String,
    /// Extra CSS classes after the kind (mkdocs `!!! note inline end`).
    pub classes: Vec<String>,
    /// The quoted title; `Some("")` means an explicitly empty title (no title bar).
    pub title: Option<String>,
    /// `Some` for collapsible `???` admonitions.
    pub fold: Option<Fold>,
}

/// Payload of [`NodeKind::Callout`]: `> [!kind]± title`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callout {
    /// The type keyword as written, e.g. `NOTE` (GFM) or `tip` (Obsidian).
    pub kind: String,
    /// Title text after the marker (Obsidian only).
    pub title: Option<String>,
    /// `Some` for foldable Obsidian callouts (`+` / `-`).
    pub fold: Option<Fold>,
}

/// Payload of [`NodeKind::AbbreviationDefinition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbbreviationDefinition {
    /// The abbreviation, e.g. `HTML`.
    pub abbr: String,
    /// What it stands for.
    pub expansion: String,
}

/// Payload of [`NodeKind::IncludeDirective`] and [`NodeKind::TocDirective`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Directive {
    /// `key=value` attributes in source order.
    pub attributes: Vec<DirectiveAttribute>,
}

impl Directive {
    /// The value of the first attribute named `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attr| attr.name == name)
            .map(|attr| attr.value.as_str())
    }
}

/// One `name=value` or `name="value"` attribute of a [`Directive`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveAttribute {
    /// The attribute name.
    pub name: String,
    /// The value with quotes removed and escapes resolved.
    pub value: String,
    /// Bytes of the whole `name=value` in the source.
    pub span: Span,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_type_names_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &ty in NodeType::ALL {
            assert!(seen.insert(ty.as_str()), "duplicate name {ty}");
            assert_eq!(NodeType::from_name(ty.as_str()), Some(ty));
        }
        assert_eq!(NodeType::from_name("nope"), None);
    }

    #[test]
    fn kind_reports_its_type_and_category() {
        let heading = NodeKind::Heading(Heading::atx(2));
        assert_eq!(heading.node_type(), NodeType::Heading);
        assert!(heading.is_block());
        let text = NodeKind::Text("x".into());
        assert_eq!(text.node_type().as_str(), "text");
        assert!(text.is_inline());
        assert!(NodeType::Callout.is_block());
        assert!(NodeType::WikiLink.is_inline());
    }

    #[test]
    fn payload_spans_are_mapped() {
        let shift = |s: Span| Span::new(s.start + 10, s.end + 10);
        let mut heading = NodeKind::Heading(Heading {
            attrs: Some(HeadingAttrs {
                span: Span::new(1, 2),
                ..HeadingAttrs::default()
            }),
            ..Heading::atx(1)
        });
        heading.map_spans(shift);
        let NodeKind::Heading(Heading {
            attrs: Some(attrs), ..
        }) = &heading
        else {
            unreachable!()
        };
        assert_eq!(attrs.span, Span::new(11, 12));
        let attribute = DirectiveAttribute {
            name: "a".into(),
            value: "b".into(),
            span: Span::new(3, 4),
        };
        let mut toc = NodeKind::TocDirective(Directive {
            attributes: vec![attribute],
        });
        toc.map_spans(shift);
        let NodeKind::TocDirective(directive) = &toc else {
            unreachable!()
        };
        assert_eq!(directive.attributes[0].span, Span::new(13, 14));
    }

    #[test]
    fn code_block_lang_is_first_info_word() {
        let block = |info: &str| CodeBlock {
            fence: None,
            info: info.into(),
            literal: String::new(),
        };
        assert_eq!(block("rust ignore").lang(), Some("rust"));
        assert_eq!(block("").lang(), None);
    }

    #[test]
    fn task_state_from_marker() {
        assert_eq!(TaskState::from_marker(' '), TaskState::Open);
        assert_eq!(TaskState::from_marker('X'), TaskState::Done);
        assert_eq!(TaskState::from_marker('?'), TaskState::Other('?'));
    }

    #[test]
    fn directive_attribute_lookup() {
        let directive = Directive {
            attributes: vec![DirectiveAttribute {
                name: "path".into(),
                value: "a.md".into(),
                span: Span::new(0, 0),
            }],
        };
        assert_eq!(directive.get("path"), Some("a.md"));
        assert_eq!(directive.get("shift"), None);
    }
}
