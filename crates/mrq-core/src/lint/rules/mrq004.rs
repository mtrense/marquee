//! MRQ004 `stale-include`.

use crate::diag::Diagnostic;
use crate::directive::DirectiveKind;
use crate::frontmatter::Frontmatter;
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::validate::is_remote;
use crate::{Document, Workspace};

/// `mrq:include` region out of date.
///
/// For now this reports only regions that are certainly stale without expanding them:
/// an empty region (an `mrq:end` with nothing but whitespace before it) whose file has
/// a body after its frontmatter, when no attribute could make the expansion empty. Only
/// `path`, `shift`, `dedent`, `frontmatter`, `lang` and `sha256` are allowed; `section`,
/// `select`, `lines`, symbol attributes or unknown ones skip the region. The path
/// resolves like `mrq validate`'s (relative to the document, a leading `/` from the
/// current directory); missing files (`mrq validate` reports them) and remote ones are
/// skipped. A bare start marker without `mrq:end` is not stale: `mrq interpolate`
/// expands it to stdout, and only `--in-place` adds the end marker. No fix yet.
// TODO(M7.4, M7.8): compare every region's content with the include resolver's
// expansion, as `mrq validate` will for `stale-include`, and fix by replacing the content.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mrq004;

/// Attributes that never make an include's expansion empty.
const NON_FILTERING: &[&str] = &["path", "shift", "dedent", "frontmatter", "lang", "sha256"];

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
        let ast = doc.ast();
        let workspace = Workspace::new(".");
        let mut diagnostics = Vec::new();
        for region in doc.directives() {
            if region.kind != DirectiveKind::Include
                || region.is_bare()
                || !doc.text()[region.content.start..region.content.end]
                    .trim()
                    .is_empty()
            {
                continue;
            }
            let directive = region.directive(ast);
            let Some(written) = directive.get("path") else {
                continue;
            };
            if is_remote(written)
                || directive
                    .attributes
                    .iter()
                    .any(|attr| !NON_FILTERING.contains(&attr.name.as_str()))
            {
                continue;
            }
            let path = workspace.resolve(doc.path(), written);
            let Ok(included) = std::fs::read_to_string(&path) else {
                continue;
            };
            let body_start = Frontmatter::extract(&included).map_or(0, |fm| fm.body_start());
            if included[body_start..].trim().is_empty() {
                continue;
            }
            diagnostics.push(
                Self::META
                    .diagnostic(
                        doc,
                        ast.span(region.start),
                        format!("`mrq:include` region is empty, but `{written}` is not"),
                    )
                    .with_help("run `mrq interpolate --in-place` to regenerate it"),
            );
        }
        diagnostics
    }
}
