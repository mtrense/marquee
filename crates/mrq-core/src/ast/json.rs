//! The JSON projection of a document: mrq's versioned, mdast-like external AST format.
//!
//! [`project`] (also [`Document::to_json_ast`]) turns a [`Document`] into a [`JsonAst`],
//! whose serde serialization is the contract used by `mrq render -o json`, `mrq query -o
//! json` and the jq front-end. The types also deserialize, so tests and Rust consumers can
//! read the JSON back into typed values; [`schema_json`] is its JSON Schema, checked in at
//! `crates/mrq-core/schema/mrq-ast-1.schema.json` (regenerate with `just schema`).
//!
//! ```
//! use mrq_core::Document;
//!
//! let json = Document::parse("# Hi\n").to_json_ast().to_json_string();
//! let value: serde_json::Value = serde_json::from_str(&json)?;
//! assert_eq!(value["mrq_ast"], 1);
//! assert_eq!(value["children"][0]["type"], "heading");
//! assert_eq!(value["children"][0]["depth"], 1);
//! assert_eq!(value["children"][0]["position"]["start"]["column"], 1);
//! # Ok::<(), serde_json::Error>(())
//! ```
//!
//! # Shape
//!
//! The top level is the root node plus document data, in this key order:
//!
//! ```json
//! { "mrq_ast": 1, "flavor": "gfm", "frontmatter": { "title": "T" },
//!   "type": "root", "id": 0, "position": { … }, "children": [ … ] }
//! ```
//!
//! - `mrq_ast` is the format version, [`VERSION`]. Additive changes (new node types, new
//!   fields) keep it; renaming or removing a type or field, or changing a field's meaning,
//!   bumps it.
//! - `flavor` is the name of the flavor the document was parsed with.
//! - `frontmatter` is the parsed frontmatter mapping, or `null` when the document has none
//!   or it is malformed. The block itself is also a `frontmatter` node among the children.
//!
//! Every node (unist style) has `type`, its kind-specific fields, `id`, `position` and,
//! for parent types ([`is_parent`]), `children`; leaves have no `children` key. Optional
//! fields are always present and `null` when absent. Field names are `camelCase`.
//!
//! # Type names
//!
//! A fixed table, [`type_name`], independent of [`NodeType::as_str`]: mdast's name where
//! mdast (or its GFM, frontmatter and math extensions) has the same concept, and
//! `camelCase` names of the mrq node type otherwise. Inline variants of block concepts
//! take an `inline` prefix, as in mdast (`code`/`inlineCode`, `html`/`inlineHtml`).
//!
//! | mrq node type | JSON `type` | fields |
//! | --- | --- | --- |
//! | `document` | `root` | (top level only, see above) |
//! | `front_matter` | `frontmatter` | `format` (`yaml`/`toml`), `value` (verbatim) |
//! | `paragraph` | `paragraph` | |
//! | `heading` | `heading` | `depth`, `setext`, `attrs` (`{id, classes, attributes: [{name, value}], position}`) |
//! | `thematic_break` | `thematicBreak` | |
//! | `block_quote` | `blockquote` | |
//! | `list` | `list` | `ordered`, `start`, `spread` (not tight), `marker` (bullet), `delimiter` (ordered) |
//! | `list_item` | `listItem` | `checked`, `taskMarker` |
//! | `code_block` | `code` | `lang`, `meta`, `fence` (`{char, length}`, `null` if indented), `value` |
//! | `html_block` | `html` | `value` |
//! | `table` | `table` | `align` (`left`/`center`/`right`/`null` per column) |
//! | `table_row` | `tableRow` | `header` |
//! | `table_cell` | `tableCell` | |
//! | `footnote_definition` | `footnoteDefinition` | `label` |
//! | `link_definition` | `definition` | `label`, `url`, `title` |
//! | `definition_list` | `definitionList` | |
//! | `definition_item` | `definitionItem` | |
//! | `definition_term` | `definitionTerm` | |
//! | `definition_details` | `definitionDetails` | |
//! | `admonition` | `admonition` | `kind`, `classes`, `title`, `fold` (`open`/`closed`) |
//! | `callout` | `callout` | `kind`, `title`, `fold` |
//! | `abbreviation_definition` | `abbreviationDefinition` | `abbr`, `expansion` |
//! | `include_directive` | `includeDirective` | `attributes` (`[{name, value, position}]`) |
//! | `toc_directive` | `tocDirective` | `attributes` |
//! | `directive_end` | `directiveEnd` | |
//! | `text` | `text` | `value` |
//! | `soft_break` | `softBreak` | |
//! | `line_break` | `break` | |
//! | `code` | `inlineCode` | `value` |
//! | `emphasis` | `emphasis` | |
//! | `strong` | `strong` | |
//! | `strikethrough` | `delete` | |
//! | `highlight` | `highlight` | |
//! | `superscript` | `superscript` | |
//! | `subscript` | `subscript` | |
//! | `link` | `link` | `url`, `title`, `form` (`inline`/`reference`/`autolink`/`bare`), `label`, `referenceType` (`full`/`collapsed`/`shortcut`) |
//! | `image` | `image` | as `link`, plus `alt` (plain text of the children) |
//! | `html_inline` | `inlineHtml` | `value` |
//! | `footnote_reference` | `footnoteReference` | `label` |
//! | `wiki_link` | `wikiLink` | `target`, `anchor`, `alias` |
//! | `embed` | `embed` | `target`, `anchor`, `alias` |
//! | `tag` | `tag` | `name` |
//! | `math` | `math` | `display`, `value` |
//! | `comment` | `comment` | `syntax` (`html`/`percent`), `value` |
//!
//! Differences from mdast: references stay `link`/`image` (with the resolved `url`, plus
//! `form`, `label` and `referenceType`) instead of `linkReference`/`imageReference`;
//! images keep their alt text as children; soft line breaks are `softBreak` nodes instead
//! of `\n` in text; `code` values keep their final newline; footnotes and definitions
//! carry the `label` as written but no normalized `identifier`. A task item with a custom
//! marker (Obsidian `[?]`, `[-]`) has `checked: true` and the marker in `taskMarker`;
//! `taskMarker` is `null` otherwise. Directive start and end markers are flat siblings:
//! which `directiveEnd` closes which start is not projected (see
//! [`DirectiveRegion`](crate::directive::DirectiveRegion)).
//!
//! # Positions
//!
//! `position` is `{start, end}`, each a point `{line, column, offset}` as in unist:
//! `line` and `column` are 1-based, `offset` is 0-based and `end` points just past the
//! node. `offset` counts UTF-8 bytes, so it indexes the source like mrq's spans;
//! `column` counts UTF-16 code units, like mrq's diagnostics, LSP and JavaScript mdast
//! tools. A `\r` before a `\n` counts as part of its line.
//!
//! # Ids
//!
//! `id` is the node's [`NodeId::index`] in the document's [`Ast`]: unique within
//! the document, deterministic for the same input and mrq version, and usable to map a
//! JSON node back to the tree. Ids are not ordered or contiguous (flavor passes create
//! nodes) and may change between mrq versions; use positions to identify nodes across
//! runs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Ast, NodeId, NodeKind, NodeType, Span};
use crate::document::Document;
use crate::source::{PositionEncoding, SourceFile};

