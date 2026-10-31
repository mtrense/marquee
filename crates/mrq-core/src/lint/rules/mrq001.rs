//! MRQ001 `valid-anchors`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::diag::{Diagnostic, Severity};
use crate::document::DocumentOptions;
use crate::lint::{Rule, RuleMeta};
use crate::validate::Checker;
use crate::{Document, Workspace};

/// Link to a missing anchor.
///
/// The lint mirror of `mrq validate`'s `missing-anchor`, with the same resolution and
/// messages: anchors come from the document's flavor (its slugger, `{#id}` attributes,
/// Obsidian's case-insensitive headings and `^block` ids), where MD051 always uses
/// markdownlint's GitHub slugs. Checked are links and images to `#anchor` and wikilinks
/// to `[[#heading]]` in the document itself and, with `cross_file` (the default), links
/// to `file.md#anchor`: the target is read from disk relative to the document (a leading
/// `/` from the current directory) and parsed with the document's flavor unless its
/// frontmatter picks another. Documents without a path (stdin) resolve from the current
/// directory. Targets that do not exist or are not Markdown are skipped (missing files
/// are `mrq validate`'s business), and so are wikilinks to other notes, which need the
/// vault-wide lookup only `mrq validate` does. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mrq001;

/// Options of MRQ001 `valid-anchors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Mrq001Options {
    /// Also check anchors in other Markdown files, read from disk.
    pub cross_file: bool,
}

impl Default for Mrq001Options {
    fn default() -> Self {
        Self { cross_file: true }
    }
}

impl Rule for Mrq001 {
    type Options = Mrq001Options;

    const META: RuleMeta = RuleMeta {
        id: "MRQ001",
        aliases: &["valid-anchors"],
        tags: &["links", "mrq"],
        description: "Link to a missing anchor",
        // MD051 reports the same-file cases already; enable this one instead of it for
        // flavor-aware and cross-file checks.
        default_enabled: false,
    };

    fn check(&self, doc: &Document, options: &Mrq001Options) -> Vec<Diagnostic> {
        let targets = DocumentOptions::new().config_flavor(doc.flavor().name());
        let workspace = Workspace::with_options(".", targets);
        Checker::new(&workspace)
            .missing_anchors(doc, options.cross_file)
            .into_iter()
            .map(|found| Diagnostic {
                code: Self::META.id.to_owned(),
                severity: Severity::Warning,
                ..found
            })
            .collect()
    }
}
