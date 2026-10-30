//! Source text, positions and byte-range edits.
//!
//! Everything in mrq addresses source text by **byte offsets** into the original UTF-8
//! text ([`ByteRange`]). [`SourceFile`] converts those to and from (line, column)
//! [`Position`]s, with columns in UTF-8 bytes or UTF-16 code units (for LSP).
//! Modifications (`lint --fix`, `interpolate --in-place`, `toc --insert`, LSP edits) are
//! expressed as [`TextEdit`]s against the original text and applied with [`apply_edits`],
//! so bytes outside the edited ranges are preserved exactly.

mod edit;
mod file;

pub use edit::{ByteRange, EditError, TextEdit, apply_edits};
pub use file::{Position, PositionEncoding, PositionError, SourceFile};
