//! macOS bootstrap for the component UI.

mod actions;
mod assets;
mod common;
mod file_picker;
mod platform;
mod ui;

use assets::ProductAssets;
use gpui_kit::component::Root;
use gpui_kit::*;

pub(super) struct LaunchProfile(pub Option<std::path::PathBuf>);
#[cfg(feature = "ui-test-support")]
pub(super) struct TestLaunch {
    pub preferences: std::path::PathBuf,
    pub worker: std::path::PathBuf,
}
#[cfg(feature = "ui-test-support")]
impl Global for TestLaunch {}
#[cfg(feature = "ui-test-support")]
pub mod testing;
impl Global for LaunchProfile {}

pub fn run(profile: Option<std::path::PathBuf>) {
    gpui_kit::application()
        .with_assets(ProductAssets)
        .run(move |cx| {
            cx.set_global(LaunchProfile(profile));
            cx.set_global(file_picker::FileDialogs::native());
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
