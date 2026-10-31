//! Spec conformance of the HTML backend: every `CommonMark` 0.31.2 and GFM 0.29 spec
//! example renders to the spec's expected HTML, compared after normalization.
//!
//! # Normalization
//!
//! Both sides go through [`normalize`], modelled on the spec's own `normalize.py`:
//! attributes are sorted and written as `name="value"` (a bare attribute gets `""`),
//! void-element slashes (`<br />`) are dropped, character references in text and
//! attribute values are decoded and the result re-escaped the same way, whitespace runs
//! outside `<pre>` become one space and whitespace next to block-level tags is removed.
//! Two rules are mrq's own: heading `id`s are removed (the spec has none, mrq adds the
//! slugger's), and highlighting is off ([`HtmlOptions::highlight`]), which only adds
//! `<span>`s inside code blocks.
//!
//! As in cmark-gfm's spec runner, GFM extensions are on only for the examples of their
//! section (`example table`, …): those run with the `gfm` flavor, every other example
//! with `commonmark`, and GFM's tag filter is on only for the `tagfilter` example.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use mrq_core::document::DocumentOptions;
use mrq_render::Theme;
use mrq_render::html::{HtmlOptions, HtmlRenderer};

const FENCE: &str = "````````````````````````````````";

/// One example of a spec file.
struct Example {
    /// 1-based example number within its file.
    number: usize,
    /// The extension named after `example` (`table`, `tagfilter`, …), if any.
    extension: Option<String>,
    /// The Markdown input, with the spec's `→` replaced by tabs.
    markdown: String,
    /// The expected HTML, likewise.
    html: String,
}

/// The examples of a spec file in mrq-core's fixtures.
fn examples(file: &str) -> Vec<Example> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mrq-core/tests/fixtures/spec")
        .join(file);
    let text = fs::read_to_string(&path).unwrap();
    let mut examples = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(header) = line.strip_prefix(FENCE) else {
            continue;
        };
        let Some(extension) = header.trim_start().strip_prefix("example") else {
            continue;
        };
        let extension = Some(extension.trim()).filter(|e| !e.is_empty());
        let mut markdown = String::new();
        for line in lines.by_ref() {
            if line == "." {
                break;
            }
            markdown.push_str(line);
            markdown.push('\n');
        }
        let mut html = String::new();
        for line in lines.by_ref() {
            if line.starts_with(FENCE) {
                break;
            }
            html.push_str(line);
            html.push('\n');
        }
        examples.push(Example {
            number: examples.len() + 1,
            extension: extension.map(str::to_owned),
            markdown: markdown.replace('→', "\t"),
            html: html.replace('→', "\t"),
        });
    }
    examples
}

