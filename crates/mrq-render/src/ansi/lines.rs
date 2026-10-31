//! The backend-neutral model: [`Line`]s of [`StyledSpan`]s, with what each line shows
//! ([`BlockInfo`]) and an optional [`Graphic`]. The ANSI serializer and the TUI both
//! read it.

use std::sync::Arc;

use mrq_core::ast::NodeId;
use mrq_core::source::ByteRange;
use unicode_width::UnicodeWidthStr;

use crate::theme::Color;

/// How a [`StyledSpan`] looks. `None` colors keep the terminal's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag per terminal attribute"
)]
pub struct TextStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Foreground and background swapped.
    pub reverse: bool,
}

impl TextStyle {
    /// `self` with what `over` sets: its colors where set, and every flag that is on in
    /// either.
    #[must_use]
    pub fn or(self, over: Self) -> Self {
        Self {
            fg: over.fg.or(self.fg),
            bg: over.bg.or(self.bg),
            bold: self.bold || over.bold,
            dim: self.dim || over.dim,
            italic: self.italic || over.italic,
            underline: self.underline || over.underline,
            strikethrough: self.strikethrough || over.strikethrough,
            reverse: self.reverse || over.reverse,
        }
    }
}

/// A run of text in one style, maybe a link; never contains a line break or a control
/// character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    pub text: String,
    pub style: TextStyle,
    /// The link destination as written in the source (`https://…`, `other.md#x`,
    /// `#anchor`), or a wikilink's `target#anchor`. The serializer turns it into an OSC 8
    /// hyperlink where it can.
    pub link: Option<String>,
}

impl StyledSpan {
    /// Unlinked text.
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
            link: None,
        }
    }

    /// The display width in terminal columns.
    pub fn width(&self) -> usize {
        self.text.width()
    }
}

/// Where a line comes from: the innermost block it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    /// The block node in the document's AST (a paragraph, heading, code block, table
    /// row, …; the container for its prefix-only lines).
    pub node: NodeId,
    /// The block's span in the source.
    pub span: ByteRange,
    /// Set on the lines of a heading, for outlines.
    pub heading: Option<HeadingInfo>,
}

/// A heading's level and anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingInfo {
    pub level: u8,
    /// The anchor the flavor's slugger gives the heading.
    pub anchor: Option<String>,
}

/// A PNG image with its size in pixels.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    /// The PNG file.
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Image {
    /// `png` if it is a PNG file, with the size from its header.
    pub fn from_png(png: Vec<u8>) -> Option<Self> {
        const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
        if !png.starts_with(SIGNATURE) || png.get(12..16) != Some(b"IHDR") {
            return None;
        }
        let number = |at: usize| Some(u32::from_be_bytes(png.get(at..at + 4)?.try_into().ok()?));
        let (width, height) = (number(16)?, number(20)?);
        (width > 0 && height > 0).then_some(Self { png, width, height })
    }
}

/// PNG bytes would flood test failures; the size is enough.
impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Image({}x{}, {} bytes)",
            self.width,
            self.height,
            self.png.len()
        )
    }
}

/// An image shown in a line, after the line's spans (its container prefix), on
/// `rows` terminal rows of `columns` cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graphic {
    pub image: Arc<Image>,
    pub columns: usize,
    pub rows: usize,
    /// The image size in pixels it is shown at (at most its own size).
    pub pixels: (u32, u32),
    /// What to show instead, without a graphics protocol: the alt text.
    pub fallback: Vec<StyledSpan>,
}

/// One terminal line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub spans: Vec<StyledSpan>,
    /// The block the line shows; `None` for blank lines between blocks.
    pub block: Option<BlockInfo>,
    /// An image after the spans.
    pub graphic: Option<Graphic>,
}

impl Line {
    /// A line of `spans`.
    pub fn new(spans: Vec<StyledSpan>) -> Self {
        Self {
            spans,
            ..Self::default()
        }
    }

    /// The display width without a graphics protocol: spans, then the fallback.
    pub fn width(&self) -> usize {
        self.spans
            .iter()
            .chain(self.fallback())
            .map(StyledSpan::width)
            .sum()
    }

    /// The text without styles, as [`width`](Self::width) measures it.
    pub fn text(&self) -> String {
        self.spans
            .iter()
            .chain(self.fallback())
            .map(|span| span.text.as_str())
            .collect()
    }

    fn fallback(&self) -> impl Iterator<Item = &StyledSpan> {
        self.graphic.iter().flat_map(|graphic| &graphic.fallback)
    }
}
