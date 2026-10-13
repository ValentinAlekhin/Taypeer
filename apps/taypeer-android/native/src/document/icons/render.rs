//! Presentation-only thumbnails from content already accepted by the common validator.
use super::{AndroidError, IconOrigin, RenderedIcon, icon_error};
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;
use zeroize::Zeroize;

const THUMBNAIL_SIDE: u32 = 256;
// A native decoder allocation budget, independent of any Kotlin validation.
const DECODE_ALLOCATION: u64 = 128 * 1024 * 1024;

pub(super) fn thumbnail(
    original: &[u8],
    identity: String,
    origin: IconOrigin,
) -> Result<RenderedIcon, AndroidError> {
    taypeer_services::icons::validate(original).map_err(icon_error)?;
    let (width, height, png) = match image::guess_format(original) {
        Ok(format) => raster(original, format)?,
        Err(_) => svg(original)?,
    };
    if png.len() as u64 > taypeer_core::ICON_LIMIT {
        return Err(invalid());
    }
    Ok(RenderedIcon {
        identity,
        width,
        height,
        png,
        origin,
    })
}

fn invalid() -> AndroidError {
    icon_error(taypeer_services::icons::IconError::InvalidImage)
}
fn raster(original: &[u8], format: ImageFormat) -> Result<(u32, u32, Vec<u8>), AndroidError> {
    let mut reader = ImageReader::with_format(Cursor::new(original), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(DECODE_ALLOCATION);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| invalid())?;
    let image = if decoded.width() > THUMBNAIL_SIDE || decoded.height() > THUMBNAIL_SIDE {
        decoded.thumbnail(THUMBNAIL_SIDE, THUMBNAIL_SIDE)
    } else {
        decoded
    };
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|_| invalid())?;
    Ok((image.width(), image.height(), png.into_inner()))
}
fn svg(original: &[u8]) -> Result<(u32, u32, Vec<u8>), AndroidError> {
    let options = resvg::usvg::Options {
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(original, &options).map_err(|_| invalid())?;
    let size = tree.size();
    let scale = (THUMBNAIL_SIDE as f32 / size.width().max(size.height())).min(1.0);
    let width = (size.width() * scale)
        .ceil()
        .clamp(1.0, THUMBNAIL_SIDE as f32) as u32;
    let height = (size.height() * scale)
        .ceil()
        .clamp(1.0, THUMBNAIL_SIDE as f32) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(invalid)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png();
    pixmap.data_mut().zeroize();
    let png = png.map_err(|_| invalid())?;
    Ok((width, height, png))
}
