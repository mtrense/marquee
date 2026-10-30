//! External link checks: the [`ExternalChecker`] seam, [`ExternalStatus`],
//! [`ExternalExclude`] and [`ExternalOptions`].

use std::collections::{HashMap, HashSet};
use std::fmt;

use regex::{Regex, RegexSet};

use super::{DocumentLink, Target, codes, scheme};
use crate::Document;
use crate::config::Config;
use crate::diag::{Diagnostic, Severity};

/// What checking one external URL found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExternalStatus {
    /// The URL works (after redirects, if any).
    Ok,
    /// The URL is broken; the reason is shown in the diagnostic (`404 Not Found`, a
    /// timeout, a connection error, …).
    Broken(String),
    /// The URL was not checked (unsupported, or excluded by the checker itself); not
    /// reported.
    Skipped,
}

/// Checks external URLs: the seam between the [`Checker`](super::Checker) and the
/// network.
///
/// With the `external-links` feature, `HttpChecker` checks over HTTP; tests and the LSP
/// can substitute their own. The checker only ever gets http(s) URLs that no
/// [`ExternalExclude`] pattern matches, each once, as written in the document (with any
/// `#fragment`).
pub trait ExternalChecker: fmt::Debug {
    /// The status of each of `urls`, in the same order. A call may block for as long as
    /// the checks take.
    fn check(&self, urls: &[&str]) -> Vec<ExternalStatus>;
}

/// External URLs never to check: the `validate.external_exclude` regular expressions,
/// any of which may match anywhere in the URL. The default excludes nothing.
#[derive(Debug, Clone)]
pub struct ExternalExclude(RegexSet);

impl Default for ExternalExclude {
    fn default() -> Self {
        Self(RegexSet::empty())
    }
}

impl ExternalExclude {
    /// Compiles `patterns` (Rust [`regex`] syntax).
    pub fn new<I, S>(patterns: I) -> Result<Self, ExcludeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let patterns: Vec<S> = patterns.into_iter().collect();
        RegexSet::new(&patterns).map(Self).map_err(|source| {
            // Name the pattern that fails on its own; a set can also fail only together
            // (size limits), then the error names none in particular.
            let pattern = patterns
                .iter()
                .map(AsRef::as_ref)
                .find(|pattern| Regex::new(pattern).is_err())
                .unwrap_or_default();
            ExcludeError {
                pattern: pattern.to_owned(),
                source,
            }
        })
    }

    /// Whether `url` matches any of the patterns.
    pub fn is_excluded(&self, url: &str) -> bool {
        self.0.is_match(url)
    }
}

/// An `external_exclude` pattern that is not a valid regular expression.
#[derive(Debug, thiserror::Error)]
#[error("invalid `validate.external_exclude` pattern `{pattern}`")]
pub struct ExcludeError {
    /// The pattern as written.
    pub pattern: String,
    /// What is wrong with it.
    #[source]
    pub source: regex::Error,
}

/// Whether and which external links to check: `validate.external` (or `--external`) and
/// `validate.external_exclude`.
///
/// Nothing is checked unless `enabled`; then a caller builds an [`ExternalChecker`] (such
/// as `HttpChecker`) and passes it to
/// [`Checker::with_external_links`](super::Checker::with_external_links) with `exclude`.
#[derive(Debug, Clone, Default)]
pub struct ExternalOptions {
    /// Check external links at all (off by default).
    pub enabled: bool,
    /// URLs never to check.
    pub exclude: ExternalExclude,
}

impl ExternalOptions {
    /// The options of the `validate` section of `config`.
    pub fn from_config(config: &Config) -> Result<Self, ExcludeError> {
        Ok(Self {
            enabled: config.validate.external,
            exclude: ExternalExclude::new(&config.validate.external_exclude)?,
        })
    }

    /// Also enabled if `external` (the `--external` flag) is set; the flag never turns
    /// checking off.
    #[must_use]
    pub fn with_flag(mut self, external: bool) -> Self {
        self.enabled |= external;
        self
    }
}

/// The external checks of a [`Checker`](super::Checker): the checker, the excludes and
/// the statuses found so far.
#[derive(Debug)]
pub(super) struct External<'w> {
    checker: &'w dyn ExternalChecker,
    exclude: &'w ExternalExclude,
    statuses: HashMap<String, ExternalStatus>,
}

impl<'w> External<'w> {
    pub(super) fn new(checker: &'w dyn ExternalChecker, exclude: &'w ExternalExclude) -> Self {
        Self {
            checker,
            exclude,
            statuses: HashMap::new(),
        }
    }

    /// The URL of `link` if it is to be checked: http(s) and not excluded.
    fn url<'l>(&self, link: &'l DocumentLink) -> Option<&'l str> {
        let Target::External(url) = &link.target else {
            return None;
        };
        let http = scheme(url).is_some_and(|scheme| {
            scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
        });
        (http && !self.exclude.is_excluded(url)).then_some(url)
    }

    /// Checks, in one batch, the URLs of `links` that are not checked yet.
    pub(super) fn prefetch<'l>(&mut self, links: impl IntoIterator<Item = &'l DocumentLink>) {
        let mut pending: Vec<&str> = Vec::new();
        let mut seen = HashSet::new();
        for link in links {
            if let Some(url) = self.url(link)
                && !self.statuses.contains_key(url)
                && seen.insert(url)
            {
                pending.push(url);
            }
        }
        if pending.is_empty() {
            return;
        }
        let statuses = self.checker.check(&pending);
        for (url, status) in pending.into_iter().zip(statuses) {
            self.statuses.insert(url.to_owned(), status);
        }
    }

    /// A [`codes::BROKEN_EXTERNAL_LINK`] for each link of `links` (from `document`) whose
    /// URL is broken.
    pub(super) fn check(
        &mut self,
        document: &Document,
        links: &[DocumentLink],
        out: &mut Vec<Diagnostic>,
    ) {
        self.prefetch(links);
        for link in links {
            if let Some(url) = self.url(link)
                && let Some(ExternalStatus::Broken(reason)) = self.statuses.get(url)
            {
                out.push(Diagnostic::new(
                    codes::BROKEN_EXTERNAL_LINK,
                    Severity::Error,
                    document.name(),
                    link.span,
                    format!("external link `{url}` is broken: {reason}"),
                ));
            }
        }
    }
}
