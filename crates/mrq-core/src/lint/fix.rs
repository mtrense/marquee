//! Applying fixes: one pass over a set of findings, and [`Linter::fix`]'s loop to a
//! fixpoint.

use std::path::Path;

use super::Linter;
use crate::Document;
use crate::diag::Diagnostic;
use crate::document::DocumentOptions;
use crate::flavor::FlavorError;
use crate::source::{EditError, SourceFile, TextEdit, apply_edits};

/// How many times [`Linter::fix`] fixes, re-parses and re-lints at most.
pub const MAX_FIX_PASSES: usize = 10;

/// `text` after applying the fixes of `diagnostics` that do not overlap an earlier one.
///
/// Fixes are taken in the order of their first edit (ties in the order of
/// `diagnostics`, which [`Linter::check`] sorts by position and rule), each fix as a
/// whole: one starting before the previous applied fix ends is skipped, to be redone on
/// the next pass. Like markdownlint, identical fixes (two rules or findings inserting the
/// same blank line) apply once. Findings without a fix are ignored.
///
/// # Errors
///
/// A fix's edits are out of bounds, not on a char boundary or overlap each other (a bug
/// in the rule).
pub fn apply_fixes(text: &str, diagnostics: &[Diagnostic]) -> Result<String, EditError> {
    let start = |edits: &[TextEdit]| edits.iter().map(|edit| edit.range.start).min();
    let mut fixes: Vec<&[TextEdit]> = diagnostics
        .iter()
        .filter_map(|diag| diag.fix.as_ref().map(|fix| &fix.edits[..]))
        .filter(|edits| !edits.is_empty())
        .collect();
    fixes.sort_by_key(|edits| start(edits));
    let mut edits = Vec::new();
    let mut end = 0;
    // The applied fixes starting where the last one does; identical fixes start alike.
    let mut same_start: Vec<&[TextEdit]> = Vec::new();
    for fix in fixes {
        let first = start(fix).unwrap_or_default();
        if same_start.first().and_then(|other| start(other)) != Some(first) {
            same_start.clear();
        }
        if same_start.contains(&fix) || (first < end && !edits.is_empty()) {
            continue;
        }
        end = fix.iter().map(|edit| edit.range.end).max().unwrap_or(first);
        edits.extend(fix.iter().cloned());
        same_start.push(fix);
    }
    apply_edits(text, &edits)
}

/// Why [`Linter::fix`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FixError {
    /// A rule produced edits that do not apply.
    #[error("{name}: invalid fix: {source}")]
    Edit {
        /// The document's name.
        name: String,
        source: EditError,
    },
    /// The fixed text could not be parsed again with the options.
    #[error(transparent)]
    Flavor(#[from] FlavorError),
}

/// The result of [`Linter::fix`].
#[derive(Debug, Clone)]
pub struct Fixed {
    /// The fixed document, parsed again; its text is the original's when nothing was
    /// fixed.
    pub document: Document,
    /// Every finding left in the fixed document, as [`Linter::check`] reports them.
    pub diagnostics: Vec<Diagnostic>,
    /// How many passes changed the text.
    pub passes: usize,
}

impl Linter {
    /// Fixes `doc` until no fix changes it: applies the non-overlapping fixes of every
    /// finding ([`apply_fixes`]), parses the text again with `options` under the same
    /// path, lints it again, and repeats, at most [`MAX_FIX_PASSES`] times. Each rule
    /// fixes what it sees in one pass, so overlapping fixes, and findings that only show
    /// up after another fix, are taken care of by the following passes.
    ///
    /// # Errors
    ///
    /// A rule's edits do not apply, or `options` cannot parse the fixed text (an unknown
    /// forced or configured flavor).
    pub fn fix(&self, doc: Document, options: &DocumentOptions) -> Result<Fixed, FixError> {
        let mut document = doc;
        let mut diagnostics = self.check(&document);
        let mut passes = 0;
        while passes < MAX_FIX_PASSES && diagnostics.iter().any(|diag| diag.fix.is_some()) {
            let text =
                apply_fixes(document.text(), &diagnostics).map_err(|source| FixError::Edit {
                    name: document.name().display().to_string(),
                    source,
                })?;
            if text == document.text() {
                break;
            }
            let path = document.path().map(Path::to_path_buf);
            document = options.parse_source(SourceFile::new(path, text))?;
            diagnostics = self.check(&document);
            passes += 1;
        }
        Ok(Fixed {
            document,
            diagnostics,
            passes,
        })
    }
}
