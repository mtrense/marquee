//! MD024 `no-duplicate-heading`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::Diagnostic;
use crate::lint::{Rule, RuleMeta};

use super::{ellipsify, heading_line, heading_text, message};

/// Multiple headings with the same content.
///
/// A heading whose text (its source without inline HTML) repeats an earlier heading's
/// is reported; with `siblings_only`, only earlier headings under the same parent
/// heading count. No fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md024;

/// Options of MD024 `no-duplicate-heading`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md024Options {
    /// Only check sibling headings.
    pub siblings_only: bool,
}

impl Rule for Md024 {
    type Options = Md024Options;

    const META: RuleMeta = RuleMeta {
        id: "MD024",
        aliases: &["no-duplicate-heading"],
        tags: &["headings"],
        description: "Multiple headings with the same content",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md024Options) -> Vec<Diagnostic> {
        let ast = doc.ast();
        // The texts seen so far, per level with `siblings_only` (index 0 otherwise).
        let mut known: Vec<Vec<String>> = vec![Vec::new(); 7];
        let mut last_level = 1;
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let NodeKind::Heading(heading) = ast.kind(id) else {
                continue;
            };
            let text = heading_text(doc, id);
            let level = if options.siblings_only {
                let level = usize::from(heading.level);
                // Entering a deeper level or leaving one forgets what was below.
                while last_level < level {
                    last_level += 1;
                    known[last_level].clear();
                }
                while last_level > level {
                    known[last_level].clear();
                    last_level -= 1;
                }
                level
            } else {
                0
            };
            if known[level].contains(&text) {
                let shown = ellipsify(&text, false, false);
                diagnostics.push(Self::META.diagnostic(
                    doc,
                    heading_line(doc, id),
                    message(&Self::META, None, Some(&shown)),
                ));
            } else {
                known[level].push(text);
            }
        }
        diagnostics
    }
}