/// Version of the JSON format, the value of `mrq_ast`.
pub const VERSION: u32 = 1;

/// A whole document: the root node plus document data. Serializes as one JSON object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(
    title = "mrq AST (JSON projection, version 1)",
    description = "The mdast-like JSON projection of a Markdown document parsed by mrq."
)]
#[non_exhaustive]
pub struct JsonAst {
    /// Format version; always 1 for this schema.
    #[schemars(extend("const" = 1))]
    pub mrq_ast: u32,
    /// Name of the flavor the document was parsed with, e.g. `gfm`.
    pub flavor: String,
    /// The parsed frontmatter mapping; null without frontmatter or when it is malformed.
    #[schemars(with = "Option<serde_json::Map<String, Value>>")]
    pub frontmatter: Option<Value>,
    /// The root node (type `root`).
    #[serde(flatten)]
    pub root: JsonNode,
}

impl JsonAst {
    /// Pretty-printed JSON with a trailing newline.
    pub fn to_json_string(&self) -> String {
        let mut json = serde_json::to_string_pretty(self).expect("the projection serializes");
        json.push('\n');
        json
    }
}

/// One node: its type and type-specific fields, id, position and children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct JsonNode {
    /// `type` and the fields specific to it.
    #[serde(flatten)]
    pub kind: JsonKind,
    /// Identifier of the node, unique within the document; not stable across mrq versions.
    pub id: usize,
    /// Where the node is in the source.
    pub position: Position,
    /// Child nodes; present (possibly empty) on parent types, absent on leaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<JsonNode>")]
    pub children: Option<Vec<JsonNode>>,
}

/// A source range: `start` and the point just past the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    /// First character of the node.
    pub start: Point,
    /// Just past the last character of the node.
    pub end: Point,
}

/// A place in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Point {
    /// 1-based line.
    pub line: usize,
    /// 1-based column in UTF-16 code units.
    pub column: usize,
    /// 0-based offset in UTF-8 bytes from the start of the source.
    pub offset: usize,
}

