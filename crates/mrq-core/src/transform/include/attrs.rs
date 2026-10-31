//! The attributes of an `mrq:include`: [`Spec`].

use super::INCLUDE_CODE;
use super::code::{CodeSpec, SYMBOL_KINDS};
use crate::ast::{Directive, DirectiveAttribute, Span};
use crate::diag::{Diagnostic, Severity};
use crate::document::Document;
use crate::query::{Query, QueryError};
use crate::validate::codes;

/// Attributes of later kinds of include (remote includes), rejected until they are
/// supported.
const RESERVED: &[&str] = &["sha256"];

/// Attributes of Markdown includes, ignored (with a warning) by code includes.
const MARKDOWN_ONLY: &[&str] = &["shift", "section", "select", "frontmatter"];

/// What happens to the included file's frontmatter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FrontmatterMode {
    /// Left out.
    #[default]
    Strip,
    /// Included as written, before the content.
    Keep,
    /// Left out, its keys merged into the including file's frontmatter.
    Merge,
}

/// An include's attributes, checked.
#[derive(Debug)]
pub(super) struct Spec<'d> {
    pub path: &'d DirectiveAttribute,
    /// Levels to shift headings by, and the attribute.
    pub shift: Option<(i32, &'d DirectiveAttribute)>,
    pub section: Option<&'d DirectiveAttribute>,
    pub select: Option<(Query, &'d DirectiveAttribute)>,
    pub frontmatter: FrontmatterMode,
    /// Whether to strip common indentation, if given.
    pub dedent: Option<bool>,
    /// With `lang` or a symbol, how the file is included as code instead of Markdown.
    pub code: Option<CodeSpec<'d>>,
}

impl Spec<'_> {
    /// Whether to strip common indentation: as given, else for code includes only.
    pub fn dedent(&self) -> bool {
        self.dedent.unwrap_or(self.code.is_some())
    }
}

/// The [`Spec`] of `directive`, a directive of `host` spanning `marker`; `None` after
/// reporting an error in `out`. Unknown attributes get a warning and are ignored.
pub(super) fn parse<'d>(
    host: &Document,
    directive: &'d Directive,
    marker: Span,
    out: &mut Vec<Diagnostic>,
) -> Option<Spec<'d>> {
    let report = |severity, attr: &DirectiveAttribute, message: String| {
        Diagnostic::new(INCLUDE_CODE, severity, host.name(), attr.span, message)
    };
    let Some(path) = directive.attributes.iter().find(|attr| attr.name == "path") else {
        out.push(no_path(host, marker));
        return None;
    };
    let errors = out.len();
    let mut spec = Spec {
        path,
        shift: None,
        section: None,
        select: None,
        frontmatter: FrontmatterMode::default(),
        dedent: None,
        code: None,
    };
    let is_code = directive.get("lang").is_some()
        || SYMBOL_KINDS
            .iter()
            .any(|(name, _)| directive.get(name).is_some());
    let mut code = Vec::new();
    // Duplicates are reported by the parser; the first one counts.
    let mut seen = Vec::new();
    for attr in &directive.attributes {
        if seen.contains(&attr.name.as_str()) {
            continue;
        }
        seen.push(&attr.name);
        let value = attr.value.as_str();
        match attr.name.as_str() {
            "path" => {}
            name if is_code && MARKDOWN_ONLY.contains(&name) => out.push(report(
                Severity::Warning,
                attr,
                format!("`{name}` does not apply to code includes and is ignored"),
            )),
            "shift" => match value.parse::<i32>() {
                Ok(by) => spec.shift = Some((by, attr)),
                Err(_) => out.push(report(
                    Severity::Error,
                    attr,
                    format!("`shift` must be a whole number of levels, not `{value}`"),
                )),
            },
            "section" => spec.section = Some(attr),
            "select" => match Query::selector(value) {
                Ok(query) => spec.select = Some((query, attr)),
                Err(QueryError::Syntax(err)) => {
                    out.push(err.to_diagnostic(host.name(), value_start(host, attr)));
                }
                Err(err) => out.push(report(Severity::Error, attr, err.to_string())),
            },
            "frontmatter" => match value {
                "strip" => spec.frontmatter = FrontmatterMode::Strip,
                "keep" => spec.frontmatter = FrontmatterMode::Keep,
                "merge" => spec.frontmatter = FrontmatterMode::Merge,
                _ => out.push(report(
                    Severity::Error,
                    attr,
                    format!("`frontmatter` must be `strip`, `keep` or `merge`, not `{value}`"),
                )),
            },
            "dedent" => match value {
                "true" => spec.dedent = Some(true),
                "false" => spec.dedent = Some(false),
                _ => out.push(report(
                    Severity::Error,
                    attr,
                    format!("`dedent` must be `true` or `false`, not `{value}`"),
                )),
            },
            "lang" | "lines" | "with_docs" => code.push(attr),
            name if SYMBOL_KINDS.iter().any(|(kind, _)| *kind == name) => code.push(attr),
            name if RESERVED.contains(&name) => out.push(report(
                Severity::Error,
                attr,
                format!("`{name}` is not supported yet; the include is skipped"),
            )),
            name => out.push(report(
                Severity::Warning,
                attr,
                format!("unknown `mrq:include` attribute `{name}` is ignored"),
            )),
        }
    }
    spec.code = CodeSpec::parse(host, &code, out);
    let failed = out[errors..]
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    (!failed).then_some(spec)
}

/// The error for an include without a `path`, at its start marker.
fn no_path(host: &Document, marker: Span) -> Diagnostic {
    Diagnostic::new(
        codes::MISSING_INCLUDE,
        Severity::Error,
        host.name(),
        marker,
        "`mrq:include` has no `path`",
    )
    .with_help("write e.g. `<!-- mrq:include path=\"chapter.md\" -->`")
}

/// Where the value of `attr` starts in `host`'s source: after `name=` and any quote.
fn value_start(host: &Document, attr: &DirectiveAttribute) -> usize {
    let at = attr.span.start + attr.name.len() + 1;
    let quoted = matches!(host.text().as_bytes().get(at), Some(b'"' | b'\''));
    at + usize::from(quoted)
}
