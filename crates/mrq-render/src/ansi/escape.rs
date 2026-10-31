//! Styled lines to text with escape sequences: SGR for styles (true colour or the 256
//! palette), OSC 8 for hyperlinks, inline image escapes for graphics.

use std::fmt::Write as _;
use std::path::Path;

use mrq_core::validate::Target;

use super::{Capabilities, ColorLevel, Line, StyledSpan, TextStyle, graphics};
use crate::theme::Color;

/// `lines` for a terminal with `caps`; relative links resolve against `source`, the
/// document's path.
pub(super) fn write(lines: &[Line], caps: Capabilities, source: Option<&Path>) -> String {
    let mut out = String::new();
    for line in lines {
        let graphic = line.graphic.as_ref().and_then(|graphic| {
            let protocol = caps.graphics?;
            graphics::escape(graphic, protocol)
        });
        let mut spans: Vec<&StyledSpan> = line.spans.iter().collect();
        if graphic.is_none() {
            spans.extend(line.graphic.iter().flat_map(|g| &g.fallback));
        }
        let trimmed = trim_end(&spans, caps.color);
        let mut link: Option<String> = None;
        let mut style = TextStyle::default();
        for (span, text) in spans.iter().zip(&trimmed) {
            if text.is_empty() {
                continue;
            }
            let uri = span
                .link
                .as_deref()
                .filter(|_| caps.hyperlinks)
                .and_then(|l| hyperlink(l, source));
            if uri != link {
                if link.is_some() {
                    out.push_str("\x1b]8;;\x1b\\");
                }
                if let Some(uri) = &uri {
                    let _ = write!(out, "\x1b]8;;{uri}\x1b\\");
                }
                link = uri;
            }
            if caps.color != ColorLevel::None && span.style != style {
                out.push_str(&sgr(span.style, caps.color));
                style = span.style;
            }
            out.push_str(text);
        }
        if link.is_some() {
            out.push_str("\x1b]8;;\x1b\\");
        }
        if style != TextStyle::default() {
            out.push_str("\x1b[0m");
        }
        if let Some(graphic) = graphic {
            out.push_str(&graphic);
        }
        out.push('\n');
    }
    out
}

/// The text of each span with the line's trailing whitespace dropped where nothing
/// shows it: all of it without colour, else where the style paints no background.
fn trim_end<'a>(spans: &[&'a StyledSpan], color: ColorLevel) -> Vec<&'a str> {
    let mut texts: Vec<&str> = spans.iter().map(|span| span.text.as_str()).collect();
    for (span, text) in spans.iter().zip(texts.iter_mut()).rev() {
        let visible = color != ColorLevel::None
            && (span.style.bg.is_some() || span.style.reverse || span.style.underline);
        if visible {
            break;
        }
        *text = text.trim_end();
        if !text.is_empty() {
            break;
        }
    }
    texts
}

/// The SGR sequence for `style`, starting with a reset.
fn sgr(style: TextStyle, level: ColorLevel) -> String {
    let mut codes = vec!["0".to_owned()];
    let flags = [
        (style.bold, "1"),
        (style.dim, "2"),
        (style.italic, "3"),
        (style.underline, "4"),
        (style.reverse, "7"),
        (style.strikethrough, "9"),
    ];
    codes.extend(
        flags
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, code)| (*code).to_owned()),
    );
    let color = |base: u8, c: Color| match level {
        ColorLevel::TrueColor => format!("{base};2;{};{};{}", c.r, c.g, c.b),
        _ => format!("{base};5;{}", palette(c)),
    };
    codes.extend(style.fg.map(|c| color(38, c)));
    codes.extend(style.bg.map(|c| color(48, c)));
    format!("\x1b[{}m", codes.join(";"))
}

/// The closest colour of the xterm 256 palette: its 6×6×6 cube or its grey ramp.
fn palette(c: Color) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |v: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(v)).abs())
            .unwrap_or(0)
    };
    let distance = |(r, g, b): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(r, c.r) + d(g, c.g) + d(b, c.b)
    };
    let (r, g, b) = (nearest(c.r), nearest(c.g), nearest(c.b));
    let cube = (LEVELS[r], LEVELS[g], LEVELS[b]);
    let average = (u16::from(c.r) + u16::from(c.g) + u16::from(c.b)) / 3;
    let grey = u8::try_from((average.saturating_sub(3) / 10).min(23)).unwrap_or(23);
    let grey_value = 8 + 10 * grey;
    if distance((grey_value, grey_value, grey_value)) < distance(cube) {
        232 + grey
    } else {
        u8::try_from(16 + 36 * r + 6 * g + b).unwrap_or(16)
    }
}

/// The OSC 8 URI of a link: URLs with a scheme as written, files and anchors as
/// `file://` URIs of the resolved path when the document has one; `None` for wikilinks
/// and for anchors and relative files of a document without a path.
fn hyperlink(link: &str, source: Option<&Path>) -> Option<String> {
    let file = |path: &Path, anchor: Option<&str>| {
        let path = std::path::absolute(path).ok()?;
        let mut uri = String::from("file://");
        let path = path.to_string_lossy().replace('\\', "/");
        if !path.starts_with('/') {
            uri.push('/');
        }
        encode(&mut uri, &path, false);
        if let Some(anchor) = anchor {
            uri.push('#');
            encode(&mut uri, anchor, false);
        }
        Some(uri)
    };
    if link.starts_with("[[") {
        return None;
    }
    match Target::parse(link) {
        Target::External(_) | Target::Mailto(_) => {
            let mut uri = String::new();
            encode(&mut uri, link, true);
            Some(uri)
        }
        Target::Empty => file(source?, None),
        Target::Anchor(anchor) => file(source?, Some(&anchor)),
        Target::File { path, anchor } => {
            let path = if path.is_absolute() {
                path
            } else {
                source?.parent()?.join(path)
            };
            file(&path, anchor.as_deref())
        }
        _ => None,
    }
}

/// Appends `text` percent-encoded where a URI may not hold it raw: bytes outside the
/// printable ASCII range, spaces, and `%` unless `keep_percent`.
fn encode(out: &mut String, text: &str, keep_percent: bool) {
    for byte in text.bytes() {
        match byte {
            b'%' if keep_percent => out.push('%'),
            b'!'..=b'~' if byte != b'%' => out.push(char::from(byte)),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_picks_cube_and_grey_entries() {
        assert_eq!(palette(Color::rgb(0, 0, 0)), 16);
        assert_eq!(palette(Color::rgb(255, 255, 255)), 231);
        assert_eq!(palette(Color::rgb(0xff, 0, 0)), 196);
        assert_eq!(palette(Color::rgb(0x80, 0x80, 0x80)), 244);
        assert_eq!(palette(Color::rgb(0x1a, 0x5f, 0xb4)), 25);
    }

    #[test]
    fn uris_are_percent_encoded() {
        assert_eq!(
            hyperlink("https://a.example/x y?q=%20#ü", None).as_deref(),
            Some("https://a.example/x%20y?q=%20#%C3%BC")
        );
        assert_eq!(
            hyperlink("mailto:a@b.example", None).as_deref(),
            Some("mailto:a@b.example")
        );
        assert_eq!(hyperlink("#x", None), None);
        assert_eq!(hyperlink("other.md", None), None);
        assert_eq!(hyperlink("[[Note]]", None), None);
        assert_eq!(
            hyperlink("a%20b.md#c", Some(Path::new("/docs/x.md"))).as_deref(),
            Some("file:///docs/a%20b.md#c")
        );
    }
}