/// The `type` of a node with its type-specific fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum JsonKind {
    /// The document root.
    Root,
    /// YAML or TOML frontmatter block.
    Frontmatter(Frontmatter),
    /// Paragraph; children are inlines.
    Paragraph,
    /// ATX or setext heading; children are inlines.
    Heading(Heading),
    /// `---`, `***` or `___`.
    ThematicBreak,
    /// Block quote; children are blocks.
    Blockquote,
    /// Bullet or ordered list; children are `listItem`s.
    List(List),
    /// List item, possibly a task; children are blocks.
    ListItem(ListItem),
    /// Fenced or indented code block.
    Code(Code),
    /// Raw HTML block.
    Html(Literal),
    /// GFM table; children are `tableRow`s, the header row first.
    Table(Table),
    /// Table row; children are `tableCell`s.
    TableRow(TableRow),
    /// Table cell; children are inlines.
    TableCell,
    /// `[^label]: …`; children are blocks.
    FootnoteDefinition(Label),
    /// Link reference definition `[label]: url "title"`.
    Definition(Definition),
    /// Definition list; children are `definitionItem`s.
    DefinitionList,
    /// A term with its details; children are a `definitionTerm` and `definitionDetails`.
    DefinitionItem,
    /// Defined term; children are blocks.
    DefinitionTerm,
    /// `: …` details; children are blocks.
    DefinitionDetails,
    /// mkdocs admonition `!!! kind "title"`; children are blocks.
    Admonition(Admonition),
    /// GFM alert or Obsidian callout `> [!kind]± title`; children are blocks.
    Callout(Callout),
    /// Abbreviation definition `*[abbr]: expansion`.
    AbbreviationDefinition(AbbreviationDefinition),
    /// `<!-- mrq:include … -->`.
    IncludeDirective(Directive),
    /// `<!-- mrq:toc … -->`.
    TocDirective(Directive),
    /// `<!-- mrq:end -->`.
    DirectiveEnd,
    /// Literal text.
    Text(Literal),
    /// A line ending inside a paragraph that is not a hard break.
    SoftBreak,
    /// Hard line break.
    Break,
    /// Code span.
    InlineCode(Literal),
    /// `*…*`; children are inlines.
    Emphasis,
    /// `**…**`; children are inlines.
    Strong,
    /// `~~…~~`; children are inlines.
    Delete,
    /// `==…==`; children are inlines.
    Highlight,
    /// `^…^`; children are inlines.
    Superscript,
    /// `~…~`; children are inlines.
    Subscript,
    /// Link; children are the link text.
    Link(Link),
    /// Image; children are the alt text.
    Image(Image),
    /// Raw inline HTML.
    InlineHtml(Literal),
    /// `[^label]`.
    FootnoteReference(Label),
    /// Obsidian `[[target#anchor|alias]]`.
    WikiLink(WikiLink),
    /// Obsidian `![[target#anchor|alias]]`.
    Embed(WikiLink),
    /// Obsidian `#tag`.
    Tag(Tag),
    /// `$…$` or `$$…$$`.
    Math(Math),
    /// `<!-- … -->` or Obsidian `%%…%%`.
    Comment(Comment),
}

/// Fields of a node whose content is a string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Literal {
    /// The content (text with escapes resolved, code, or raw HTML).
    pub value: String,
}

/// Fields of a footnote definition or reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Label {
    /// The label as written, without `[^` and `]`.
    pub label: String,
}

/// Syntax of a frontmatter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum FrontmatterFormat {
    /// `---` delimited YAML.
    Yaml,
    /// `+++` delimited TOML.
    Toml,
}

/// Fields of a `frontmatter` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Frontmatter {
    /// YAML or TOML.
    pub format: FrontmatterFormat,
    /// The text between the delimiter lines, verbatim.
    pub value: String,
}

/// Fields of a `heading` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Heading {
    /// Level, 1 to 6.
    #[schemars(range(min = 1, max = 6))]
    pub depth: u8,
    /// Whether the heading is underlined (`===` / `---`) rather than `#`-prefixed.
    pub setext: bool,
    /// The `{#id .class key=value}` attribute list (mkdocs), if any.
    pub attrs: Option<HeadingAttrs>,
}

/// A heading's `{#id .class key=value}` attribute list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct HeadingAttrs {
    /// `#id`, which overrides the generated anchor.
    pub id: Option<String>,
    /// `.class` entries in source order.
    pub classes: Vec<String>,
    /// `key=value` entries in source order.
    pub attributes: Vec<Attribute>,
    /// Where the `{…}` block is.
    pub position: Position,
}

/// A `name=value` attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Attribute {
    /// The name.
    pub name: String,
    /// The value, unquoted.
    pub value: String,
}

/// Fields of a `list` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct List {
    /// Ordered (`1.`) rather than bullet (`-`).
    pub ordered: bool,
    /// Number of the first item of an ordered list; null for bullet lists.
    pub start: Option<u64>,
    /// Whether the list is loose (items separated by blank lines).
    pub spread: bool,
    /// Bullet character (`-`, `*` or `+`); null for ordered lists.
    pub marker: Option<char>,
    /// `.` or `)` after the numbers of an ordered list; null for bullet lists.
    pub delimiter: Option<char>,
}

