//! macOS application bootstrap. UI state and screens live in `client`.

mod actions;
mod assets;
mod client;
mod common;

use assets::ProductAssets;
use client::Client;
use gpui_kit::component::Root;
use gpui_kit::*;

pub fn run(demo_mode: bool) {
    gpui_kit::application()
        .with_assets(ProductAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            actions::bind(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1320.), px(820.)), cx)),
                window_min_size: Some(size(px(1100.), px(720.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Taypeer".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    let view = cx.new(|cx| Client::new(window, cx, demo_mode));
                    cx.activate(true);
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("Could not open Taypeer window");
            })
            .detach();
        });
}
