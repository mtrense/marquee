//! The `obsidian` flavor: GFM plus Obsidian's wikilinks, embeds, callouts, tags, `%%`
//! comments, math and highlights.
//!
//! comrak parses the GFM base, `$…$`/`$$…$$` math and `==highlight==`; three passes
//! recognize the rest, in this order:
//!
//! 1. `callouts` ([`callouts`]): a block quote whose first line is `[!kind]`, optionally
//!    followed by a fold marker `+`/`-` and a title, becomes a
//!    [`Callout`](crate::ast::NodeKind::Callout). comrak's GFM alerts are switched off for
//!    this flavor: they know only the five GitHub kinds and no fold markers, so the pass
//!    handles `> [!NOTE]` as well, and both syntaxes give the same node.
//! 2. `block_comments` ([`comments`]): a `%%` comment that opens a paragraph and closes at
//!    the end of a later sibling paragraph becomes one block-level
//!    [`Comment`](crate::ast::NodeKind::Comment) in place of everything in between.
//! 3. `inlines` ([`inlines`]): wikilinks, embeds, tags and `%%` comments within a
//!    paragraph are cut out of its text; a paragraph that is nothing but one `%%` comment
//!    becomes a block-level comment, like a lone HTML comment does.
//!
//! The user-facing summary is on [`Obsidian`]; the pass modules have the exact rules.

mod callouts;
mod comments;
mod inlines;

use super::{Flavor, ObsidianSlugger, Pass, Slugger};
use crate::parse::ParseOptions;

/// [Obsidian](https://obsidian.md) Markdown: GFM plus wikilinks, embeds, callouts, tags,
/// `%%` comments, math and highlights.
///
/// Parses with [`ParseOptions::gfm`] plus `==highlight==` (math is on in GFM already) and
/// slugs with [`ObsidianSlugger`] (heading text, not GitHub slugs), matched regardless of
/// case ([`Flavor::case_insensitive_anchors`]). Three passes,
/// `callouts`, `block_comments` and `inlines`, recognize the rest:
///
/// - **Callouts**: a block quote whose first line starts with `[!kind]` becomes a
///   [`Callout`](crate::ast::Callout) with the kind as written, `fold` from a `+`/`-`
///   right after the `]`, and the rest of the line, trimmed, as verbatim `title`. The
///   marker line is removed from the body. GFM alerts are off for this flavor (comrak
///   knows only the five GitHub kinds and no fold markers), so `> [!NOTE]` goes through
///   the same pass and gives the same node.
/// - **Wikilinks and embeds**: `[[target#anchor|alias]]` and `![[…]]` become
///   [`WikiLink`](crate::ast::NodeKind::WikiLink) and
///   [`Embed`](crate::ast::NodeKind::Embed) nodes; the parts are trimmed, empty ones are
///   `None`, `\|` also separates the alias (for tables), the anchor keeps a `^` of a
///   block reference, and an embed's size (`|100x200`) is its alias. A link stays on one
///   line, contains no brackets and has a target or an anchor.
/// - **Tags**: `#name` at the start of text or after whitespace, where the name (letters,
///   digits, `_`, `-`, `/`) has a non-digit, becomes a [`Tag`](crate::ast::Tag).
/// - **`%%` comments**: `%%…%%` within a paragraph becomes an inline
///   [`Comment`](crate::ast::Comment) with [`CommentSyntax::Percent`] and the verbatim
///   source between the delimiters as literal. A paragraph that is only one comment
///   becomes a block-level comment, as a lone HTML comment does; so does a comment that
///   opens a paragraph and closes at the end of a later sibling paragraph, swallowing the
///   blocks in between.
///
/// [`CommentSyntax::Percent`]: crate::ast::CommentSyntax::Percent
///
/// Syntax is only found in text: never in code, math, raw HTML, link text or
/// destinations, and not when its first delimiter is backslash-escaped. Every node a pass
/// creates spans exactly its source.
///
/// Limits: comrak parses inlines first, so a wikilink or comment that emphasis split
/// across nesting levels (`*[[a*]]`), or whose `[label]` resolved as a reference link, is
/// not recognized; a block comment that opens or closes mid-paragraph, or at another
/// nesting level than its other end, stays text.
#[derive(Debug, Clone, Copy, Default)]
pub struct Obsidian;

impl Flavor for Obsidian {
    fn name(&self) -> &'static str {
        "obsidian"
    }

    fn parse_options(&self) -> ParseOptions {
        ParseOptions {
            alerts: false,
            highlight: true,
            ..ParseOptions::gfm()
        }
    }

    fn slugger(&self) -> Box<dyn Slugger> {
        Box::new(ObsidianSlugger::new())
    }

    fn case_insensitive_anchors(&self) -> bool {
        true
    }

    fn passes(&self) -> &[&dyn Pass] {
        &[
            &callouts::Callouts,
            &comments::BlockComments,
            &inlines::Inlines,
        ]
    }
}
