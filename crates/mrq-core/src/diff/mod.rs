//! The model behind `mrq diff`: [`normalize`] turns a [`Document`] into a tree of
//! [`Section`]s whose [`Block`]s are in a canonical form, so that two documents that
//! differ only in formatting compare equal block by block.
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::diff;
//!
//! let old = diff::normalize(&Document::parse("# Notes\n\n* one\n* two\n\nA long\nline.\n"));
//! let new = diff::normalize(&Document::parse("Notes\n=====\n\n- one\n- two\n\nA long line.\n"));
//! let canonical = |root: &diff::Section| -> Vec<String> {
//!     root.iter()
//!         .flat_map(|s| s.heading.iter().chain(&s.blocks))
//!         .map(|block| block.canonical.clone())
//!         .collect()
//! };
//! assert_eq!(canonical(&old), ["# Notes", "- one\n- two", "A long line."]);
//! assert_eq!(canonical(&old), canonical(&new));
//! ```
//!
//! # Sections
//!
//! Headings at the top level of the document (not those in block quotes, lists or
//! other containers) open sections. A section holds the blocks after its heading up to
//! the next heading, and the sections of the deeper headings that follow, up to the
//! next heading of its level or a shallower one; a skipped level (`###` right under
//! `#`) nests one step. The root section, level 0 without a heading, holds what comes
//! before the first heading (frontmatter included) and the top-level sections.
//!
//! # Canonical form
//!
//! Every top-level block is printed by the [`fmt`](crate::fmt) printer on its own, with
//! the default markers (`-` bullets, `*` emphasis, `**` strong, backtick fences,
//! ordered lists counting up from their start number), `wrap: never` (each paragraph
//! on one line, whitespace runs as one space) and normalized frontmatter, without the
//! final newline. On top of what fmt normalizes (ATX headings, fenced code, `---`
//! breaks, escaping, reference definitions):
//!
//! - each block is printed as if it followed another one, so it does not depend on its
//!   neighbors (a list after a list keeps `-`, a thematic break first in the document
//!   is `---`);
//! - tables are not padded: `| a | b |`, with the narrowest delimiter cells (`---`,
//!   `:-:`), so re-aligning a table or widening one cell changes nothing or one row;
//! - ordered lists are delimited by `.` (`1)` becomes `1.`);
//! - fenced code is kept as written, without `fmt.code_formatters`.
//!
//! What changes the rendered HTML changes the canonical form: the text, inline markup,
//! list kind and start number, loose versus tight lists, task states, table alignments,
//! heading levels, code. Link reference definitions at the top level are blocks of
//! their own ([`NodeType::LinkDefinition`]), printed from what their links resolve to;
//! those inside containers are part of the container's block.

use std::ops::Range;

use crate::Document;
use crate::ast::{NodeKind, NodeType, Span};
use crate::config::{FmtConfig, FrontmatterFormat, Wrap};
use crate::fmt::{self, Piece};
use crate::source::{PositionEncoding, SourceFile};

/// A heading and what belongs to it; see [Sections](self#sections).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The heading; `None` for the root section.
    pub heading: Option<Block>,
    /// The heading's plain text ([`Ast::text`](crate::ast::Ast::text)); empty for the
    /// root section.
    pub title: String,
    /// 1–6, or 0 for the root section.
    pub level: u8,
    /// The titles of the enclosing sections' headings and of this one, from the top;
    /// empty for the root section.
    pub path: Vec<String>,
    /// From the start of the heading to the start of the next heading that ends the
    /// section, or the end of the document; the whole document for the root section.
    pub span: Span,
    /// The blocks between the heading and the next heading, in document order.
    pub blocks: Vec<Block>,
    /// The sections of the deeper headings, in document order.
    pub children: Vec<Self>,
}

impl Section {
    /// This section and every section below it, depth first in document order.
    pub fn iter(&self) -> impl Iterator<Item = &Self> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let section = stack.pop()?;
            stack.extend(section.children.iter().rev());
            Some(section)
        })
    }
}

/// A top-level block (or link reference definition) of a document in its canonical
/// form; see [Canonical form](self#canonical-form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// What the block is.
    pub kind: NodeType,
    /// The block as printed, lines separated by `\n`, without a final newline.
    pub canonical: String,
    /// The block's source; for a reference definition, from its label to the end of
    /// the line its destination ends on.
    pub span: Span,
    /// The source lines the block is on, counted from 0 as in
    /// [`SourceFile`], end exclusive.
    pub lines: Range<usize>,
}

/// The section tree of `document` with every block in its canonical form.
pub fn normalize(document: &Document) -> Section {
    let config = FmtConfig {
        wrap: Wrap::Never,
        frontmatter: FrontmatterFormat::Normalize,
        ..FmtConfig::default()
    };
    let ast = document.ast();
    let source = document.source();
    let len = document.text().len();
    let mut open = vec![Section {
        heading: None,
        title: String::new(),
        level: 0,
        path: Vec::new(),
        span: Span::new(0, len),
        blocks: Vec::new(),
        children: Vec::new(),
    }];
    for piece in fmt::pieces(document, &config) {
        let (kind, span, canonical, heading) = match piece {
            Piece::Block(id, lines) => {
                let heading = match ast.kind(id) {
                    NodeKind::Heading(heading) => Some((heading.level, ast.text(id))),
                    _ => None,
                };
                (
                    ast.kind(id).node_type(),
                    ast.span(id),
                    lines.join("\n"),
                    heading,
                )
            }
            Piece::Definition(span, line) => (NodeType::LinkDefinition, span, line, None),
        };
        let block = Block {
            kind,
            canonical,
            lines: lines(source, span),
            span,
        };
        let Some((level, title)) = heading else {
            open.last_mut()
                .expect("the root stays open")
                .blocks
                .push(block);
            continue;
        };
        while open.last().is_some_and(|section| section.level >= level) {
            close(&mut open, span.start);
        }
        let mut path = open.last().expect("the root stays open").path.clone();
        path.push(title.clone());
        open.push(Section {
            heading: Some(block),
            title,
            level,
            path,
            span: Span::new(span.start, len),
            blocks: Vec::new(),
            children: Vec::new(),
        });
    }
    while open.len() > 1 {
        close(&mut open, len);
    }
    open.pop().expect("the root stays open")
}

/// Ends the innermost open section at `end` and adds it to its parent.
fn close(open: &mut Vec<Section>, end: usize) {
    let mut section = open.pop().expect("a section to close");
    section.span.end = end;
    open.last_mut()
        .expect("the root stays open")
        .children
        .push(section);
}

/// The lines `span` is on in `source`, end exclusive (one line for an empty span).
fn lines(source: &SourceFile, span: Span) -> Range<usize> {
    let line = |offset| {
        source
            .offset_to_position(offset, PositionEncoding::Utf8)
            .map_or(0, |position| position.line)
    };
    let first = line(span.start);
    // The line of the last character.
    let last = source.text()[span.start..span.end]
        .char_indices()
        .next_back()
        .map_or(first, |(at, _)| line(span.start + at));
    first..last + 1
}
