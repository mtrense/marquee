//! Images and diagrams in the document panel, drawn with ratatui-image.
//!
//! What the terminal can show is a ratatui-image [`Picker`]: a protocol (kitty, iTerm2,
//! sixel, or halfblocks: `▀` cells in two colours, which every terminal can show) and
//! the pixel size of a cell. [`Graphics`] is how to get one: [`Graphics::from_env`]
//! chooses from the environment, and [`Graphics::picker`] asks the terminal through the
//! function it is given (ratatui-image's query on the real terminal, which
//! [`run`](crate::run) does once it is in raw mode; a fake in tests).
//!
//! With a picker, the document is laid out with graphics at the picker's cell size and
//! with the rows of each graphic reserved
//! ([`AnsiOptions::reserve_rows`](mrq_render::ansi::AnsiOptions::reserve_rows)): an
//! image's block has a line per row, so scrolling, the selection and search count rows
//! as for text. The images on screen are drawn over their lines as ratatui-image
//! [`SlicedImage`]s, which leave out the rows above or below the panel for every
//! protocol. Each image is decoded and scaled once per size, when it is first drawn
//! ([`App::cached_images`](crate::App::cached_images) counts them), and one that cannot
//! be decoded shows its alt text on its first row instead.
//! Without a picker, images and diagrams show their text fallbacks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use image::{DynamicImage, ImageFormat};
use mrq_render::ansi::{ColorLevel, Graphic, GraphicsProtocol};
use ratatui::layout::Size;
use ratatui_image::Resize;
use ratatui_image::sliced::SlicedProtocol;

pub use ratatui_image::errors::Errors;
pub use ratatui_image::picker::{Picker, ProtocolType};
pub use ratatui_image::sliced::{SignedPosition, SlicedImage};

/// How many images at one size [`Images`] keeps before it starts afresh.
const CACHED: usize = 64;

/// How `mrq show` gets its [`Picker`]; see the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Graphics {
    /// No images: their alt text, and diagrams as text.
    Off,
    /// Halfblocks, without asking the terminal.
    Halfblocks,
    /// The protocol and cell size the terminal answers with; halfblocks if it has no
    /// protocol or does not answer, when `halfblocks` (else no images).
    Detect { halfblocks: bool },
    /// This protocol, at the cell size the terminal answers with (else 10×20 pixels).
    Protocol(GraphicsProtocol),
}

impl Graphics {
    /// The graphics for a terminal with colours of `level`, with `env` looking up
    /// environment variables. As for `render -o ansi`
    /// ([`Capabilities::detect`](mrq_render::ansi::Capabilities::detect)), `MRQ_GRAPHICS`
    /// decides when set: `kitty`, `iterm2` or `sixel` force that protocol, `none` (or
    /// anything else) turns images off, and `halfblocks` uses those. `TERM=dumb` has no
    /// images. Inside tmux or screen, which drop image escapes, images are halfblocks.
    /// Otherwise the terminal is asked. Halfblocks need colours, so without them (`level`
    /// is [`ColorLevel::None`]) only a protocol shows images.
    pub fn from_env(level: ColorLevel, env: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| env(name).filter(|value| !value.is_empty());
        let term = var("TERM").unwrap_or_default();
        let colored = level != ColorLevel::None;
        let forced = var("MRQ_GRAPHICS").map(|value| value.to_ascii_lowercase());
        let protocol = match forced.as_deref() {
            None => None,
            Some("halfblocks") => return Self::Halfblocks,
            Some("kitty") => Some(GraphicsProtocol::Kitty),
            Some("iterm2" | "iterm") => Some(GraphicsProtocol::Iterm2),
            Some("sixel") => Some(GraphicsProtocol::Sixel),
            Some(_) => return Self::Off,
        };
        if term == "dumb" {
            Self::Off
        } else if var("TMUX").is_some() || term.starts_with("screen") {
            if colored { Self::Halfblocks } else { Self::Off }
        } else if let Some(protocol) = protocol {
            Self::Protocol(protocol)
        } else {
            Self::Detect {
                halfblocks: colored,
            }
        }
    }

