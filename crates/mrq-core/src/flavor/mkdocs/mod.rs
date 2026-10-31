//! The `mkdocs` flavor: GFM plus the Python-Markdown / pymdown-extensions syntax that
//! mkdocs sites (usually with Material for `MkDocs`) rely on.
//!
//! comrak parses the GFM base, `$…$`/`$$…$$` math, `==highlight==` and definition lists;
//! five passes do the rest, in this order:
//!
//! 1. `admonitions` ([`admonitions`]): `!!!`/`???` headers with indented bodies become
//!    [`Admonition`](crate::ast::NodeKind::Admonition)s. comrak sees the header as a
//!    paragraph and the body as lazy continuation lines or an indented code block, so
//!    the pass re-scans the source lines, dedents the body and re-parses it as a nested
//!    document (with all passes, so admonitions nest).
//! 2. `heading_attrs` ([`attrs`]): a trailing `{#id .class key=value}` is cut from a
//!    heading's text into its [`HeadingAttrs`](crate::ast::HeadingAttrs).
//! 3. `definition_lists` ([`deflists`]): comrak gives a second `:` definition of a term
//!    an item of its own; the pass moves it under the term's item.
//! 4. `abbreviations` ([`abbreviations`]): `*[abbr]: expansion` lines are cut out of
//!    paragraphs into
//!    [`AbbreviationDefinition`](crate::ast::NodeKind::AbbreviationDefinition)s.
//! 5. `math` ([`math`]): arithmatex's `\(…\)` and `\[…\]` become
//!    [`Math`](crate::ast::NodeKind::Math).
//!
//! Admonitions run first: the other passes would otherwise rewrite the header paragraph
//! and its lazy body lines. Admonition bodies have run every pass already when they are
//! imported; the later passes leave such nodes alone (they only match unconverted
//! syntax), so running them again over the whole tree is harmless.
//!
//! The printer hooks ([`printer`]) escape text that the passes would read as an
//! admonition header or heading attributes.
//!
//! The user-facing summary is on [`Mkdocs`]; the pass modules have the exact rules.

mod abbreviations;
mod admonitions;
mod attrs;
mod deflists;
mod math;
mod printer;

use super::{Flavor, MkdocsSlugger, Pass, PrinterHooks, Slugger};
use crate::parse::ParseOptions;

/// [mkdocs](https://www.mkdocs.org) Markdown as configured by most sites (Material for
/// `MkDocs`' recommended extensions): GFM plus admonitions, heading attributes, definition
/// lists, abbreviations, math and highlights.
///
/// Parses with [`ParseOptions::gfm`] minus GFM alerts, plus definition lists and
/// `==highlight==`, and slugs with [`MkdocsSlugger`] (Python-Markdown's `toc` ids; use
/// [`heading_anchors`](super::heading_anchors) so `{#id}` wins). Five passes,
/// `admonitions`, `heading_attrs`, `definition_lists`, `abbreviations` and `math`,
/// recognize the rest:
///
/// - **Admonitions** (`admonition`, `pymdownx.details`): a line `!!! kind "Title"`
///   (`???` collapsed, `???+` open) followed by a body indented 4 spaces (or one tab)
///   more than the header becomes an [`Admonition`](crate::ast::Admonition). Extra words
///   after the kind are `classes`; the title is the verbatim text between the first and
///   the last `"`, `""` being an explicitly empty title. The body ends before the first
///   non-blank line with less indent and may hold any blocks, nested admonitions
///   included; its nodes span the original, indented source. Admonitions are found in
///   any container (list items, quotes, definitions, footnotes) and on any line of a
///   paragraph, as in Python-Markdown; lines left over after a body are re-parsed.
/// - **Heading attributes** (`attr_list`): `{#id .class key=value}` (or `{: …}`) after at
///   least one space at the end of a heading sets
///   [`Heading::attrs`](crate::ast::Heading::attrs) and is cut from the heading text.
///   `key="quoted value"` and `'…'` work; `id=…` sets the id and `class=…` adds
///   classes; a bare word `w` is the attribute `w=w`.
/// - **Definition lists** (`def_list`): `Term` lines followed by `:   details`; every
///   detail of a term lives in the term's
///   [`DefinitionItem`](crate::ast::NodeKind::DefinitionItem).
/// - **Abbreviations** (`abbr`): a paragraph line `*[abbr]: expansion` (an optional space
///   before the `:`) becomes a block-level
///   [`AbbreviationDefinition`](crate::ast::AbbreviationDefinition) with the trimmed
///   expansion, splitting the paragraph around it. Uses of the abbreviation in text are
///   not marked.
/// - **Math** (`pymdownx.arithmatex`): `$…$` and `$$…$$` (comrak), plus `\(…\)` (inline)
///   and `\[…\]` (display) whose delimiters lie in text of the same parent; the literal
///   is the verbatim source between the delimiters. `\begin{…}…\end{…}` blocks are not
///   recognized.
///
/// Limits: tabs only count as indentation where a body's 4-space step is expected; an
/// admonition header inside a multi-line inline (emphasis spanning lines) is not found;
/// attribute lists are only read on headings (not on paragraphs, links or images); math
/// delimiters split across emphasis nesting (`*\(a* \)`) are not recognized. Python-
/// Markdown's 4-space list indentation is not emulated: list structure is `CommonMark`'s.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mkdocs;

impl Flavor for Mkdocs {
    fn name(&self) -> &'static str {
        "mkdocs"
    }

    fn parse_options(&self) -> ParseOptions {
        ParseOptions {
            alerts: false,
            highlight: true,
            description_lists: true,
            ..ParseOptions::gfm()
        }
    }

    fn slugger(&self) -> Box<dyn Slugger> {
        Box::new(MkdocsSlugger::new())
    }

    fn passes(&self) -> &[&dyn Pass] {
        &[
            &admonitions::Admonitions,
            &attrs::HeadingAttributes,
            &deflists::DefinitionLists,
            &abbreviations::Abbreviations,
            &math::BackslashMath,
        ]
    }

    fn printer_hooks(&self) -> Option<&dyn PrinterHooks> {
        Some(&printer::MkdocsPrinter)
    }
}
