//! Bundled product icons with the kit assets as fallback.

use gpui_kit::assets::Assets;
use gpui_kit::{AssetSource, SharedString};

/// Embedded product icons; no network asset fetches.
pub struct ProductAssets;
impl AssetSource for ProductAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(name) = path
            .strip_prefix("ui-icons/")
            .and_then(|p| p.strip_suffix(".svg"))
        {
            return ui_icon(name);
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Assets.list(path)
    }
}

#[derive(serde::Deserialize)]
struct ExtraIcon {
    name: String,
    paths: Vec<ExtraPath>,
}
#[derive(serde::Deserialize)]
struct ExtraPath {
    data: String,
}

fn ui_icon(name: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
    use std::{borrow::Cow, collections::BTreeMap, sync::LazyLock};
    static EXTRAS: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
        let icons: Vec<ExtraIcon> = serde_json::from_str(include_str!(
            "../../../../../wireframes/assets/lucide-extra.json"
        ))
        .expect("validated embedded Lucide paths");
        icons.into_iter().map(|icon| {
            let paths = icon.paths.iter().map(|path| format!("<path d=\"{}\"/>", path.data)).collect::<String>();
            (icon.name.trim_start_matches("Icon / lucide:").to_owned(), format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.4\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{paths}</svg>"))
        }).collect()
    });
    if let Some(svg) = EXTRAS.get(name) {
        return Ok(Some(Cow::Owned(svg.as_bytes().to_vec())));
    }
    if name == "x" {
        return Ok(Some(Cow::Borrowed(br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"><path d="m18 6-12 12M6 6l12 12"/></svg>"#)));
    }
    let bundled: Option<&'static [u8]> = match name {
        "trash-2" => Some(include_bytes!(
            "../../../../../resources/lucide/trash-2.svg"
        )),
        "download" => Some(include_bytes!(
            "../../../../../resources/lucide/download.svg"
        )),
        "refresh-cw" => Some(include_bytes!(
            "../../../../../resources/lucide/refresh-cw.svg"
        )),
        "bookmark" => Some(include_bytes!(
            "../../../../../resources/lucide/bookmark.svg"
        )),
        "credit-card" => Some(include_bytes!(
            "../../../../../resources/lucide/credit-card.svg"
        )),
        "database" => Some(include_bytes!(
            "../../../../../resources/lucide/database.svg"
        )),
        "file-key-2" => Some(include_bytes!(
            "../../../../../resources/lucide/file-key-2.svg"
        )),
        "file" => Some(include_bytes!("../../../../../resources/lucide/file.svg")),
        "folder-open" => Some(include_bytes!(
            "../../../../../resources/lucide/folder-open.svg"
        )),
        "folder" => Some(include_bytes!("../../../../../resources/lucide/folder.svg")),
        "globe" => Some(include_bytes!("../../../../../resources/lucide/globe.svg")),
        "heart" => Some(include_bytes!("../../../../../resources/lucide/heart.svg")),
        "home" => Some(include_bytes!("../../../../../resources/lucide/home.svg")),
        "key-round" => Some(include_bytes!(
            "../../../../../resources/lucide/key-round.svg"
        )),
        "laptop" => Some(include_bytes!("../../../../../resources/lucide/laptop.svg")),
        "lock" => Some(include_bytes!("../../../../../resources/lucide/lock.svg")),
        "mail" => Some(include_bytes!("../../../../../resources/lucide/mail.svg")),
        "server" => Some(include_bytes!("../../../../../resources/lucide/server.svg")),
        "shield" => Some(include_bytes!("../../../../../resources/lucide/shield.svg")),
        "smartphone" => Some(include_bytes!(
            "../../../../../resources/lucide/smartphone.svg"
        )),
        "star" => Some(include_bytes!("../../../../../resources/lucide/star.svg")),
        "terminal" => Some(include_bytes!(
            "../../../../../resources/lucide/terminal.svg"
        )),
        "user" => Some(include_bytes!("../../../../../resources/lucide/user.svg")),
        "users" => Some(include_bytes!("../../../../../resources/lucide/users.svg")),
        "wallet" => Some(include_bytes!("../../../../../resources/lucide/wallet.svg")),
        _ => None,
    };
    let bytes = match bundled {
        Some(bytes) => Some(Cow::Borrowed(bytes)),
        None => Assets.load(&format!("icons/{name}.svg"))?,
    };
    Ok(bytes.map(|bytes| {
        Cow::Owned(
            String::from_utf8_lossy(&bytes)
                .replace("stroke-width=\"2\"", "stroke-width=\"1.4\"")
                .into_bytes(),
        )
    }))
}
