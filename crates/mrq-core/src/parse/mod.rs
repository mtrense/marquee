//! Parsing Markdown source into an [`Ast`].
//!
//! [`Parser`] is the seam between mrq and the Markdown parser it builds on. The only
//! implementation, [`ComrakParser`], runs [comrak](https://docs.rs/comrak) and converts its
//! arena into mrq's own [`Ast`]; no comrak type crosses this module's boundary, so a
//! different backend (e.g. a lossless CST) can replace it later.
//!
//! Which syntax is recognized is controlled by [`ParseOptions`], a set of extension flags.
//! Flavors produce these options; the parser knows nothing about flavors.
//!
//! ```
//! use mrq_core::ast::NodeType;
//! use mrq_core::parse::{ComrakParser, ParseOptions, Parser};
//!
//! let src = "# Title\n\n- [x] done\n";
//! let ast = ComrakParser::new(ParseOptions::gfm()).parse(src);
//! let heading = ast.node(ast.children(ast.root())[0]);
//! assert_eq!(heading.node_type(), NodeType::Heading);
//! assert_eq!(heading.source(src), "# Title");
//! ```
//!
//! # Spans
//!
//! Every node gets a byte [`Span`](crate::ast::Span) into `src`, so `&src[span]` is its
//! source text:
//!
//! - The document spans the whole input.
//! - A block spans from its first character (its marker for containers, after any
//!   enclosing container markers; the indentation for an indented code block) to the
//!   last character of its content, without the final line ending. Lists and list items
//!   end with their last child, never with trailing blank lines.
//! - An inline spans its delimiters too (`*a*`, `` `x` ``, `[t](u)`); a [`SoftBreak`]
//!   spans the line ending, a [`LineBreak`] the trailing spaces or backslash plus the
//!   line ending.
//! - Table cells that the parser adds to pad a short row have an empty span at the end
//!   of the row; every other node's span is non-empty.
//!
//! comrak's (line, byte column) positions are converted to byte offsets. Where they are
//! known to drift (inlines after link reference definitions or after a multi-line link
//! destination or title, escaped pipes in table cells, definition terms, empty footnote
//! definitions), they are corrected first; whatever is still missing or inconsistent is
//! repaired from the surrounding source: text is searched for in its parent, containers
//! cover their children. Spans are always nested in their parent's span, ordered among
//! siblings and on `char` boundaries.
//!
//! Known gap: after a tab that is only partly consumed as indentation (e.g. a tab
//! continuing a list item inside a quote), comrak counts columns as if the tab were
//! expanded. Positions on that line are corrected only when text, code or HTML on the
//! same line anchors them; otherwise a span there may miss its delimiters.
//!
//! [`SoftBreak`]: crate::ast::NodeKind::SoftBreak
//! [`LineBreak`]: crate::ast::NodeKind::LineBreak
//!
//! # Not (yet) produced
//!
//! - [`LinkDefinition`](crate::ast::NodeKind::LinkDefinition): comrak consumes link
//!   reference definitions without leaving a node; links that use them still carry the
//!   resolved url and a [`LinkForm::Reference`](crate::ast::LinkForm::Reference) with
//!   the label. Their source lies in the gaps between blocks or before a paragraph's first
//!   inline; a pass that needs them (validate, M4.2) can scan those gaps.
//! - [`FrontMatter`](crate::ast::NodeKind::FrontMatter): frontmatter is split off before
//!   parsing; [`frontmatter::parse_document`](crate::frontmatter::parse_document) parses
//!   the body on its own and assembles the tree with a frontmatter node.
//! - Flavor syntax (wikilinks, admonitions, tags, directives, heading attributes, …) is
//!   recognized by flavor passes over the converted tree.

mod comrak;
mod span;

pub use self::comrak::ComrakParser;

use crate::ast::Ast;

/// Turns Markdown source into an [`Ast`]. Parsing never fails: any input is a document.
pub trait Parser {
    /// Parses `src` into a tree whose spans index into `src`.
    fn parse(&self, src: &str) -> Ast;
}

/// Which syntax extensions a [`Parser`] recognizes on top of `CommonMark`.
///
/// The [`Default`] is plain `CommonMark` (every flag off). Construct with
/// [`commonmark`](Self::commonmark) or [`gfm`](Self::gfm) and adjust fields from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
#[allow(clippy::struct_excessive_bools)] // independent extension flags
pub struct ParseOptions {
    /// GFM `~~strikethrough~~`.
    pub strikethrough: bool,
    /// GFM pipe tables.
    pub table: bool,
    /// GFM extended autolinks: bare `www.…`, `https://…` and e-mail addresses.
    pub autolink: bool,
    /// GFM task list items `- [ ]` / `- [x]`.
    pub tasklist: bool,
    /// Footnote references `[^a]` and definitions `[^a]: …`.
    pub footnotes: bool,
    /// GFM alerts `> [!NOTE]`, parsed as [`Callout`](crate::ast::NodeKind::Callout)s.
    pub alerts: bool,
    /// `$…$` and `$$…$$` math.
    pub math: bool,
    /// `==highlight==`.
    pub highlight: bool,
    /// `^superscript^`.
    pub superscript: bool,
    /// `~subscript~` (single tildes; double tildes stay strikethrough).
    pub subscript: bool,
    /// Definition lists: a term line followed by `: details`.
    pub description_lists: bool,
}

impl ParseOptions {
    /// Plain `CommonMark`: no extensions.
    pub const fn commonmark() -> Self {
        Self {
            strikethrough: false,
            table: false,
            autolink: false,
            tasklist: false,
            footnotes: false,
            alerts: false,
            math: false,
            highlight: false,
            superscript: false,
            subscript: false,
            description_lists: false,
        }
    }

    /// GitHub Flavored Markdown as rendered on github.com: tables, task lists,
    /// strikethrough, extended autolinks, footnotes, alerts and math.
    pub const fn gfm() -> Self {
        Self {
            strikethrough: true,
            table: true,
            autolink: true,
            tasklist: true,
            footnotes: true,
            alerts: true,
            math: true,
            ..Self::commonmark()
        }
    }
}

/// Parses `src` with a [`ComrakParser`] configured by `options`.
pub fn parse(src: &str, options: ParseOptions) -> Ast {
    ComrakParser::new(options).parse(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_commonmark() {
        assert_eq!(ParseOptions::default(), ParseOptions::commonmark());
        let gfm = ParseOptions::gfm();
        assert!(gfm.table && gfm.tasklist && gfm.strikethrough && gfm.autolink);
        assert!(gfm.footnotes && gfm.alerts && gfm.math);
        assert!(!gfm.highlight && !gfm.subscript && !gfm.description_lists);
    }
}
