//! Structural transforms of documents, all expressed as [`TextEdit`](crate::source::TextEdit)s
//! against the original source so untouched bytes stay as they were.
//!
//! - [`shift_headings`]: push the headings in a byte range down or up by N levels
//!   (used by `merge --shift`, `mrq:include shift=N`, `split` and the TUI's
//!   promote/demote).
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

mod shift;

pub use shift::{HEADING_CLAMP_CODE, Overflow, ShiftOptions, Shifted, shift_headings};
