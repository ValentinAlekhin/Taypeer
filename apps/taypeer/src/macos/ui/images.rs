//! Decoded custom icons are scoped to unlocked databases and removed from GPUI's asset cache on lock.
use gpui_kit::*;
use std::{collections::BTreeMap, sync::Arc};
use taypeer_core::{BlobId, DatabaseId};
use taypeer_services::{IconEncoding, IconPreview};
#[derive(Default)]
pub(super) struct Images(BTreeMap<(DatabaseId, BlobId), Arc<Image>>);
impl Global for Images {}
impl Images {
    pub fn install(database: &DatabaseId, preview: IconPreview, cx: &mut App) {
        let key = (database.clone(), preview.blob);
        if cx.global::<Self>().0.contains_key(&key) {
            return;
        }
        let format = match preview.encoding {
            IconEncoding::Png => ImageFormat::Png,
            IconEncoding::Jpeg => ImageFormat::Jpeg,
            IconEncoding::Webp => ImageFormat::Webp,
            IconEncoding::Gif => ImageFormat::Gif,
            IconEncoding::Ico => ImageFormat::Ico,
            IconEncoding::Svg => ImageFormat::Svg,
        };
        cx.global_mut::<Self>().0.insert(
            key,
            Arc::new(Image::from_bytes(format, preview.bytes.to_vec())),
        );
    }
    pub fn clear(database: &DatabaseId, cx: &mut App) {
        let keys: Vec<_> = cx
            .global::<Self>()
            .0
            .keys()
            .filter(|(db, _)| db == database)
            .cloned()
            .collect();
        for key in keys {
            if let Some(image) = cx.global_mut::<Self>().0.remove(&key) {
                ImageSource::from(image).remove_asset(cx);
            }
        }
    }
    pub fn get(blob: &BlobId, cx: &App) -> Option<Arc<Image>> {
        cx.global::<Self>()
            .0
            .iter()
            .find(|((_, id), _)| id == blob)
            .map(|(_, image)| image.clone())
    }
}
pub(super) fn stored_icon(name: &str, blob: Option<&BlobId>, cx: &App) -> AnyElement {
    if let Some(image) = blob.and_then(|blob| Images::get(blob, cx)) {
        img(image)
            .size(rems(1.))
            .object_fit(ObjectFit::Contain)
            .into_any_element()
    } else {
        super::style::icon(name).into_any_element()
    }
}