/// Fields of a `listItem` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ListItem {
    /// null if the item is not a task; false for `[ ]`; true for `[x]` and custom markers.
    pub checked: Option<bool>,
    /// The custom task marker (Obsidian `[?]`, `[-]`, …); null otherwise.
    pub task_marker: Option<char>,
}

/// Fields of a `code` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Code {
    /// The first word of the info string, if any.
    pub lang: Option<String>,
    /// The rest of the info string after the language, if any.
    pub meta: Option<String>,
    /// The opening fence; null for an indented code block.
    pub fence: Option<Fence>,
    /// The code, including its final newline.
    pub value: String,
}

/// Opening fence of a fenced code block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Fence {
    /// `` ` `` or `~`.
    pub char: char,
    /// Number of fence characters (at least 3).
    pub length: usize,
}

/// Column alignment of a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    /// `:--`
    Left,
    /// `:-:`
    Center,
    /// `--:`
    Right,
}

/// Fields of a `table` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Table {
    /// One entry per column; null for columns without alignment.
    pub align: Vec<Option<Align>>,
}

/// Fields of a `tableRow` node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TableRow {
    /// Whether this is the header row.
    pub header: bool,
}

/// Fields of a `definition` node (link reference definition).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Definition {
    /// The label as written, without brackets (not case-folded).
    pub label: String,
    /// The destination, unescaped.
    pub url: String,
    /// The title, unescaped.
    pub title: Option<String>,
}

/// Whether a collapsible admonition or callout starts open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Fold {
    /// mkdocs `???+`, Obsidian `[!note]+`.
    Open,
    /// mkdocs `???`, Obsidian `[!note]-`.
    Closed,
}

/// Fields of an `admonition` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Admonition {
    /// The type keyword as written, e.g. `note`.
    pub kind: String,
    /// Extra CSS classes after the kind.
    pub classes: Vec<String>,
    /// The quoted title; an empty string means an explicitly empty title.
    pub title: Option<String>,
    /// null unless collapsible (`???`).
    pub fold: Option<Fold>,
}

/// Fields of a `callout` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Callout {
    /// The type keyword as written, e.g. `NOTE` or `tip`.
    pub kind: String,
    /// Title text after the marker (Obsidian only).
    pub title: Option<String>,
    /// null unless foldable (`+` / `-`).
    pub fold: Option<Fold>,
}

/// Fields of an `abbreviationDefinition` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AbbreviationDefinition {
    /// The abbreviation, e.g. `HTML`.
    pub abbr: String,
    /// What it stands for.
    pub expansion: String,
}

/// Fields of an `includeDirective` or `tocDirective` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Directive {
    /// `name=value` attributes in source order.
    pub attributes: Vec<DirectiveAttribute>,
}

/// One attribute of a directive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct DirectiveAttribute {
    /// The name.
    pub name: String,
    /// The value with quotes removed and escapes resolved.
    pub value: String,
    /// Where the whole `name=value` is.
    pub position: Position,
}

/// How a link's destination was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LinkForm {
    /// `[text](url "title")`
    Inline,
    /// `[text][label]`, `[label][]` or `[label]`.
    Reference,
    /// `<https://…>` or `<user@host>`.
    Autolink,
    /// A GFM extended autolink: a bare URL or e-mail address in text.
    Bare,
}

/// Style of a reference link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceType {
    /// `[text][label]`
    Full,
    /// `[label][]`
    Collapsed,
    /// `[label]`
    Shortcut,
}

/// Fields of a `link` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Link {
    /// The destination, unescaped (resolved through the definition for references).
    pub url: String,
    /// The title, unescaped.
    pub title: Option<String>,
    /// How the destination was written.
    pub form: LinkForm,
    /// The reference label as written; null unless `form` is `reference`.
    pub label: Option<String>,
    /// The reference style; null unless `form` is `reference`.
    pub reference_type: Option<ReferenceType>,
}

/// Fields of an `image` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Image {
    /// The source, unescaped (resolved through the definition for references).
    pub url: String,
    /// The title, unescaped.
    pub title: Option<String>,
    /// Plain text of the alt text (the children).
    pub alt: String,
    /// How the destination was written.
    pub form: LinkForm,
    /// The reference label as written; null unless `form` is `reference`.
    pub label: Option<String>,
    /// The reference style; null unless `form` is `reference`.
    pub reference_type: Option<ReferenceType>,
}

/// Fields of a `wikiLink` or `embed` node: `[[target#anchor|alias]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WikiLink {
    /// The page or file name; empty for a same-page link `[[#heading]]`.
    pub target: String,
    /// Text after `#`: a heading, or `^block-id`.
    pub anchor: Option<String>,
    /// Text after `|`: display text, or a size such as `100x200` for embeds.
    pub alias: Option<String>,
}

/// Fields of a `tag` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Tag {
    /// The tag without its leading `#`, e.g. `project/active`.
    pub name: String,
}

