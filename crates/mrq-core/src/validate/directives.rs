//! Directive checks: missing include targets, include cycles, and stale `mrq:include` and
//! `mrq:toc` regions.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Checker, codes, scheme};
use crate::Slashed as _;
use crate::ast::Span;

use crate::diag::{Diagnostic, Severity};
use crate::directive::DirectiveKind;
use crate::document::normalize;
use crate::toc;
use crate::transform::{FileLoader, INCLUDE_CODE, Includer, RegionRefresh};
use crate::{Document, Workspace};

/// An `mrq:include` of a document, its `path` resolved.
#[derive(Debug)]
enum Include {
    /// No `path` attribute; the span is the start marker's.
    NoPath(Span),
    /// A local `path` (written as `written`, spanning `span` with its `path=`), resolved.
    Local {
        span: Span,
        written: String,
        path: PathBuf,
    },
}

impl Checker<'_> {
    /// Includes without a `path`, whose file does not exist, or that lead back to
    /// `document`. The others are refreshed as `mrq interpolate --in-place` would: their
    /// problems in `document` (bad attributes, a `section` or `select` that finds nothing;
    /// code [`INCLUDE_CODE`]) are reported, and so is a region whose body would change
    /// (`stale-include`), unless it is bare or including it failed somewhere. Remote
    /// (`http(s)://`) paths are skipped.
    pub(super) fn check_includes(&mut self, document: &Document, out: &mut Vec<Diagnostic>) {
        let origin = document.path().map(identity);
        // Refreshed on first use, when an include passed the checks above.
        let mut refreshed: Option<Vec<RegionRefresh>> = None;
        for include in includes(self.workspace, document) {
            let report = |code, span, message: String| {
                Diagnostic::new(code, Severity::Error, document.name(), span, message)
            };
            match include {
                Include::NoPath(span) => out.push(
                    report(
                        codes::MISSING_INCLUDE,
                        span,
                        "`mrq:include` has no `path`".into(),
                    )
                    .with_help("write e.g. `<!-- mrq:include path=\"chapter.md\" -->`"),
                ),
                Include::Local {
                    span,
                    written,
                    path,
                } => {
                    if self.workspace.get(&path).is_none() && !path.is_file() {
                        out.push(
                            report(
                                codes::MISSING_INCLUDE,
                                span,
                                format!("included file `{written}` does not exist"),
                            )
                            .with_help(format!("resolved to `{}`", path.slashed())),
                        );
                    } else if let Some(origin) = &origin
                        && let Some(chain) = self.chain_back(&path, origin)
                    {
                        let message =
                            format!("`mrq:include` cycle: `{written}` includes this file again");
                        out.push(
                            report(codes::INCLUDE_CYCLE, span, message)
                                .with_help(describe_cycle(document.name(), &chain)),
                        );
                    } else if let Some(region) = refreshed
                        .get_or_insert_with(|| self.refresh_includes(document))
                        .iter()
                        .find(|region| {
                            region.marker.start <= span.start && span.end <= region.marker.end
                        })
                    {
                        out.extend(
                            region
                                .diagnostics
                                .iter()
                                .filter(|d| d.code == INCLUDE_CODE && d.file == document.name())
                                .cloned(),
                        );
                        if region.edit.is_some() && !region.bare && !region.has_errors() {
                            out.push(
                                Diagnostic::new(
                                    codes::STALE_INCLUDE,
                                    Severity::Warning,
                                    document.name(),
                                    region.marker,
                                    "`mrq:include` region is out of date",
                                )
                                .with_help("run `mrq interpolate --in-place` to regenerate it"),
                            );
                        }
                    }
                }
            }
        }
    }

    /// What `mrq interpolate --in-place` would do to the include regions of `document`,
    /// offline: files are read from disk (not the workspace), and remote includes, also
    /// in included files, fail.
    fn refresh_includes(&self, document: &Document) -> Vec<RegionRefresh> {
        Includer::new()
            .loader(FileLoader)
            .root(self.workspace.root())
            .document_options(self.workspace.options().clone())
            .allow_remote(false)
            .refresh(document)
    }

    /// The files from `start` (included) through which includes lead back to `origin`
    /// (an [`identity`]): empty if `start` is `origin`, `None` if they never do.
    fn chain_back(&mut self, start: &Path, origin: &Path) -> Option<Vec<PathBuf>> {
        if identity(start) == origin {
            return Some(Vec::new());
        }
        let mut chain = Vec::new();
        self.search(start, origin, &mut HashSet::new(), &mut chain)
            .then_some(chain)
    }

    /// Depth-first search from `at` for an include of `origin`, extending `chain` with
    /// the files on the way to it; `seen` holds the identities already searched.
    fn search(
        &mut self,
        at: &Path,
        origin: &Path,
        seen: &mut HashSet<PathBuf>,
        chain: &mut Vec<PathBuf>,
    ) -> bool {
        if !seen.insert(identity(at)) {
            return false;
        }
        let workspace = self.workspace;
        let next: Vec<PathBuf> = self
            .markdown(at)
            .map(|document| {
                includes(workspace, document)
                    .into_iter()
                    .filter_map(|include| match include {
                        Include::Local { path, .. } => Some(path),
                        Include::NoPath(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        chain.push(at.to_owned());
        for path in next {
            if identity(&path) == origin || self.search(&path, origin, seen, chain) {
                return true;
            }
        }
        chain.pop();
        false
    }
}

/// The `mrq:include` regions of `document` in order, with remote paths left out.
fn includes(workspace: &Workspace, document: &Document) -> Vec<Include> {
    let ast = document.ast();
    document
        .directives()
        .iter()
        .filter(|region| region.kind == DirectiveKind::Include)
        .filter_map(|region| {
            let directive = region.directive(ast);
            let Some(attr) = directive.attributes.iter().find(|a| a.name == "path") else {
                return Some(Include::NoPath(ast.span(region.start)));
            };
            if is_remote(&attr.value) {
                // Validation stays offline; the include resolver reports failed fetches.
                return None;
            }
            Some(Include::Local {
                span: attr.span,
                written: attr.value.clone(),
                path: workspace.resolve(document.path(), &attr.value),
            })
        })
        .collect()
}

/// Whether an include `path` is an `http(s)://` URL.
pub(crate) fn is_remote(path: &str) -> bool {
    scheme(path).is_some_and(|s| s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
}

/// What identifies a file for cycle detection: its canonical path, else (a file that is
/// only in the workspace) its normalized path.
fn identity(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| normalize(path))
}

/// `a includes b, which includes c, which includes a` for the file `origin` and the
/// `chain` of files between; `a includes itself` if there are none.
fn describe_cycle(origin: &Path, chain: &[PathBuf]) -> String {
    let origin = origin.slashed();
    if chain.is_empty() {
        return format!("{origin} includes itself");
    }
    let mut text = format!("{origin} includes ");
    for path in chain {
        let _ = write!(text, "{}, which includes ", path.slashed());
    }
    let _ = write!(text, "{origin}");
    text
}

/// `mrq:toc` regions that `mrq toc --insert` would change (reported at their start
/// marker), and regions it cannot refresh (an invalid `depth`).
pub(super) fn check_toc(document: &Document, out: &mut Vec<Diagnostic>) {
    let refresh = toc::refresh_edits(document, None);
    out.extend(refresh.diagnostics);
    let ast = document.ast();
    let markers: Vec<_> = document
        .directives()
        .iter()
        .filter(|region| region.kind == DirectiveKind::Toc)
        .map(|region| (ast.span(region.start), region.is_bare()))
        .collect();
    for edit in refresh.edits {
        // An edit starts after its own region's start marker, before the next one's.
        let Some(&(marker, bare)) = markers
            .iter()
            .rev()
            .find(|(marker, _)| marker.end <= edit.range.start)
        else {
            continue;
        };
        let message = if bare {
            "table of contents has no `mrq:end` and is not generated yet"
        } else {
            "table of contents is out of date"
        };
        out.push(
            Diagnostic::new(
                codes::STALE_TOC,
                Severity::Warning,
                document.name(),
                marker,
                message,
            )
            .with_help("run `mrq toc --insert` to regenerate it"),
        );
    }
}
