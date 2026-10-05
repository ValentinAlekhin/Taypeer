//! Taypeer UI prototype and portable headless service smoke.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use taypeer_runtime_client as backend;
mod launch;
#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    feature = "ui-test-support"
))]
use taypeer_settings_ui::local_settings;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod desktop;
#[cfg(any(test, feature = "ui-test-support"))]
mod smoke;
#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    feature = "ui-test-support"
))]
pub use desktop::testing;
#[cfg(any(target_os = "macos", target_os = "linux"))]
rust_i18n::i18n!("locales", fallback = "en");

/// Start the native application or its private worker mode.
pub fn run() {
    #[cfg(feature = "ui-test-support")]
    if std::env::args().skip(1).eq(["__public_fixture_worker"]) {
        let result = taypeer_runtime::run_test_worker(std::io::stdin(), std::io::stdout());
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
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
        #[cfg(any(test, feature = "ui-test-support"))]
        {
            smoke::run();
            return;
        }
        #[cfg(not(any(test, feature = "ui-test-support")))]
        {
            eprintln!("The public demo smoke requires the ui-test-support feature.");
            std::process::exit(2);
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    match mode {
        launch::LaunchMode::Ui => {
            desktop::run(launch::profile(&args).expect("validated UI arguments"))
        }
        launch::LaunchMode::Smoke => unreachable!("smoke mode returned above"),
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        eprintln!("The GUI requires macOS or Linux. Use --smoke-test for the portable demo.");
        std::process::exit(2);
    }
}