/// Fields of a `math` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Math {
    /// `$$…$$` (display) rather than `$…$` (inline).
    pub display: bool,
    /// The TeX source between the delimiters.
    pub value: String,
}

/// Delimiters of a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CommentSyntax {
    /// `<!-- … -->`
    Html,
    /// Obsidian `%% … %%`
    Percent,
}

/// Fields of a `comment` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Comment {
    /// HTML or Obsidian syntax.
    pub syntax: CommentSyntax,
    /// The text between the delimiters, verbatim.
    pub value: String,
}

/// The JSON `type` of nodes of type `ty`; see the [module docs](self) for the table.
pub const fn type_name(ty: NodeType) -> &'static str {
    match ty {
        NodeType::Document => "root",
        NodeType::FrontMatter => "frontmatter",
        NodeType::Paragraph => "paragraph",
        NodeType::Heading => "heading",
        NodeType::ThematicBreak => "thematicBreak",
        NodeType::BlockQuote => "blockquote",
        NodeType::List => "list",
        NodeType::ListItem => "listItem",
        NodeType::CodeBlock => "code",
        NodeType::HtmlBlock => "html",
        NodeType::Table => "table",
        NodeType::TableRow => "tableRow",
        NodeType::TableCell => "tableCell",
        NodeType::FootnoteDefinition => "footnoteDefinition",
        NodeType::LinkDefinition => "definition",
        NodeType::DefinitionList => "definitionList",
        NodeType::DefinitionItem => "definitionItem",
        NodeType::DefinitionTerm => "definitionTerm",
        NodeType::DefinitionDetails => "definitionDetails",
        NodeType::Admonition => "admonition",
        NodeType::Callout => "callout",
        NodeType::AbbreviationDefinition => "abbreviationDefinition",
        NodeType::IncludeDirective => "includeDirective",
        NodeType::TocDirective => "tocDirective",
        NodeType::DirectiveEnd => "directiveEnd",
        NodeType::Text => "text",
        NodeType::SoftBreak => "softBreak",
        NodeType::LineBreak => "break",
        NodeType::Code => "inlineCode",
        NodeType::Emphasis => "emphasis",
        NodeType::Strong => "strong",
        NodeType::Strikethrough => "delete",
        NodeType::Highlight => "highlight",
        NodeType::Superscript => "superscript",
        NodeType::Subscript => "subscript",
        NodeType::Link => "link",
        NodeType::Image => "image",
        NodeType::HtmlInline => "inlineHtml",
        NodeType::FootnoteReference => "footnoteReference",
        NodeType::WikiLink => "wikiLink",
        NodeType::Embed => "embed",
        NodeType::Tag => "tag",
        NodeType::Math => "math",
        NodeType::Comment => "comment",
    }
}

/// Whether nodes of type `ty` always carry `children` (possibly empty). Leaves get the
/// key only if they unexpectedly have children, so nothing in the tree is dropped.
pub const fn is_parent(ty: NodeType) -> bool {
    matches!(
        ty,
        NodeType::Document
            | NodeType::Paragraph
            | NodeType::Heading
            | NodeType::BlockQuote
            | NodeType::List
            | NodeType::ListItem
            | NodeType::Table
            | NodeType::TableRow
            | NodeType::TableCell
            | NodeType::FootnoteDefinition
            | NodeType::DefinitionList
            | NodeType::DefinitionItem
            | NodeType::DefinitionTerm
            | NodeType::DefinitionDetails
            | NodeType::Admonition
            | NodeType::Callout
            | NodeType::Emphasis
            | NodeType::Strong
            | NodeType::Strikethrough
            | NodeType::Highlight
            | NodeType::Superscript
            | NodeType::Subscript
            | NodeType::Link
            | NodeType::Image
    )
}

/// Projects a whole document.
pub fn project(doc: &Document) -> JsonAst {
    JsonAst {
        mrq_ast: VERSION,
        flavor: doc.flavor().name().to_owned(),
        frontmatter: doc.frontmatter().and_then(|fm| fm.value()).cloned(),
        root: project_node(doc, doc.ast().root()),
    }
}

/// Projects the subtree of `doc` at `id`, e.g. a query match.
pub fn project_node(doc: &Document, id: NodeId) -> JsonNode {
    Projector {
        ast: doc.ast(),
        source: doc.source(),
    }
    .node(id)
}

/// The [`Position`] of `span` in `source`, as in the projection. Offsets are clamped into
/// the text and back to a char boundary (spans of parsed trees always are; hand-built
/// trees may not be).
pub fn position(source: &SourceFile, span: Span) -> Position {
    Position {
        start: point(source, span.start),
        end: point(source, span.end),
    }
}

fn point(source: &SourceFile, offset: usize) -> Point {
    let text = source.text();
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let position = source
        .offset_to_position(offset, PositionEncoding::Utf16)
        .expect("a char boundary within the text");
    Point {
        line: position.line + 1,
        column: position.col + 1,
        offset,
    }
}

