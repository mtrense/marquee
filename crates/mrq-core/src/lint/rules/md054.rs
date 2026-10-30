//! MD054 `link-image-style`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{LinkForm, NodeKind, ReferenceStyle};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::{Definitions, edit, ellipsify, first_line_span, link_syntax, message};

/// Link and image style.
///
/// Links and images written in a style that is switched off are reported: `autolink`
/// (`<https://…>`), `inline`, `full`, `collapsed` and `shortcut` references, and with
/// `url_inline` off, inline links whose text is their (absolute URL) destination, which
/// could be autolinks. References are only checked when their label, looked up as
/// written, has a definition. A single-line link is fixed to an inline link when
/// those are allowed (and URL texts are, or no autolink fits), else to an autolink when
/// the destination fits one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md054;

/// Options of MD054 `link-image-style`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct Md054Options {
    /// Allow autolinks.
    pub autolink: bool,
    /// Allow inline links and images.
    pub inline: bool,
    /// Allow full reference links and images.
    pub full: bool,
    /// Allow collapsed reference links and images.
    pub collapsed: bool,
    /// Allow shortcut reference links and images.
    pub shortcut: bool,
    /// Allow URLs as inline links.
    pub url_inline: bool,
}

impl Default for Md054Options {
    fn default() -> Self {
        Self {
            autolink: true,
            inline: true,
            full: true,
            collapsed: true,
            shortcut: true,
            url_inline: true,
        }
    }
}

impl Rule for Md054 {
    type Options = Md054Options;

    const META: RuleMeta = RuleMeta {
        id: "MD054",
        aliases: &["link-image-style"],
        tags: &["images", "links"],
        description: "Link and image style",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md054Options) -> Vec<Diagnostic> {
        let Md054Options {
            autolink,
            inline,
            full,
            collapsed,
            shortcut,
            url_inline,
        } = *options;
        if autolink && inline && full && collapsed && shortcut && url_inline {
            return Vec::new();
        }
        let ast = doc.ast();
        let text = doc.text();
        let definitions = Definitions::of(doc);
        let source_of = |range: ByteRange| &text[range.start..range.end];
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let (NodeKind::Link(link) | NodeKind::Image(link)) = ast.kind(id) else {
                continue;
            };
            let image = matches!(ast.kind(id), NodeKind::Image(_));
            let span = ast.span(id);
            let (label, destination, error) = if link.form == LinkForm::Autolink {
                let inner = &text[span.start + 1..span.end - 1];
                (inner, inner.to_owned(), !autolink && !inner.is_empty())
            } else if link.form == LinkForm::Bare {
                continue;
            } else {
                let Some(syntax) = link_syntax(doc, id) else {
                    continue;
                };
                let label = source_of(syntax.text);
                if let Some(destination) = syntax.destination {
                    let destination = source_of(destination);
                    let error = !inline
                        || (!url_inline
                            && autolink
                            && !image
                            && !syntax.title
                            && label == destination
                            && autolinkable(destination));
                    (label, destination.to_owned(), error)
                } else {
                    let style = match &link.form {
                        LinkForm::Reference { style, .. } => *style,
                        _ => ReferenceStyle::Shortcut,
                    };
                    let reference = syntax
                        .reference
                        .map(source_of)
                        .filter(|reference| !reference.is_empty());
                    let destination = definitions
                        .get(reference.unwrap_or(label))
                        .unwrap_or_default()
                        .to_owned();
                    let allowed = match style {
                        ReferenceStyle::Full => full,
                        ReferenceStyle::Collapsed => collapsed,
                        ReferenceStyle::Shortcut => shortcut,
                    };
                    let error = !destination.is_empty() && !allowed;
                    (label, destination, error)
                }
            };
            if !error {
                continue;
            }
            let link_text = &text[span.start..span.end];
            let first_line = link_text.split(['\r', '\n']).next().unwrap_or_default();
            let shown = ellipsify(first_line, false, false);
            let range = first_line_span(doc, span);
            let single_line = range.end == span.end;
            let diagnostic =
                Self::META.diagnostic(doc, range, message(&Self::META, None, Some(&shown)));
            let can_inline = inline && !label.is_empty();
            let can_autolink = autolink && !image && autolinkable(&destination);
            let replacement = if !single_line {
                None
            } else if can_inline && (url_inline || !can_autolink) {
                let prefix = if image { "!" } else { "" };
                let label = escape(label, &['[', ']']);
                let destination = escape(&destination, &['(', ')']);
                Some(format!("{prefix}[{label}]({destination})"))
            } else if can_autolink {
                Some(format!("<{}>", remove_backslash_escapes(&destination)))
            } else {
                None
            };
            diagnostics.push(match replacement {
                Some(replacement) => diagnostic.with_fix(Fix::new(
                    "Change the link style",
                    vec![edit(range, replacement)],
                )),
                None => diagnostic,
            });
        }
        diagnostics
    }
}

/// `text` with a backslash before each of `chars`.
fn escape(text: &str, chars: &[char]) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if chars.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `text` without the backslashes of backslash escapes.
fn remove_backslash_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// markdownlint's `autolinkAble`: an absolute URL (as the WHATWG URL parser accepts it,
/// approximated: a scheme, and a host for the special schemes but `file`) without
/// spaces or angle brackets.
fn autolinkable(destination: &str) -> bool {
    let Some((scheme, rest)) = destination.split_once(':') else {
        return false;
    };
    let scheme_ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let special = ["http", "https", "ws", "wss", "ftp"]
        .iter()
        .any(|special| special.eq_ignore_ascii_case(scheme));
    let host_ok = !special || {
        let host = rest.trim_start_matches(['/', '\\']);
        let host = host.split(['/', '\\', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit('@').next().unwrap_or_default();
        !host.is_empty() && !host.starts_with(':')
    };
    scheme_ok && host_ok && !destination.contains([' ', '<', '>'])
}
