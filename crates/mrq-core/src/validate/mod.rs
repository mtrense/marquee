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

mod links;

pub(crate) use links::scheme;
pub use links::{DocumentLink, Target, links};