/// Projects nodes of `ast`, whose spans index `source`.
struct Projector<'a> {
    ast: &'a Ast,
    source: &'a SourceFile,
}

impl Projector<'_> {
    fn node(&self, id: NodeId) -> JsonNode {
        let ast = self.ast;
        let ty = ast.kind(id).node_type();
        let children = ast.children(id);
        JsonNode {
            kind: self.kind(id),
            id: id.index(),
            position: self.position(ast.span(id)),
            children: (is_parent(ty) || !children.is_empty())
                .then(|| children.iter().map(|&child| self.node(child)).collect()),
        }
    }

    fn position(&self, span: Span) -> Position {
        position(self.source, span)
    }

    #[allow(clippy::too_many_lines)] // one arm per node kind
    fn kind(&self, id: NodeId) -> JsonKind {
        use super::{
            Alignment, CommentSyntax as Syntax, FrontMatterFormat, LinkForm as Form, ListKind,
            OrderedDelimiter, ReferenceStyle, TaskState,
        };

        let literal = |value: &String| Literal {
            value: value.clone(),
        };
        let fold = |fold: Option<super::Fold>| {
            fold.map(|fold| match fold {
                super::Fold::Open => Fold::Open,
                super::Fold::Closed => Fold::Closed,
            })
        };
        let directive = |directive: &super::Directive| Directive {
            attributes: directive
                .attributes
                .iter()
                .map(|attr| DirectiveAttribute {
                    name: attr.name.clone(),
                    value: attr.value.clone(),
                    position: self.position(attr.span),
                })
                .collect(),
        };
        let link_form = |form: &Form| match form {
            Form::Inline => (LinkForm::Inline, None, None),
            Form::Reference { label, style } => (
                LinkForm::Reference,
                Some(label.clone()),
                Some(match style {
                    ReferenceStyle::Full => ReferenceType::Full,
                    ReferenceStyle::Collapsed => ReferenceType::Collapsed,
                    ReferenceStyle::Shortcut => ReferenceType::Shortcut,
                }),
            ),
            Form::Autolink => (LinkForm::Autolink, None, None),
            Form::Bare => (LinkForm::Bare, None, None),
        };
        let wiki = |link: &super::WikiLink| WikiLink {
            target: link.target.clone(),
            anchor: link.anchor.clone(),
            alias: link.alias.clone(),
        };

        match self.ast.kind(id) {
            NodeKind::Document => JsonKind::Root,
            NodeKind::FrontMatter(fm) => JsonKind::Frontmatter(Frontmatter {
                format: match fm.format {
                    FrontMatterFormat::Yaml => FrontmatterFormat::Yaml,
                    FrontMatterFormat::Toml => FrontmatterFormat::Toml,
                },
                value: fm.literal.clone(),
            }),
            NodeKind::Paragraph => JsonKind::Paragraph,
            NodeKind::Heading(heading) => JsonKind::Heading(Heading {
                depth: heading.level,
                setext: heading.setext,
                attrs: heading.attrs.as_ref().map(|attrs| HeadingAttrs {
                    id: attrs.id.clone(),
                    classes: attrs.classes.clone(),
                    attributes: attrs
                        .attributes
                        .iter()
                        .map(|(name, value)| Attribute {
                            name: name.clone(),
                            value: value.clone(),
                        })
                        .collect(),
                    position: self.position(attrs.span),
                }),
            }),
            NodeKind::ThematicBreak => JsonKind::ThematicBreak,
            NodeKind::BlockQuote => JsonKind::Blockquote,
            NodeKind::List(list) => JsonKind::List(match list.kind {
                ListKind::Bullet { marker } => List {
                    ordered: false,
                    start: None,
                    spread: !list.tight,
                    marker: Some(marker),
                    delimiter: None,
                },
                ListKind::Ordered { start, delimiter } => List {
                    ordered: true,
                    start: Some(start),
                    spread: !list.tight,
                    marker: None,
                    delimiter: Some(match delimiter {
                        OrderedDelimiter::Period => '.',
                        OrderedDelimiter::Paren => ')',
                    }),
                },
            }),
            NodeKind::ListItem(item) => JsonKind::ListItem(match item.task {
                None => ListItem {
                    checked: None,
                    task_marker: None,
                },
                Some(TaskState::Open) => ListItem {
                    checked: Some(false),
                    task_marker: None,
                },
                Some(TaskState::Done) => ListItem {
                    checked: Some(true),
                    task_marker: None,
                },
                Some(TaskState::Other(marker)) => ListItem {
                    checked: Some(true),
                    task_marker: Some(marker),
                },
            }),
            NodeKind::CodeBlock(code) => JsonKind::Code(Code {
                lang: code.lang().map(str::to_owned),
                meta: code
                    .info
                    .trim()
                    .split_once(char::is_whitespace)
                    .map(|(_, rest)| rest.trim().to_owned())
                    .filter(|meta| !meta.is_empty()),
                fence: code.fence.map(|fence| Fence {
                    char: fence.char,
                    length: fence.len,
                }),
                value: code.literal.clone(),
            }),
            NodeKind::HtmlBlock(html) => JsonKind::Html(literal(html)),
            NodeKind::Table(table) => JsonKind::Table(Table {
                align: table
                    .alignments
                    .iter()
                    .map(|align| match align {
                        Alignment::None => None,
                        Alignment::Left => Some(Align::Left),
                        Alignment::Center => Some(Align::Center),
                        Alignment::Right => Some(Align::Right),
                    })
                    .collect(),
            }),
            NodeKind::TableRow(row) => JsonKind::TableRow(TableRow { header: row.header }),
            NodeKind::TableCell => JsonKind::TableCell,
            NodeKind::FootnoteDefinition(def) => JsonKind::FootnoteDefinition(Label {
                label: def.label.clone(),
            }),
            NodeKind::LinkDefinition(def) => JsonKind::Definition(Definition {
                label: def.label.clone(),
                url: def.url.clone(),
                title: def.title.clone(),
            }),
            NodeKind::DefinitionList => JsonKind::DefinitionList,
            NodeKind::DefinitionItem => JsonKind::DefinitionItem,
            NodeKind::DefinitionTerm => JsonKind::DefinitionTerm,
            NodeKind::DefinitionDetails => JsonKind::DefinitionDetails,
            NodeKind::Admonition(adm) => JsonKind::Admonition(Admonition {
                kind: adm.kind.clone(),
                classes: adm.classes.clone(),
                title: adm.title.clone(),
                fold: fold(adm.fold),
            }),
            NodeKind::Callout(callout) => JsonKind::Callout(Callout {
                kind: callout.kind.clone(),
                title: callout.title.clone(),
                fold: fold(callout.fold),
            }),
            NodeKind::AbbreviationDefinition(abbr) => {
                JsonKind::AbbreviationDefinition(AbbreviationDefinition {
                    abbr: abbr.abbr.clone(),
                    expansion: abbr.expansion.clone(),
                })
            }
            NodeKind::IncludeDirective(d) => JsonKind::IncludeDirective(directive(d)),
            NodeKind::TocDirective(d) => JsonKind::TocDirective(directive(d)),
            NodeKind::DirectiveEnd => JsonKind::DirectiveEnd,
            NodeKind::Text(text) => JsonKind::Text(literal(text)),
            NodeKind::SoftBreak => JsonKind::SoftBreak,
            NodeKind::LineBreak => JsonKind::Break,
            NodeKind::Code(code) => JsonKind::InlineCode(literal(code)),
            NodeKind::Emphasis => JsonKind::Emphasis,
            NodeKind::Strong => JsonKind::Strong,
            NodeKind::Strikethrough => JsonKind::Delete,
            NodeKind::Highlight => JsonKind::Highlight,
            NodeKind::Superscript => JsonKind::Superscript,
            NodeKind::Subscript => JsonKind::Subscript,
            NodeKind::Link(link) => {
                let (form, label, reference_type) = link_form(&link.form);
                JsonKind::Link(Link {
                    url: link.url.clone(),
                    title: link.title.clone(),
                    form,
                    label,
                    reference_type,
                })
            }
            NodeKind::Image(link) => {
                let (form, label, reference_type) = link_form(&link.form);
                JsonKind::Image(Image {
                    url: link.url.clone(),
                    title: link.title.clone(),
                    alt: self.ast.text(id),
                    form,
                    label,
                    reference_type,
                })
            }
            NodeKind::HtmlInline(html) => JsonKind::InlineHtml(literal(html)),
            NodeKind::FootnoteReference(reference) => JsonKind::FootnoteReference(Label {
                label: reference.label.clone(),
            }),
            NodeKind::WikiLink(link) => JsonKind::WikiLink(wiki(link)),
            NodeKind::Embed(link) => JsonKind::Embed(wiki(link)),
            NodeKind::Tag(tag) => JsonKind::Tag(Tag {
                name: tag.name.clone(),
            }),
            NodeKind::Math(math) => JsonKind::Math(Math {
                display: math.display,
                value: math.literal.clone(),
            }),
            NodeKind::Comment(comment) => JsonKind::Comment(Comment {
                syntax: match comment.syntax {
                    Syntax::Html => CommentSyntax::Html,
                    Syntax::Percent => CommentSyntax::Percent,
                },
                value: comment.literal.clone(),
            }),
        }
    }
}

