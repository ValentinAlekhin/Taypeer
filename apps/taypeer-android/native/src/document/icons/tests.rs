//! PUBLIC image fixtures verify actual pixels, finite previews and ephemeral capability cleanup.
use super::*;
use crate::SelectedOutput;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::{
    io::Cursor,
    sync::atomic::{AtomicUsize, Ordering},
};

fn raster(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb([220, 20, 10])));
    // ICO's embedded PNG requires RGBA, while JPEG accepts the RGB fixture.
    let image = if format == ImageFormat::Ico {
        DynamicImage::ImageRgba8(image.to_rgba8())
    } else {
        image
    };
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, format).unwrap();
    output.into_inner()
}

#[test]
fn all_six_shared_formats_render_actual_pixels_as_png_without_exposing_originals() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#dc140a"/></svg>"##;
    let mut originals = vec![(None, svg.to_vec())];
    for format in [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::WebP,
        ImageFormat::Gif,
        ImageFormat::Ico,
    ] {
        originals.push((Some(format), raster(format, 16, 16)));
    }
    for (format, original) in originals {
        assert!(
            taypeer_services::icons::validate(&original).is_ok(),
            "PUBLIC {format:?} original refused by shared validation"
        );
        let view = render::thumbnail(&original, "PUBLIC image identity".into(), IconOrigin::File)
            .unwrap_or_else(|_| panic!("PUBLIC {format:?} original refused by native renderer"));
        assert_eq!(image::guess_format(&view.png).unwrap(), ImageFormat::Png);
        assert_eq!((view.width, view.height), (16, 16));
        let decoded = image::load_from_memory(&view.png).unwrap().to_rgba8();
        let pixel = decoded.get_pixel(8, 8).0;
        assert!(pixel[0] > 180 && pixel[1] < 50 && pixel[2] < 50);
        assert_eq!(pixel[3], 255);
        assert_eq!(view.origin, IconOrigin::File);
        assert_eq!(format!("{view:?}"), "RenderedIcon([REDACTED])");
    }
}

#[test]
fn preview_bounds_preserve_aspect_and_reject_content_refused_by_shared_policy() {
    let original = raster(ImageFormat::Png, 1024, 512);
    let view =
        render::thumbnail(&original, "PUBLIC large icon".into(), IconOrigin::ImageUrl).unwrap();
    assert_eq!((view.width, view.height), (256, 128));
    assert!(view.png.len() as u64 <= taypeer_core::ICON_LIMIT);
    let oversized_svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="2048"><rect width="4096" height="2048" fill="red"/></svg>"#;
    let view = render::thumbnail(oversized_svg, "PUBLIC svg".into(), IconOrigin::Favicon).unwrap();
    assert_eq!((view.width, view.height), (256, 128));
    for invalid in [
        b"PUBLIC invalid image".as_slice(),
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script>PUBLIC</script></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="http://127.0.0.1/PUBLIC"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000"/>"#,
    ] {
        assert!(matches!(
            render::thumbnail(invalid, "PUBLIC rejected".into(), IconOrigin::File),
            Err(AndroidError::Icon { .. })
        ));
    }
}

#[test]
fn preview_output_requires_exact_offsets_bounded_bytes_and_explicit_completion() {
    let output = memory::ImageOutput::default();
    assert!(output.take().is_err());
    assert!(output.write(1, vec![7]).is_err());
    assert_eq!(output.write(0, vec![9, 8]).unwrap(), 2);
    assert!(output.take().is_err());
    assert!(matches!(
        output.write(2, vec![7; taypeer_core::ICON_LIMIT as usize]),
        Err(AndroidError::Icon {
            failure: IconFailure::TooLarge
        })
    ));
    output.finish().unwrap();
    assert!(output.write(2, vec![7]).is_err());
    assert_eq!(output.take().unwrap().as_slice(), &[9, 8]);
    output.close();
    assert!(output.take().is_err());
    assert!(output.finish().is_err());
}

struct UnopenedInput {
    callbacks: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
}
impl SelectedInput for UnopenedInput {
    fn length(&self) -> Result<u64, AndroidError> {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        Err(AndroidError::InvalidFile)
    }
    fn read(&self, _: u64, _: u32) -> Result<Vec<u8>, AndroidError> {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        Err(AndroidError::InvalidFile)
    }
    fn close(&self) {
        self.closes.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn unopened_input_is_closed_on_preflight_refusal_or_receipt_only_replay() {
    let callbacks = Arc::new(AtomicUsize::new(0));
    let closes = Arc::new(AtomicUsize::new(0));
    let input = InputCapability::new(Box::new(UnopenedInput {
        callbacks: Arc::clone(&callbacks),
        closes: Arc::clone(&closes),
    }));
    drop(input);
    assert_eq!(callbacks.load(Ordering::Relaxed), 0);
    assert_eq!(closes.load(Ordering::Relaxed), 1);
}

#[test]
fn downloaded_original_is_erased_when_its_ephemeral_input_is_closed() {
    let input = memory::ImageInput::new(zeroize::Zeroizing::new(vec![11, 22, 33]));
    assert_eq!(input.length().unwrap(), 3);
    assert_eq!(input.read(1, 64).unwrap(), vec![22, 33]);
    assert!(input.read(0, 65_537).is_err());
    input.close();
    assert_eq!(input.length().unwrap(), 0);
    assert!(input.read(1, 1).is_err());
}
