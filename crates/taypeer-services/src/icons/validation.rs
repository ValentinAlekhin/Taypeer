use super::IconError;
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;
use taypeer_core::ICON_LIMIT;

pub(super) fn validate(bytes: &[u8]) -> Result<(), IconError> {
    if bytes.len() as u64 > ICON_LIMIT {
        return Err(IconError::TooLarge);
    }
    if bytes.is_empty() {
        return Err(IconError::InvalidImage);
    }
    match image::guess_format(bytes) {
        Ok(
            format @ (ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::WebP
            | ImageFormat::Gif
            | ImageFormat::Ico),
        ) => {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            let mut limits = Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(128 * 1024 * 1024);
            reader.limits(limits);
            reader.decode().map_err(|_| IconError::InvalidImage)?;
            Ok(())
        }
        _ => validate_svg(bytes),
    }
}

fn validate_svg(bytes: &[u8]) -> Result<(), IconError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IconError::InvalidImage)?;
    let xml = roxmltree::Document::parse(text).map_err(|_| IconError::InvalidImage)?;
    if xml.root_element().tag_name().name() != "svg" {
        return Err(IconError::InvalidImage);
    }
    for (count, node) in xml.descendants().enumerate() {
        if count > 10_000 {
            return Err(IconError::TooLarge);
        }
        if matches!(
            node.tag_name().name(),
            "script" | "foreignObject" | "image" | "style" | "animate" | "animateTransform" | "set"
        ) {
            return Err(IconError::InvalidImage);
        }
        for attr in node.attributes() {
            if attr.value().len() > 65_536 {
                return Err(IconError::TooLarge);
            }
            let value = attr.value().to_ascii_lowercase();
            if attr.name().starts_with("on")
                || (attr.name() == "href" && !value.starts_with('#'))
                || (value.contains("url(") && !value.starts_with("url(#"))
            {
                return Err(IconError::InvalidImage);
            }
        }
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(text, &options).map_err(|_| IconError::InvalidImage)?;
    let size = tree.size();
    if size.width() > 4096.0 || size.height() > 4096.0 {
        return Err(IconError::TooLarge);
    }
    Ok(())
}
