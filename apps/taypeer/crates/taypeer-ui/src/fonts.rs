//! Embedded desktop typography, shared by the application and headless rendering.

use gpui_kit::App;
use std::borrow::Cow;

pub(crate) const UI_FAMILY: &str = "Inter";

/// Register the bundled Inter faces before creating windows or measuring text.
///
/// Returns the platform font loader's error if a face cannot be registered.
pub fn register(cx: &App) -> gpui_kit::Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!(
            "../../../../../resources/fonts/inter/Inter-Regular.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../../../resources/fonts/inter/Inter-Medium.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../../../resources/fonts/inter/Inter-SemiBold.ttf"
        )),
        Cow::Borrowed(include_bytes!(
            "../../../../../resources/fonts/inter/Inter-Bold.ttf"
        )),
    ])
}
