//! Documents and workspaces: what every command works on.
//!
//! A [`Document`] is one parsed Markdown source: the [`SourceFile`](crate::source::SourceFile),
//! its [flavor](crate::flavor) and where that was chosen, the frontmatter, the syntax
//! tree after the flavor's passes, the diagnostics found while parsing, and a lazily
//! built [`AnchorIndex`] of its headings.
//!
//! [`Document::parse`] and [`Document::load`] resolve the flavor from the frontmatter
//! (`mrq.flavor`), else `gfm`. [`DocumentOptions`] adds `--flavor`, the configured flavor
//! and custom registries. Parsing never fails on the document's content: malformed
//! frontmatter and a bad frontmatter flavor are [diagnostics](Document::diagnostics).
//!
//! ```
//! use mrq_core::Document;
//!
//! let doc = Document::parse("---\nmrq:\n  flavor: obsidian\n---\n# My Notes\n\nSee [[Other#Top]].\n");
//! assert_eq!(doc.flavor().name(), "obsidian");
//! assert!(doc.diagnostics().is_empty());
//! assert!(doc.anchors().contains("my notes")); // Obsidian anchors ignore case
//! assert_eq!(doc.toc()[0].text, "My Notes");
//! ```
//!
//! A [`Workspace`] holds the documents of one run, resolves link paths between them and
//! loads link targets on demand.

mod anchors;
mod doc;
mod workspace;

pub use crate::toc::TocEntry;
pub use anchors::AnchorIndex;
pub use doc::{Document, DocumentError, DocumentOptions};
pub use workspace::{Workspace, normalize_path};
pub(crate) use workspace::{normalize, resolve};