/// Renders every example of `file` (with `extension_flavor` when it names an extension,
/// else `commonmark`) and compares it with the expected HTML.
/// `known` lists the examples expected to differ; they must still differ, so the list
/// never outlives a fix.
fn check_spec(file: &str, extension_flavor: &str, known: &[usize]) {
    let examples = examples(file);
    assert!(examples.len() > 600, "{file}: only {}", examples.len());
    let theme = Theme::default();
    let renderer = |tagfilter| {
        let options = HtmlOptions {
            fragment: true,
            highlight: false,
            tagfilter,
        };
        HtmlRenderer::new(&theme, options).unwrap()
    };
    let (plain, filtered) = (renderer(false), renderer(true));
    let mut failures = Vec::new();
    let mut fixed = Vec::new();
    for example in &examples {
        let doc = DocumentOptions::new()
            .flavor(if example.extension.is_some() {
                extension_flavor
            } else {
                "commonmark"
            })
            .parse(example.markdown.as_str())
            .unwrap();
        let tagfilter = example.extension.as_deref() == Some("tagfilter");
        let renderer = if tagfilter { &filtered } else { &plain };
        let actual = renderer.render(&doc);
        let same = normalize(&actual) == normalize(&example.html);
        match (same, known.contains(&example.number)) {
            (false, false) => failures.push(format!(
                "{file} example {}:\n--- markdown\n{}--- expected\n{}--- actual\n{actual}--- normalized expected\n{}\n--- normalized actual\n{}",
                example.number,
                example.markdown,
                example.html,
                normalize(&example.html),
                normalize(&actual),
            )),
            (true, true) => fixed.push(example.number),
            _ => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        fixed.is_empty(),
        "{file}: examples {fixed:?} match now; drop them from the known list"
    );
}

/// Spec examples whose HTML differs, each with its reason.
///
/// - 96, 98: a document starting with a `---` line pair is YAML frontmatter in every
///   mrq flavor, so the thematic break and setext heading of the spec are not there.
const COMMONMARK_KNOWN: &[usize] = &[96, 98];
/// - 66, 68: frontmatter, as `COMMONMARK_KNOWN` 96 and 98.
/// - 398, 426, 434–436, 473–475, 477: the GFM spec (0.29) predates `CommonMark` 0.30's
///   rule for nested strong emphasis (`****foo****` is `<strong><strong>`), which comrak
///   and the `CommonMark` 0.31.2 suite follow.
const GFM_KNOWN: &[usize] = &[66, 68, 398, 426, 434, 435, 436, 473, 474, 475, 477];

#[test]
fn commonmark_spec_examples_render_the_expected_html() {
    check_spec("commonmark-spec.txt", "commonmark", COMMONMARK_KNOWN);
}

#[test]
fn gfm_spec_examples_render_the_expected_html() {
    check_spec("gfm-spec.txt", "gfm", GFM_KNOWN);
}

#[test]
fn normalization_ignores_formatting_only() {
    assert_eq!(
        normalize("<p>a\n  b</p>\n<hr />\n<input type=\"checkbox\" disabled>"),
        normalize("<p>a b</p><hr><input disabled=\"\" type=\"checkbox\" />"),
    );
    assert_eq!(
        normalize("<h2 id=\"x\">A &amp; &#66;</h2>"),
        "<h2>A &amp; B</h2>"
    );
    assert_eq!(
        normalize("<pre><code>a\n  b\n</code></pre>"),
        "<pre><code>a\n  b\n</code></pre>"
    );
    assert_ne!(normalize("<p>a</p>"), normalize("<p>b</p>"));
}

/// Block-level elements, around which whitespace does not matter (`normalize.py`).
const BLOCK_TAGS: &[&str] = &[
    "article",
    "header",
    "aside",
    "hgroup",
    "blockquote",
    "hr",
    "iframe",
    "body",
    "li",
    "map",
    "button",
    "object",
    "canvas",
    "ol",
    "caption",
    "output",
    "col",
    "p",
    "colgroup",
    "pre",
    "dd",
    "progress",
    "div",
    "section",
    "dl",
    "table",
    "td",
    "dt",
    "tbody",
    "embed",
    "textarea",
    "fieldset",
    "tfoot",
    "figcaption",
    "th",
    "figure",
    "thead",
    "footer",
    "tr",
    "form",
    "ul",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "video",
    "script",
    "style",
];

/// `html` in the canonical form described in the module docs.
fn normalize(html: &str) -> String {
    let mut out = String::new();
    let mut in_pre = false;
    // Whether the last token was a block tag, so leading whitespace of text is dropped.
    let mut after_block = false;
    let mut rest = html;
    while !rest.is_empty() {
        if rest.starts_with('<')
            && let Some((tag, len)) = tag(rest)
        {
            let name = tag.name.as_str();
            let block = BLOCK_TAGS.contains(&name);
            if block {
                let trimmed = out.trim_end().len();
                out.truncate(trimmed);
            }
            if name == "pre" {
                in_pre = !tag.closing;
            }
            out.push_str(&tag.render());
            after_block = block;
            rest = &rest[len..];
            continue;
        }
        if let Some(verbatim) = verbatim(rest) {
            out.push_str(verbatim);
            after_block = false;
            rest = &rest[verbatim.len()..];
            continue;
        }
        let first = rest.chars().next().map_or(0, char::len_utf8);
        let end = rest[first..].find('<').map_or(rest.len(), |i| i + first);
        let mut text = decode(&rest[..end]);
        if !in_pre {
            text = collapse(&text);
            if after_block {
                text = text.trim_start().to_owned();
            }
        }
        out.push_str(&escape(&text));
        after_block = false;
        rest = &rest[end..];
    }
    out.trim().to_owned()
}

/// A comment, CDATA section, processing instruction or declaration at the start of
/// `text`, kept as it is.
fn verbatim(text: &str) -> Option<&str> {
    let end = [
        ("<!--", "-->"),
        ("<![CDATA[", "]]>"),
        ("<?", "?>"),
        ("<!", ">"),
    ]
    .iter()
    .find(|(open, _)| text.starts_with(open))
    .and_then(|(open, close)| {
        text[open.len()..]
            .find(close)
            .map(|i| open.len() + i + close.len())
    })?;
    Some(&text[..end])
}

struct Tag {
    name: String,
    closing: bool,
    attributes: Vec<(String, String)>,
}

impl Tag {
    fn render(&self) -> String {
        let mut out = format!("<{}{}", if self.closing { "/" } else { "" }, self.name);
        for (name, value) in &self.attributes {
            let _ = write!(out, " {name}=\"{}\"", escape(value));
        }
        out.push('>');
        out
    }
}

/// A well-formed start or end tag at the start of `text` and its length; `None` for
/// anything else, which is then treated as text.
fn tag(text: &str) -> Option<(Tag, usize)> {
    let bytes = text.as_bytes();
    let mut i = 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_len = text[i..]
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(text.len() - i);
    if name_len == 0 || !bytes[i].is_ascii_alphabetic() {
        return None;
    }
    let name = text[i..i + name_len].to_ascii_lowercase();
    i += name_len;
    let mut attributes = Vec::new();
    loop {
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        match bytes.get(i)? {
            b'>' => {
                i += 1;
                break;
            }
            b'/' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                break;
            }
            _ if i == start || closing => return None,
            _ => {}
        }
        let attr_len = text[i..]
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '=' | '>' | '/' | '"' | '\''))
            .unwrap_or(text.len() - i);
        if attr_len == 0 {
            return None;
        }
        let attr = text[i..i + attr_len].to_ascii_lowercase();
        i += attr_len;
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            if let &quote @ (b'"' | b'\'') = bytes.get(i)? {
                let len = text[i + 1..].find(char::from(quote))?;
                value = decode(&text[i + 1..i + 1 + len]);
                i += len + 2;
            } else {
                let len = text[i..]
                    .find(|c: char| c.is_ascii_whitespace() || c == '>')
                    .unwrap_or(text.len() - i);
                value = decode(&text[i..i + len]);
                i += len;
            }
        }
        attributes.push((attr, value));
    }
    if name.len() == 2 && name.starts_with('h') && !closing {
        attributes.retain(|(attr, _)| attr != "id");
    }
    attributes.sort();
    Some((
        Tag {
            name,
            closing,
            attributes,
        },
        i,
    ))
}

/// `text` with `&amp;`, `&lt;`, `&gt;`, `&quot;` and numeric character references
/// decoded; other named references are left alone (raw HTML keeps them on both sides).
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|&end| end <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                _ => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code).unwrap_or('\u{fffd}')
                }
            };
            Some((c, end + 1))
        });
        if let Some((c, len)) = decoded {
            out.push(c);
            rest = &rest[len..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Every run of spaces, tabs and line breaks as one space.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}