/// The JSON Schema of the projection ([`JsonAst`]).
///
/// Every property but `children` is required: optional fields are serialized as `null`,
/// never left out. Unknown properties are allowed, so adding fields keeps old schemas
/// valid.
pub fn schema() -> schemars::Schema {
    schemars::generate::SchemaSettings::draft2020_12()
        .with_transform(schemars::transform::RecursiveTransform(require_properties))
        .into_generator()
        .into_root_schema_for::<JsonAst>()
}

/// Marks all properties of an object schema, except `children`, as required.
fn require_properties(schema: &mut schemars::Schema) {
    let Some(Value::Object(properties)) = schema.get("properties") else {
        return;
    };
    let required: Vec<Value> = properties
        .keys()
        .filter(|key| *key != "children")
        .map(|key| Value::String(key.clone()))
        .collect();
    schema.insert("required".to_owned(), Value::Array(required));
}

/// [`schema`] as pretty-printed JSON with a trailing newline; the content of
/// `crates/mrq-core/schema/mrq-ast-1.schema.json`.
pub fn schema_json() -> String {
    let mut json = serde_json::to_string_pretty(&schema()).unwrap_or_default();
    json.push('\n');
    json
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, Builder, TaskState};

    /// Projects the root of `ast` over `src`.
    fn project_ast(ast: &Ast, src: &str) -> JsonNode {
        let source = SourceFile::new(None, src);
        Projector {
            ast,
            source: &source,
        }
        .node(ast.root())
    }

    fn kinds(node: &JsonNode) -> Vec<JsonKind> {
        let mut out = vec![node.kind.clone()];
        for child in node.children.iter().flatten() {
            out.extend(kinds(child));
        }
        out
    }

    #[test]
    fn kinds_the_parser_does_not_produce_yet() {
        let src = "<!-- mrq:toc depth=2 -->\n<!-- mrq:end -->\n";
        let ast = Builder::document(|b| {
            b.leaf(NodeKind::LinkDefinition(ast::LinkDefinition {
                label: "r".into(),
                url: "u".into(),
                title: None,
            }));
            b.bullet_list('*', |b| {
                b.task(TaskState::Other('-'), |_| {});
            });
            let toc = b.leaf(NodeKind::TocDirective(ast::Directive {
                attributes: vec![ast::DirectiveAttribute {
                    name: "depth".into(),
                    value: "2".into(),
                    span: Span::new(14, 21),
                }],
            }));
            b.set_span(toc, 0..24);
            b.leaf(NodeKind::DirectiveEnd);
        });
        let root = project_ast(&ast, src);
        let kinds = kinds(&root);
        assert_eq!(
            kinds[1],
            JsonKind::Definition(Definition {
                label: "r".into(),
                url: "u".into(),
                title: None,
            })
        );
        assert_eq!(
            kinds[3],
            JsonKind::ListItem(ListItem {
                checked: Some(true),
                task_marker: Some('-'),
            })
        );
        let JsonKind::TocDirective(toc) = &kinds[4] else {
            panic!("{:?}", kinds[4]);
        };
        let attr = &toc.attributes[0];
        assert_eq!((attr.name.as_str(), attr.value.as_str()), ("depth", "2"));
        assert_eq!(
            (attr.position.start.column, attr.position.end.column),
            (15, 22)
        );
        assert_eq!(kinds[5], JsonKind::DirectiveEnd);
        // An empty list item is a parent with no children; directives are leaves.
        let list = &root.children.as_ref().unwrap()[1];
        assert_eq!(list.children.as_ref().unwrap()[0].children, Some(vec![]));
        assert_eq!(root.children.as_ref().unwrap()[2].children, None);
    }

    #[test]
    fn out_of_range_spans_are_clamped() {
        let ast = Builder::document(|b| {
            let t = b.text("é");
            b.set_span(t, 1..99);
        });
        let root = project_ast(&ast, "é\n");
        let text = &root.children.as_ref().unwrap()[0];
        assert_eq!(
            text.position,
            Position {
                start: Point {
                    line: 1,
                    column: 1,
                    offset: 0
                },
                end: Point {
                    line: 2,
                    column: 1,
                    offset: 3
                },
            }
        );
    }

    #[test]
    fn every_node_type_has_a_json_name_matching_its_kind() {
        let ast = Builder::document(|b| {
            b.heading(1, |b| {
                b.code("c");
                b.line_break();
            });
            b.code_block("", "x\n");
            b.html_block("<p>\n");
            b.thematic_break();
        });
        let root = project_ast(&ast, "");
        let value = serde_json::to_value(&root).unwrap();
        let names: Vec<_> = ast
            .descendants(ast.root())
            .map(|id| type_name(ast.kind(id).node_type()))
            .collect();
        let mut json_names = Vec::new();
        let mut stack = vec![&value];
        while let Some(node) = stack.pop() {
            json_names.push(node["type"].as_str().unwrap().to_owned());
            if let Some(children) = node["children"].as_array() {
                stack.extend(children.iter().rev());
            }
        }
        assert_eq!(json_names, names);
    }
}
