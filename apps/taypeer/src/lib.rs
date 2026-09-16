//! Taypeer UI prototype and portable headless service smoke.

#[cfg(target_os = "macos")]
use taypeer_runtime_client as backend;
mod launch;
#[cfg(all(target_os = "macos", feature = "ui-test-support"))]
use taypeer_settings_ui::local_settings;
#[cfg(target_os = "macos")]
mod desktop;
mod smoke;
#[cfg(all(target_os = "macos", feature = "ui-test-support"))]
pub use desktop::testing;
#[cfg(target_os = "macos")]
rust_i18n::i18n!("locales", fallback = "en");

/// Start the native application or its private worker mode.
pub fn run() {
    #[cfg(target_os = "macos")]
    if std::env::args().skip(1).eq(["__platform-helper"]) {
        println!("{}", taypeer_desktop_platform::helper_path());
        return;
    }
    if std::env::args().skip(1).eq(["__worker"]) {
        let result = taypeer_runtime::run_worker(std::io::stdin(), std::io::stdout());
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match launch::LaunchMode::parse(&args) {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    if mode == launch::LaunchMode::Smoke {
        smoke::run();
        return;
    }
    #[cfg(target_os = "macos")]
    match mode {
        launch::LaunchMode::Ui => {
            desktop::run(launch::profile(&args).expect("validated UI arguments"))
        }
        launch::LaunchMode::Smoke => unreachable!("smoke mode returned above"),
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("The GUI requires macOS. Use --smoke-test for the portable demo.");
        std::process::exit(2);
    }
}