    /// The picker these graphics stand for, with `query` asking the terminal (only for
    /// [`Detect`](Self::Detect) and [`Protocol`](Self::Protocol)); `None` for no images.
    pub fn picker(self, query: impl FnOnce() -> Result<Picker, Errors>) -> Option<Picker> {
        match self {
            Self::Off => None,
            Self::Halfblocks => Some(Picker::halfblocks()),
            Self::Detect { halfblocks } => match query() {
                Ok(picker) if picker.protocol_type() != ProtocolType::Halfblocks => Some(picker),
                Ok(picker) => halfblocks.then_some(picker),
                Err(_) => halfblocks.then(Picker::halfblocks),
            },
            Self::Protocol(protocol) => {
                let mut picker = query().unwrap_or_else(|_| Picker::halfblocks());
                picker.set_protocol_type(match protocol {
                    GraphicsProtocol::Kitty => ProtocolType::Kitty,
                    GraphicsProtocol::Iterm2 => ProtocolType::Iterm2,
                    GraphicsProtocol::Sixel => ProtocolType::Sixel,
                });
                Some(picker)
            }
        }
    }
}

/// The key of a graphic's image in [`Images`]: a hash of its PNG.
pub(crate) fn key(graphic: &Graphic) -> u64 {
    let mut hasher = DefaultHasher::new();
    graphic.image.png.hash(&mut hasher);
    hasher.finish()
}

/// An image scaled for a size; `None` where it failed.
type Scaled = Option<Rc<SlicedProtocol>>;

/// The images drawn with one picker, decoded once and scaled once per size; see the
/// [module docs](self). Filled while drawing, which only borrows the app.
pub(crate) struct Images {
    picker: Option<Picker>,
    /// By [`key`]; `None` for a PNG that does not decode.
    decoded: RefCell<HashMap<u64, Option<DynamicImage>>>,
    /// By [`key`] and size in cells; `None` where the protocol failed.
    protocols: RefCell<HashMap<(u64, Size), Scaled>>,
}

impl Images {
    pub(crate) fn new(picker: Option<Picker>) -> Self {
        Self {
            picker,
            decoded: RefCell::default(),
            protocols: RefCell::default(),
        }
    }

    pub(crate) fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    /// The images scaled for a size so far.
    pub(crate) fn len(&self) -> usize {
        self.protocols.borrow().len()
    }

    /// `graphic` (whose image has the key `key`) for its size in cells; `None` without a
    /// picker, or if it does not decode or encode.
    pub(crate) fn protocol(&self, key: u64, graphic: &Graphic) -> Option<Rc<SlicedProtocol>> {
        let picker = self.picker.as_ref()?;
        let cells = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        let size = Size::new(cells(graphic.columns), cells(graphic.rows));
        if let Some(protocol) = self.protocols.borrow().get(&(key, size)) {
            return protocol.clone();
        }
        let mut protocols = self.protocols.borrow_mut();
        let mut decoded = self.decoded.borrow_mut();
        if protocols.len() >= CACHED {
            protocols.clear();
            decoded.clear();
        }
        let image = decoded.entry(key).or_insert_with(|| {
            image::load_from_memory_with_format(&graphic.image.png, ImageFormat::Png).ok()
        });
        let protocol = image
            .clone()
            .and_then(|image| {
                SlicedProtocol::new_with_resize(picker, image, size, Resize::Fit(None)).ok()
            })
            .map(Rc::new);
        protocols.insert((key, size), protocol.clone());
        protocol
    }
}

/// Protocols are not `Debug`; their number is enough.
impl fmt::Debug for Images {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Images")
            .field("picker", &self.picker)
            .field("protocols", &self.len())
            .finish_non_exhaustive()
    }
}
