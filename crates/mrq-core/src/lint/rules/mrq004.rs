//! MRQ004 `stale-include`.

use crate::Document;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::transform::{FileLoader, Includer};

/// `mrq:include` region out of date.
///
/// Reports a region whose body differs from what `mrq interpolate --in-place` writes
/// ([`Includer::refresh`]), at its start marker; the fix writes that body. Paths resolve
/// like `mrq validate`'s (relative to the document, a leading `/` from the current
/// directory), and included files are parsed with the default options. Regions whose
/// include fails somewhere (a missing file, a bad attribute, a nested include that
/// fails; `mrq validate` and `mrq interpolate` report why) are skipped, and so are remote
/// includes, also in included files: linting never touches the network. A bare start
/// marker without `mrq:end` is not stale: `mrq interpolate` expands it to stdout, and
/// only `--in-place` adds the end marker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mrq004;

impl Rule for Mrq004 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MRQ004",
        aliases: &["stale-include"],
        tags: &["directives", "mrq"],
        description: "`mrq:include` region out of date",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let includer = Includer::new().loader(FileLoader).allow_remote(false);
        includer
            .refresh(doc)
            .into_iter()
            .filter(|region| !region.bare && !region.has_errors())
            .filter_map(|region| {
                let edit = region.edit?;
                Some(
                    Self::META
                        .diagnostic(doc, region.marker, "`mrq:include` region is out of date")
                        .with_help("run `mrq interpolate --in-place` to regenerate it")
                        .with_fix(Fix::new("Regenerate the included content", vec![edit])),
                )
            })
            .collect()
    }
}
