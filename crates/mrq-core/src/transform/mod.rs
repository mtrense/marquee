//! Structural transforms of documents, all expressed as [`TextEdit`](crate::source::TextEdit)s
//! against the original source so untouched bytes stay as they were.
//!
//! - [`shift_headings`]: push the headings in a byte range down or up by N levels
//!   (used by `merge --shift`, `mrq:include shift=N`, `split` and the TUI's
//!   promote/demote).
//! - [`rewrite_links`]: make the links in a byte range work from another file, by
//!   rebasing relative paths and redirecting what a [`LinkMap`] says moved (used by
//!   `merge`, `mrq:include` and `split`).
//! - [`merge`]: combine documents into one, each file a section, with headings shifted,
//!   links rebased and cross-file links turned into anchors of the merged document
//!   (`mrq merge`).
//! - [`Includer`]: resolve `mrq:include` directives, recursively, with `section`,
//!   `select`, `shift`, `dedent` and `frontmatter`, and code includes of whole files,
//!   line ranges or symbols ([`symbol`](crate::symbol)) (`mrq interpolate`).
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::source::{ByteRange, apply_edits};
//! use mrq_core::transform::{ShiftOptions, shift_headings};
//!
//! let src = "# Guide\n\nIntro\n---\n";
//! let doc = Document::parse(src);
//! let shifted = shift_headings(&doc, ByteRange::new(0, src.len()), ShiftOptions::by(1));
//! assert_eq!(apply_edits(src, &shifted.edits).unwrap(), "## Guide\n\n### Intro\n");
//! ```
//!
//! Merging `docs/a.md` and `docs/b.md` into `guide.md`, where `b.md`'s `#setup`
//! becomes `#setup-1`:
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::source::{ByteRange, apply_edits};
//! use mrq_core::transform::{LinkMap, LinkRewrite, Location, rewrite_links};
//!
//! let mut map = LinkMap::new();
//! map.insert_file("docs/b.md", Location::output().with_anchor("b"));
//! map.insert_anchor("docs/b.md", "setup", Location::output().with_anchor("setup-1"));
//! let src = "[b](b.md), [setup](b.md#setup), ![logo](img/logo.png?v=2)\n";
//! let doc = Document::parse(src);
//! let rewrite = LinkRewrite::new("docs/a.md", "guide.md").map(&map);
//! let edits = rewrite_links(&doc, ByteRange::new(0, src.len()), &rewrite);
//! assert_eq!(
//!     apply_edits(src, &edits).unwrap(),
//!     "[b](#b), [setup](#setup-1), ![logo](docs/img/logo.png?v=2)\n"
//! );
//! ```

mod include;
mod links;
mod merge;
mod shift;

pub use include::{Expanded, FileLoader, INCLUDE_CODE, IncludeTarget, Includer, Inclusion, Loader};
pub use links::{LinkMap, LinkRewrite, Location, rewrite_links};
pub use merge::{FrontmatterMerge, MERGE_ANCHORS_CODE, MergeOptions, Merged, merge};
pub use shift::{HEADING_CLAMP_CODE, Overflow, ShiftOptions, Shifted, shift_headings};
