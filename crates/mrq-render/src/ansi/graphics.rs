//! Inline image escapes: the kitty graphics protocol, iTerm2's `OSC 1337 File` and DEC
//! sixel (with the `png` feature, which decodes the PNG).

use std::fmt::Write as _;

use super::{Graphic, GraphicsProtocol};
use crate::diagram::base64;

/// The escape that shows `graphic` with `protocol`; `None` when it cannot be written
/// (sixel without the `png` feature, or a PNG that does not decode).
pub(super) fn escape(graphic: &Graphic, protocol: GraphicsProtocol) -> Option<String> {
    match protocol {
        GraphicsProtocol::Kitty => Some(kitty(graphic)),
        GraphicsProtocol::Iterm2 => Some(iterm2(graphic)),
        GraphicsProtocol::Sixel => sixel(graphic),
    }
}

/// A kitty `a=T` (transmit and display) of the PNG (`f=100`), in chunks of at most 4096
/// base64 bytes, scaled to the graphic's cells; `q=2` keeps the terminal from answering.
fn kitty(graphic: &Graphic) -> String {
    let data = base64(&graphic.image.png);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    let mut out = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let chunk = std::str::from_utf8(chunk).unwrap_or_default();
        if i == 0 {
            let _ = write!(
                out,
                "\x1b_Ga=T,f=100,t=d,q=2,c={},r={},m={more};{chunk}\x1b\\",
                graphic.columns, graphic.rows
            );
        } else {
            let _ = write!(out, "\x1b_Gm={more};{chunk}\x1b\\");
        }
    }
    out
}

/// An iTerm2 inline file, `width` and `height` in cells.
fn iterm2(graphic: &Graphic) -> String {
    format!(
        "\x1b]1337;File=inline=1;size={};width={};height={};preserveAspectRatio=1:{}\x07",
        graphic.image.png.len(),
        graphic.columns,
        graphic.rows,
        base64(&graphic.image.png)
    )
}

#[cfg(not(feature = "png"))]
fn sixel(_graphic: &Graphic) -> Option<String> {
    None
}

/// Sixel at the graphic's pixel size (nearest-neighbour scaled), in the 216 colours of
/// a 6×6×6 cube; pixels less than half opaque are left transparent.
#[cfg(feature = "png")]
fn sixel(graphic: &Graphic) -> Option<String> {
    use resvg::tiny_skia::Pixmap;

    let pixmap = Pixmap::decode_png(&graphic.image.png).ok()?;
    let (width, height) = (graphic.pixels.0.max(1), graphic.pixels.1.max(1));
    let (source_width, source_height) = (pixmap.width(), pixmap.height());
    let pixels = pixmap.pixels();
    // The palette index of each pixel, `None` when transparent.
    let index = |x: u32, y: u32| -> Option<u8> {
        let scale = |at: u32, source: u32, target: u32| {
            usize::try_from(u64::from(at) * u64::from(source) / u64::from(target)).ok()
        };
        let (sx, sy) = (
            scale(x, source_width, width)?,
            scale(y, source_height, height)?,
        );
        let stride = usize::try_from(source_width).ok()?;
        let pixel = pixels.get(sy * stride + sx)?.demultiply();
        if pixel.alpha() < 128 {
            return None;
        }
        let level = |c: u8| (u16::from(c) * 5 + 127) / 255;
        let i = level(pixel.red()) * 36 + level(pixel.green()) * 6 + level(pixel.blue());
        u8::try_from(i).ok()
    };
    let grid: Vec<Vec<Option<u8>>> = (0..height)
        .map(|y| (0..width).map(|x| index(x, y)).collect())
        .collect();
    let mut used = [false; 216];
    for i in grid.iter().flatten().flatten() {
        used[usize::from(*i)] = true;
    }

    let mut out = format!("\x1bPq\"1;1;{width};{height}");
    for (i, _) in used.iter().enumerate().filter(|(_, used)| **used) {
        let (r, g, b) = (i / 36, i / 6 % 6, i % 6);
        let _ = write!(out, "#{i};2;{};{};{}", r * 20, g * 20, b * 20);
    }
    for band in grid.chunks(6) {
        let mut first = true;
        for color in (0..216u8).filter(|&c| used[usize::from(c)]) {
            let row: Vec<u8> = (0..band[0].len())
                .map(|x| {
                    band.iter()
                        .enumerate()
                        .filter(|(_, line)| line[x] == Some(color))
                        .fold(0u8, |bits, (y, _)| bits | 1 << y)
                })
                .collect();
            if row.iter().all(|&bits| bits == 0) {
                continue;
            }
            if !first {
                out.push('$');
            }
            first = false;
            let _ = write!(out, "#{color}");
            let mut x = 0;
            while x < row.len() {
                let run = row[x..].iter().take_while(|&&b| b == row[x]).count();
                let c = char::from(63 + row[x]);
                if run > 3 {
                    let _ = write!(out, "!{run}{c}");
                } else {
                    out.extend(std::iter::repeat_n(c, run));
                }
                x += run;
            }
        }
        out.push('-');
    }
    out.push_str("\x1b\\");
    Some(out)
}
