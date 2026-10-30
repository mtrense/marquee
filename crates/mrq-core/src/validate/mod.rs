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
//! A [`Checker`] reports what is broken inside a workspace, without network access unless
//! [external links](#external-links) are enabled. Each
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
//! | `frontmatter-schema` | error | the offending value; a missing key at its parent's key (the opening `---`/`+++` for top-level keys, the start of a document without frontmatter); an unexpected key at the key | the frontmatter does not match a JSON Schema configured for the file (`validate.frontmatter`); types, enums, consts and required keys name what was expected |
//! | `missing-include` | error | the `path=…` attribute; the start marker if there is none | an `mrq:include` has no `path`, or its file does not exist |
//! | `include-cycle` | error | the `path=…` attribute, with the chain of files in the help | following `mrq:include` paths from the directive leads back to its own file |
//! | `stale-toc` | warning | the start marker | an `mrq:toc` region differs from what `mrq toc --insert` writes (also a bare start marker), as [`toc::refresh_edits`](crate::toc::refresh_edits) finds; an invalid `depth` is a `directive` error |
//! | `broken-external-link` | error | the link or image | an http(s) URL is broken (an HTTP error status after retries, a timeout, a connection or TLS error), as the [`ExternalChecker`] reports; only with [`Checker::with_external_links`], see [External links](#external-links) |
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
//! - External URLs are only checked when enabled (see below); `mailto:` addresses never.
//! - **Includes** resolve like file links (relative to the including file, a leading
//!   `/` from the root, no percent-decoding) and exist if they are in the workspace or a
//!   file on disk. Cycles are found by following the `mrq:include`s of Markdown files
//!   (from the workspace, else loaded from disk), comparing canonicalized paths; each
//!   file on a cycle reports it, a file that merely includes one does not. `http(s)://`
//!   paths are skipped until remote includes exist (M7.7). The `section` and `select`
//!   attributes are not checked. Whether an include region's content matches its
//!   expansion is not checked either: that needs the include resolver (M7.4), so
//!   nothing is reported until then (see the TODO in `directives.rs`).
//!
//! # Frontmatter schemas
//!
//! With the `frontmatter-schema` feature (on by default), `FrontmatterSchemas` maps
//! globs to JSON Schemas (drafts 4 to 2020-12, picked by `$schema`, else 2020-12) and
//! [`Checker::with_frontmatter_schemas`] adds them to the checks. `from_config` reads
//! `validate.frontmatter`, resolving each entry's `files` glob and `schema` path against
//! the directory of the config file that declares it. Instance paths of validation errors
//! are mapped to frontmatter spans with
//! [`Frontmatter::entry_at_pointer`](crate::frontmatter::Frontmatter::entry_at_pointer).
//! Schemas cannot `$ref` other files or URLs: nothing is read besides the schema file
//! itself, and nothing is fetched from the network.
//!
//! # External links
//!
//! External links are checked only on request: a [`Checker`] checks none unless
//! [`Checker::with_external_links`] gives it an [`ExternalChecker`], the seam to the
//! network that tests and the LSP can substitute. [`ExternalOptions`] holds whether to
//! check at all (`validate.external`, or `--external` through
//! [`ExternalOptions::with_flag`]) and the `validate.external_exclude` patterns
//! ([`ExternalExclude`], regular expressions matched anywhere in the URL). Only `http:` and
//! `https:` URLs that no pattern excludes reach the checker, each distinct URL once, as
//! written (with any `#fragment`); results are remembered for the checker's lifetime.
//!
//! With the `external-links` feature (off by default), `HttpChecker` checks them over
//! HTTP(S) with reqwest and rustls (never OpenSSL) on a tokio runtime of its own, as
//! `HttpOptions` sets: retries with exponential backoff, a request timeout, a total and a
//! per-host concurrency limit, a minimum interval between requests to one host, and an
//! optional on-disk cache (`CacheOptions`, by default `external-links.json` in the user
//! cache directory, [`user_cache_dir`](crate::config::user_cache_dir), for one day) of
//! the URLs found working. Building one sends nothing.
//!
//! ```
//! use mrq_core::{Document, Workspace};
//! use mrq_core::config::Config;
//! use mrq_core::validate::{Checker, ExternalChecker, ExternalOptions, ExternalStatus, codes};
//!
//! #[derive(Debug)]
//! struct Offline;
//! impl ExternalChecker for Offline {
//!     fn check(&self, urls: &[&str]) -> Vec<ExternalStatus> {
//!         urls.iter().map(|_| ExternalStatus::Broken("offline".into())).collect()
//!     }
//! }
//!
//! let options = ExternalOptions::from_config(&Config::default())?.with_flag(true);
//! assert!(options.enabled);
//! let ws = Workspace::new(".");
//! let doc = Document::parse("[x](https://example.org/)\n");
//! let diagnostics = Checker::new(&ws).with_external_links(&Offline, &options.exclude).check(&doc);
//! assert_eq!(diagnostics[0].code, codes::BROKEN_EXTERNAL_LINK);
//! # Ok::<(), mrq_core::validate::ExcludeError>(())
//! ```

mod brackets;
mod check;
mod directives;
mod external;
#[cfg(feature = "external-links")]
mod http;
mod links;
mod refdefs;
#[cfg(feature = "frontmatter-schema")]
mod schema;

pub use check::{Checker, codes};
pub use external::{
    ExcludeError, ExternalChecker, ExternalExclude, ExternalOptions, ExternalStatus,
};
#[cfg(feature = "external-links")]
pub use http::{CacheOptions, HttpChecker, HttpError, HttpOptions};
pub(crate) use links::scheme;
pub use links::{DocumentLink, Target, links};
pub use refdefs::{ReferenceDefinition, reference_definitions};
#[cfg(feature = "frontmatter-schema")]
pub use schema::{FrontmatterSchemas, SchemaError};
