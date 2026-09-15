//! macOS bootstrap for the component UI.

mod actions;
mod assets;
mod common;
mod platform;
mod ui;

use assets::ProductAssets;
use gpui_kit::component::Root;
use gpui_kit::*;

pub fn run() {
    gpui_kit::application()
        .with_assets(ProductAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            if let Ok(platform) = platform::Platform::start() {
                cx.set_global(platform);
            }
            actions::bind(cx);
            ui::bind(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1320.), px(820.)), cx)),
                window_min_size: Some(size(px(1100.), px(720.))),
                ..gpui_kit::component::TitleBar::window_options()
            };
            cx.spawn(async move |cx| {
                if let Err(error) = cx.open_window(options, |window, cx| {
                    window.set_window_title("Taypeer");
                    let view = cx.new(|cx| ui::AppView::new(window, cx));
                    cx.activate(true);
                    cx.new(|cx| Root::new(view, window, cx))
                }) {
                    eprintln!("Could not open the Taypeer UI window: {error}");
                }
            })
            .detach();
        });
}
