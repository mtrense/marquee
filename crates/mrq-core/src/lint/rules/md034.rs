//! MD034 `no-bare-urls`.

use std::sync::LazyLock;

use regex::Regex;

use crate::Document;
use crate::ast::{Ast, LinkForm, NodeId, NodeKind};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{NoOptions, Rule, RuleMeta};
use crate::source::ByteRange;

use super::{edit, ellipsify, message};

/// Bare URL used.
///
/// A URL or email address that GFM turns into a link by itself is reported, unless it
/// sits between the tags of an inline HTML element (`<a href="…">https://…</a>`) or
/// right between `<` and `>`. The fix wraps it in `<…>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md034;

/// markdownlint's `getHtmlTagInfo`: closing flag and name of a tag.
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^<(/?)([^!>/\s][^/\s>]*)").expect("valid regex"));

impl Rule for Md034 {
    type Options = NoOptions;

    const META: RuleMeta = RuleMeta {
        id: "MD034",
        aliases: &["no-bare-urls"],
        tags: &["links", "url"],
        description: "Bare URL used",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, _: &NoOptions) -> Vec<Diagnostic> {
        let mut urls = Vec::new();
        bare_urls(doc.ast(), doc.ast().root(), &mut urls);
        let text = doc.text();
        urls.into_iter()
            .map(|range| {
                let url = &text[range.start..range.end];
                let fix = Fix::new("Wrap the URL in <>", vec![edit(range, format!("<{url}>"))]);
                let shown = ellipsify(url, false, false);
                Self::META
                    .diagnostic(doc, range, message(&Self::META, None, Some(&shown)))
                    .with_fix(fix)
            })
            .collect()
    }
}

/// Collects the bare URLs below `parent`, skipping the children between an inline
/// HTML opening tag and its closing tag.
fn bare_urls(ast: &Ast, parent: NodeId, urls: &mut Vec<ByteRange>) {
    let children = ast.children(parent);
    let mut index = 0;
    while index < children.len() {
        let child = children[index];
        index += 1;
        if let Some((false, name)) = tag(ast, child) {
            let mut depth = 1;
            for (offset, &candidate) in children[index..].iter().enumerate() {
                match tag(ast, candidate) {
                    Some((true, other)) if other == name => depth -= 1,
                    Some((false, other)) if other == name => depth += 1,
                    _ => {}
                }
                if depth == 0 {
                    index += offset + 1;
                    break;
                }
            }
            continue;
        }
        if matches!(ast.kind(child), NodeKind::Link(link) if link.form == LinkForm::Bare) {
            let text_around = |sibling: Option<NodeId>| match sibling.map(|id| ast.kind(id)) {
                Some(NodeKind::Text(text)) => Some(text.as_str()),
                _ => None,
            };
            let before = text_around(ast.prev_sibling(child));
            let after = text_around(ast.next_sibling(child));
            // micromark/micromark#164: `<` and `>` right around a URL are not a link.
            if !(before.is_some_and(|text| text.ends_with('<'))
                && after.is_some_and(|text| text.starts_with('>')))
            {
                let span = ast.span(child);
                urls.push(ByteRange::new(span.start, span.end));
            }
            continue;
        }
        bare_urls(ast, child, urls);
    }
}

/// `(closing, name)` of an inline HTML tag.
fn tag(ast: &Ast, id: NodeId) -> Option<(bool, &str)> {
    let NodeKind::HtmlInline(html) = ast.kind(id) else {
        return None;
    };
    let captures = TAG.captures(html)?;
    Some((
        captures.get(1).is_some_and(|slash| !slash.is_empty()),
        captures.get(2)?.as_str(),
    ))
}
