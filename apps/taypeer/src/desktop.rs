//! Shared desktop bootstrap; native capabilities are implemented for macOS and Linux.

use taypeer_database_ui::actions;
use taypeer_ui::assets;

use taypeer_desktop_platform as platform;
use taypeer_ui::file_picker;
mod ui;

use assets::ProductAssets;
use gpui_kit::component::Root;
use gpui_kit::*;

use taypeer_ui::LaunchProfile;
#[cfg(feature = "ui-test-support")]
use taypeer_ui::TestLaunch;
#[cfg(feature = "ui-test-support")]
pub mod testing;

pub fn run(profile: Option<std::path::PathBuf>) {
    gpui_kit::application()
        .with_assets(ProductAssets)
        .run(move |cx| {
            #[cfg(target_os = "linux")]
            cx.set_app_identity("io.taypeer.Taypeer", "Taypeer");
            cx.set_global(LaunchProfile::new(profile));
            cx.set_global(file_picker::FileDialogs::native());
            if let Err(error) = taypeer_ui::fonts::register(cx) {
                eprintln!("{}: {error}", taypeer_ui::style::tr("ui.font_load_failed"));
                cx.quit();
                return;
            }
            gpui_kit::init(cx);
            if let Ok(platform) = platform::Platform::start() {
                cx.set_global(platform);
            }
            actions::bind(cx);
            ui::bind(cx);
            // Initial native window bounds are platform pixels, independent of app zoom.
            let options = WindowOptions {
                app_id: cfg!(target_os = "linux").then(|| "io.taypeer.Taypeer".to_owned()),
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
