//! MD051 `link-fragments`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::Document;
use crate::ast::{Ast, LinkForm, NodeId, NodeKind, Span};
use crate::diag::{Diagnostic, Fix};
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;
use crate::validate::reference_definitions;

use super::{
    CHARACTER_REFERENCE, edit, expected_actual, first_line_span, html_attribute,
    html_attribute_pattern, html_pieces, html_tag_name, link_syntax, message,
};

/// Link fragments should be valid.
///
/// A link (or link reference definition) to `#fragment` in the same document must
/// name a fragment markdownlint knows: `#top`, a heading's GitHub-style slug (lowercase,
/// punctuation but `-` and `_` dropped, spaces as `-`, percent-encoded; repeats get
/// `-1`, `-2`, …), a `{#id}` in heading text, an HTML `id` (or `<a name>`), or a GitHub
/// line fragment like `#L12` or `#L12C3-L14C5`. Fragments matching `ignored_pattern`
/// are fine. A fragment that differs only in case is reported with the right one
/// (unless `ignore_case`), and the fix writes it; other fragments have no fix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Md051;

/// Options of MD051 `link-fragments`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Md051Options {
    /// Ignore case of fragments.
    pub ignore_case: bool,
    /// Pattern for ignoring additional fragments; an invalid one ignores nothing.
    pub ignored_pattern: String,
}

