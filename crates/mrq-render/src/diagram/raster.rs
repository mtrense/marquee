//! [`rasterize`]: SVG to PNG with resvg, the system fonts drawing the text.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb};

use super::DiagramError;

/// The resvg version, for the fingerprints of rasterized output; keep it in step with
/// Cargo.toml.
pub(super) const RESVG: &str = "0.49.0";

/// The largest side of a rasterized image, in pixels, so a huge or hostile SVG cannot
/// allocate gigabytes.
const MAX_SIDE: f32 = 16_384.0;

/// `svg` rasterized to PNG at `scale` times its size (1.0: one pixel per SVG user unit
/// at 96 dpi). Text is drawn with the system fonts, loaded once per process; where a
/// font family is missing, usvg falls back to another face that has the glyphs, and
/// without any fonts text is left out. Fails on SVG that does not parse, a scale that
/// is not positive, and an empty or oversized image.
pub fn rasterize(svg: &str, scale: f32) -> Result<Vec<u8>, DiagramError> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(DiagramError::Failed(format!("invalid PNG scale {scale}")));
    }
    let options = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|error| DiagramError::Failed(format!("the SVG does not parse: {error}")))?;
    let size = tree.size();
    let (width, height) = (size.width() * scale, size.height() * scale);
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(DiagramError::Failed(format!(
            "the PNG would be {width:.0}x{height:.0} pixels, more than {MAX_SIDE:.0} a side"
        )));
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive and at most MAX_SIDE"
    )]
    let pixmap = Pixmap::new(width.ceil() as u32, height.ceil() as u32);
    let mut pixmap =
        pixmap.ok_or_else(|| DiagramError::Failed("the SVG has no area to draw".to_owned()))?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|error| DiagramError::Failed(format!("PNG encoding failed: {error}")))
}

/// The system fonts, loaded on first use.
fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
}
