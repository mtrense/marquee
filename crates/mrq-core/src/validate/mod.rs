//! Validation of documents and workspaces.
//!
//! The checks build on [`links`]: every link, image, wikilink and embed of a document,
//! classified as a [`Target`] (same-document anchor, local file with an optional anchor,
//! external URL, `mailto:` address, wikilink name, or empty), with reference links
//! marked by their label and file paths resolved against the [`Workspace`](crate::Workspace).
//!
//! ```
//! use std::path::PathBuf;
//! use mrq_core::document::{DocumentOptions, Workspace};
//! use mrq_core::source::SourceFile;
//! use mrq_core::validate::{self, Target};
//!
//! let ws = Workspace::new("site");
//! let source = SourceFile::new(Some(PathBuf::from("site/docs/a.md")), "See [setup](/guide.md#setup).\n");
//! let doc = DocumentOptions::new().parse_source(source)?;
//! let links = validate::links(&doc, &ws);
//! assert_eq!(
//!     links[0].target,
//!     Target::File { path: "site/guide.md".into(), anchor: Some("setup".into()) }
//! );
//! # Ok::<(), mrq_core::flavor::FlavorError>(())
//! ```
//!
//! # Internal checks
//!
//! A [`Checker`] reports what is broken inside a workspace, without network access. Each
//! [`Diagnostic`](crate::diag::Diagnostic) carries one of these [`codes`] (plain words,
//! like the parser's `frontmatter` and `directive`; `MRQ###` is for lint rules):
//!
//! | Code | Severity | Reported at | When |
//! | --- | --- | --- | --- |
//! | `missing-file` | error | the link | a link's local file (or directory) does not exist |
//! | `missing-asset` | error | the image | an image's local file does not exist |
//! | `missing-anchor` | error | the link | the `#fragment` matches no heading anchor of the target (made by the target's flavor slugger), or a wikilink's `#Heading` / `#^block-id` is not in the note |
//! | `duplicate-anchor` | warning | the later heading, labelled with the first | two headings of a document have the same anchor (explicit `{#id}`s, or headings differing only in case where anchors ignore case) |
//! | `undefined-footnote` | error | `[^label]` | a footnote reference has no definition |
//! | `unused-footnote` | warning | the definition's `[^label]` | a footnote definition is never referenced |
//! | `undefined-reference` | error | `[label]` of `[text][label]`, or `[label][]` | a full or collapsed reference link has no definition (shortcut `[label]` is ordinary text) |
//! | `unused-reference` | warning | the definition | a link reference definition is never used |
//! | `unresolved-wikilink` | warning | the wikilink or embed | no file in the vault has the name |
//!
//! How targets are found:
//!
//! - **Files** resolve as [`Target::resolve`] does, lexically; a file exists if it is in
//!   the workspace or on disk. A link to an existing directory (`docs/`) is fine, and
//!   its anchor is not checked. Anchors are only checked in Markdown files (`.md`,
//!   `.markdown`); files outside the workspace are loaded for that. An empty fragment
//!   (`#`) is the top of the document.
//! - **Anchors** are the [heading anchors](crate::Document::anchors) only, so links to
//!   HTML `id`s or to footnotes are reported as missing.
//! - **Wikilinks** name a file of the vault, the workspace root: the shortest path that
//!   ends with the name, or with the name plus `.md`, in whole path components and
//!   ignoring case (`[[Note]]`, `[[folder/Note]]`, `[[Note.md]]`, `[[pic.png]]`). A
//!   heading anchor is compared with the note's anchors after the same normalization
//!   (only the last part of `#A#B` counts); `#^id` needs a line ending in `^id`.
//! - **Footnotes and references** that have no definition are not links in the tree;
//!   they are found in the text. Reference definitions are recovered from the source
//!   by [`reference_definitions`]. Labels match as in `CommonMark` (case and whitespace
//!   folded). Footnotes are only checked for flavors that have them.
//! - External URLs and `mailto:` addresses are not checked here.

mod brackets;
mod check;
mod links;
mod refdefs;

pub use check::{Checker, codes};
pub(crate) use links::scheme;
pub use links::{DocumentLink, Target, links};
pub use refdefs::{ReferenceDefinition, reference_definitions};