/// markdownlint's `anchorRe`: a `{#id}` in heading text.
static CUSTOM_ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{(#[a-z\d]+(?:[-_]+[a-z\d]+)*)\}").expect("valid regex"));

/// `id` attributes.
static ID: LazyLock<Regex> = LazyLock::new(|| html_attribute_pattern("id"));
/// `name` attributes.
static NAME: LazyLock<Regex> = LazyLock::new(|| html_attribute_pattern("name"));

/// GitHub's line fragments.
static LINE_FRAGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#(?:L\d+(?:C\d+)?-L\d+(?:C\d+)?|L\d+)$").expect("valid regex"));

/// The fragments of a document in the order markdownlint adds them, with the number of
/// headings that produced each.
#[derive(Default)]
struct Fragments {
    keys: Vec<String>,
    counts: HashMap<String, usize>,
}

impl Fragments {
    fn get(&self, key: &str) -> Option<usize> {
        self.counts.get(key).copied()
    }

    fn set(&mut self, key: String, count: usize) {
        if !self.counts.contains_key(&key) {
            self.keys.push(key.clone());
        }
        self.counts.insert(key, count);
    }

    /// The fragments of `doc`: `#top`, then headings, then HTML anchors.
    fn of(doc: &Document) -> Self {
        let ast = doc.ast();
        let text = doc.text();
        let mut fragments = Self::default();
        fragments.set("#top".to_owned(), 0);
        for id in ast.descendants(ast.root()) {
            if !matches!(ast.kind(id), NodeKind::Heading(_)) {
                continue;
            }
            let fragment = slug(&heading_inline_text(doc, id));
            if fragment == "#" {
                continue;
            }
            let count = fragments.get(&fragment).unwrap_or(0);
            if count > 0 {
                fragments.set(format!("{fragment}-{count}"), 0);
            }
            fragments.set(fragment, count + 1);
            if let Some(content) = heading_content(ast, id) {
                for anchor in CUSTOM_ANCHOR.captures_iter(&text[content.start..content.end]) {
                    if fragments.get(&anchor[1]).is_none() {
                        fragments.set(anchor[1].to_owned(), 1);
                    }
                }
            }
        }

        for (range, _) in html_pieces(doc) {
            let html = &text[range.start..range.end];
            let Some((name, false)) = html_tag_name(html) else {
                continue;
            };
            let anchor = html_attribute(html, &ID).or_else(|| {
                name.eq_ignore_ascii_case("a")
                    .then(|| html_attribute(html, &NAME))
                    .flatten()
            });
            if let Some(anchor) = anchor {
                fragments.set(format!("#{anchor}"), 0);
            }
        }
        fragments
    }
}

impl Rule for Md051 {
    type Options = Md051Options;

    const META: RuleMeta = RuleMeta {
        id: "MD051",
        aliases: &["link-fragments"],
        tags: &["links"],
        description: "Link fragments should be valid",
        default_enabled: true,
    };

    fn check(&self, doc: &Document, options: &Md051Options) -> Vec<Diagnostic> {
        let text = doc.text();
        let pattern = if options.ignored_pattern.is_empty() {
            "^$"
        } else {
            &options.ignored_pattern
        };
        let ignored = Regex::new(pattern).ok();
        let fragments = Fragments::of(doc);
        let mut diagnostics = Vec::new();
        for (range, destination) in targets(doc) {
            let fragment = unescape(&text[destination.start..destination.end]);
            let Some(rest) = fragment.strip_prefix('#') else {
                continue;
            };
            let encoded = format!("#{}", encode_uri_component(rest));
            if rest.is_empty()
                || fragments.get(&encoded).is_some()
                || LINE_FRAGMENT.is_match(&encoded)
                || ignored
                    .as_ref()
                    .is_some_and(|ignored| ignored.is_match(rest))
            {
                continue;
            }
            let reported = first_line_span(doc, Span::new(range.start, range.end));
            let single_line = reported == range;
            // markdownlint shows the whole link, without shortening it.
            let shown = single_line.then(|| &text[range.start..range.end]);
            let lower = fragment.to_lowercase();
            let mixed_case = fragments
                .keys
                .iter()
                .find(|key| key.to_lowercase() == lower);
            match mixed_case {
                Some(key) if !options.ignore_case && *key != fragment => {
                    let detail = expected_actual(key, &fragment);
                    let diagnostic = Self::META.diagnostic(
                        doc,
                        reported,
                        message(&Self::META, Some(&detail), shown),
                    );
                    diagnostics.push(if single_line {
                        diagnostic.with_fix(Fix::new(
                            format!("Link to {key}"),
                            vec![edit(destination, key.clone())],
                        ))
                    } else {
                        diagnostic
                    });
                }
                Some(_) => {}
                None => diagnostics.push(Self::META.diagnostic(
                    doc,
                    reported,
                    message(&Self::META, None, shown),
                )),
            }
        }
        diagnostics
    }
}

/// Each inline link's (but docfx tabs') and link reference definition's whole range
/// and destination.
fn targets(doc: &Document) -> Vec<(ByteRange, ByteRange)> {
    let ast = doc.ast();
    let text = doc.text();
    let mut targets = Vec::new();
    for id in ast.descendants(ast.root()) {
        if !matches!(ast.kind(id), NodeKind::Link(link) if link.form == LinkForm::Inline)
            || is_docfx_tab(doc, id)
        {
            continue;
        }
        if let Some(destination) = link_syntax(doc, id).and_then(|syntax| syntax.destination) {
            let span = ast.span(id);
            targets.push((ByteRange::new(span.start, span.end), destination));
        }
    }
    for definition in reference_definitions(doc) {
        let span = definition.span;
        let end = span.end - usize::from(text[..span.end].ends_with('>'));
        let destination = ByteRange::new(end - definition.url.len(), end);
        targets.push((ByteRange::new(span.start, span.end), destination));
    }
    targets
}

/// The source of a heading's content: from its first child to its last.
fn heading_content(ast: &Ast, heading: NodeId) -> Option<ByteRange> {
    let children = ast.children(heading);
    let (first, last) = (children.first()?, children.last()?);
    Some(ByteRange::new(ast.span(*first).start, ast.span(*last).end))
}

/// The text markdownlint slugs: text (escapes resolved, character references dropped),
/// code and math, in links too but not in images or HTML.
fn heading_inline_text(doc: &Document, heading: NodeId) -> String {
    let ast = doc.ast();
    let text = doc.text();
    let mut out = String::new();
    let mut stack: Vec<NodeId> = ast.children(heading).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let span = ast.span(id);
        match ast.kind(id) {
            NodeKind::Text(_) => out.push_str(&unescape(&text[span.start..span.end])),
            NodeKind::Code(code) => out.push_str(code),
            NodeKind::Math(math) => out.push_str(&math.literal),
            NodeKind::Image(_) | NodeKind::HtmlInline(_) | NodeKind::Comment(_) => {}
            _ => stack.extend(ast.children(id).iter().rev().copied()),
        }
    }
    out
}

/// GitHub's heading slug as markdownlint computes it, with the leading `#`.
fn slug(text: &str) -> String {
    let kept: String = text
        .to_lowercase()
        .chars()
        .filter(|&c| c == '-' || c == ' ' || is_word_category(c))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect();
    format!("#{}", encode_uri_component(&kept))
}

/// Ruby's `\p{Word}`: letters, marks, numbers and connector punctuation.
fn is_word_category(c: char) -> bool {
    use GeneralCategory as G;
    matches!(
        get_general_category(c),
        G::UppercaseLetter
            | G::LowercaseLetter
            | G::TitlecaseLetter
            | G::ModifierLetter
            | G::OtherLetter
            | G::NonspacingMark
            | G::SpacingMark
            | G::EnclosingMark
            | G::DecimalNumber
            | G::LetterNumber
            | G::OtherNumber
            | G::ConnectorPunctuation
    )
}

/// JavaScript's `encodeURIComponent`.
fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            let mut buffer = [0; 4];
            for byte in c.encode_utf8(&mut buffer).bytes() {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Source text with backslash escapes resolved and character references dropped, as
/// markdownlint joins the `data` and `characterEscapeValue` tokens of a string.
fn unescape(source: &str) -> String {
    let without_references = CHARACTER_REFERENCE.replace_all(source, "");
    let mut out = String::with_capacity(without_references.len());
    let mut chars = without_references.chars().peekable();
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

/// markdownlint's `isDocfxTab`: an ATX heading whose only content is a link to `#tab/…`.
fn is_docfx_tab(doc: &Document, link: NodeId) -> bool {
    let ast = doc.ast();
    let Some(heading) = ast.parent(link) else {
        return false;
    };
    matches!(ast.kind(heading), NodeKind::Heading(heading) if !heading.setext)
        && ast.children(heading) == [link]
        && link_syntax(doc, link)
            .and_then(|syntax| syntax.destination)
            .is_some_and(|destination| doc.text()[destination.start..].starts_with("#tab/"))
}
