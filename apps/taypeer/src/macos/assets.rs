//! Bundled product icons with the kit assets as fallback.

use gpui_kit::assets::Assets;
use gpui_kit::{AssetSource, SharedString};

pub(super) struct ProductAssets;
impl AssetSource for ProductAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let svg: Option<&'static [u8]> = match path {
            "product/pencil.svg" => Some(br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M21.174 6.812a1 1 0 0 0-3.986-3.986L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/></svg>"#),
            "product/x.svg" => Some(br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"><path d="m18 6-12 12M6 6l12 12"/></svg>"#),
            _ => None,
        };
        if let Some(svg) = svg {
            Ok(Some(std::borrow::Cow::Borrowed(svg)))
        } else {
            Assets.load(path)
        }
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Assets.list(path)
    }
}
