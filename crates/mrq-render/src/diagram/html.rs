//! [`DiagramHook`]: diagrams in the HTML backend.

use std::sync::Arc;

use mrq_core::ast::CodeBlock;

use super::{DiagramFormat, DiagramOutput, Registry, codes};
use crate::html::escape::push_escaped;
use crate::html::{CodeBlockContext, CodeBlockHook};

/// What HTML shows, best first.
const ACCEPT: &[DiagramFormat] = &[DiagramFormat::Svg, DiagramFormat::Png, DiagramFormat::Text];

/// A [`CodeBlockHook`] that renders fenced blocks in a diagram language through a
/// [`Registry`], as `<figure class="diagram" data-lang="…">` holding inline SVG (XML
/// prolog, doctype and comments before `<svg` dropped), a PNG `<img>` with a data URI, or
/// a `<pre>` of text. A diagram that fails is left to the highlighted code block, with a
/// [`codes::DIAGRAM`] warning.
#[derive(Debug, Clone)]
pub struct DiagramHook {
    registry: Arc<Registry>,
}

impl DiagramHook {
    /// Renders with `registry`, which may be shared with other backends.
    pub fn new(registry: impl Into<Arc<Registry>>) -> Self {
        Self {
            registry: registry.into(),
        }
    }
}

impl CodeBlockHook for DiagramHook {
    fn render(&self, block: &CodeBlock, cx: &mut CodeBlockContext<'_>) -> Option<String> {
        block.fence?;
        let lang = block.lang()?;
        match self.registry.render(lang, &block.literal, ACCEPT)? {
            Ok(rendered) => Some(figure(lang, &rendered.output)),
            Err(failure) => {
                cx.warn(
                    codes::DIAGRAM,
                    format!("the `{lang}` diagram is shown as code: {failure}"),
                );
                None
            }
        }
    }
}

/// The figure HTML of `output`.
fn figure(lang: &str, output: &DiagramOutput) -> String {
    let mut html = String::from("<figure class=\"diagram\" data-lang=\"");
    push_escaped(&mut html, lang);
    html.push_str("\">\n");
    match output {
        DiagramOutput::Svg(svg) => {
            let start = svg.find("<svg").unwrap_or(0);
            html.push_str(svg[start..].trim_end());
            html.push('\n');
        }
        DiagramOutput::Png(bytes) => {
            html.push_str("<img src=\"data:image/png;base64,");
            html.push_str(&base64(bytes));
            html.push_str("\" alt=\"");
            push_escaped(&mut html, lang);
            html.push_str(" diagram\" />\n");
        }
        DiagramOutput::Text(text) => {
            html.push_str("<pre>");
            push_escaped(&mut html, text);
            html.push_str("</pre>\n");
        }
    }
    html.push_str("</figure>\n");
    html
}

/// `bytes` in standard base64 with padding.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 0x3f) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_rfc_4648() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }
}
