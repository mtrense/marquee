//! MRQ005 `no-url-wikilinks`.

use crate::Document;
use crate::ast::NodeKind;
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::validate::scheme;

use super::edit;

/// Bare URL in a wikilink.
///
/// Obsidian takes whatever is between `[[` and `]]` for a note name, so
/// `[[https://example.com]]` is a link to a note of that name (created empty when
/// clicked), not to the page; the same goes for embeds. Reported are wikilinks and embeds
/// whose target starts with `scheme://` or `www.`; only flavors with wikilinks (obsidian)
/// have any. The fix turns them into Markdown: `<https://…>` without an alias,
/// `[alias](https://…)` with one, `![alias](https://…)` for an embed (the alias, which
/// may be a size such as `100x200`, becomes the alt text, which Obsidian reads the same
/// way). `www.` gets `https://`, and a destination with spaces or parentheses is put in
/// `<…>`. Targets holding `<`, `>` or line breaks, or aliases holding brackets, are not
/// fixed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mrq005;

impl Rule for Mrq005 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MRQ005",
        aliases: &["no-url-wikilinks"],
        tags: &["links", "url", "mrq"],
        description: "Bare URL in a wikilink",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let ast = doc.ast();
        let mut diagnostics = Vec::new();
        for id in ast.descendants(ast.root()) {
            let embed = match ast.kind(id) {
                NodeKind::WikiLink(_) => false,
                NodeKind::Embed(_) => true,
                _ => continue,
            };
            let span = ast.span(id);
            let source = &doc.text()[span.start..span.end];
            let Some(inner) = source
                .trim_start_matches('!')
                .strip_prefix("[[")
                .and_then(|rest| rest.strip_suffix("]]"))
            else {
                continue;
            };
            let (target, alias) = match inner.split_once('|') {
                Some((target, alias)) => (target.trim(), Some(alias)),
                None => (inner.trim(), None),
            };
            let Some(url) = url(target) else {
                continue;
            };
            let what = if embed { "an embed" } else { "a wikilink" };
            let mut diagnostic = Self::META.diagnostic(
                doc,
                span,
                format!("`{target}` in {what} is a note name, not a URL"),
            );
            if let Some(replacement) = markdown(target, &url, alias, embed) {
                let fix = Fix::new("Write a Markdown link", vec![edit(span, replacement)]);
                diagnostic = diagnostic.with_fix(fix);
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }
}

/// The URL `target` stands for, if it looks like one: `scheme://…` as written, or
/// `www.…` with `https://`.
fn url(target: &str) -> Option<String> {
    if let Some(scheme) = scheme(target)
        && target[scheme.len()..].starts_with("://")
    {
        return Some(target.to_owned());
    }
    target
        .get(..4)
        .is_some_and(|start| start.eq_ignore_ascii_case("www."))
        .then(|| format!("https://{target}"))
}

/// The Markdown link or image for a wikilink to `url` (written `target`), if one can
/// say the same.
fn markdown(target: &str, url: &str, alias: Option<&str>, embed: bool) -> Option<String> {
    if url.contains(['<', '>', '\n', '\r']) || alias.is_some_and(|a| a.contains(['[', ']'])) {
        return None;
    }
    let bracketed = url.contains(|c: char| c.is_whitespace() || c == '(' || c == ')');
    let destination = if bracketed {
        format!("<{url}>")
    } else {
        url.to_owned()
    };
    Some(match (embed, alias) {
        (true, alias) => format!("![{}]({destination})", alias.unwrap_or_default()),
        (false, Some(alias)) => format!("[{alias}]({destination})"),
        (false, None) if !bracketed && url == target => format!("<{url}>"),
        (false, None) => format!("[{target}]({destination})"),
    })
}
