//! Taypeer UI prototype and portable headless service smoke.

mod backend;
mod launch;
mod local_settings;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod preferences;
mod smoke;
#[cfg(all(target_os = "macos", feature = "ui-test-support"))]
pub use macos::testing;
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        dead_code,
        reason = "UI state is exercised by portable tests; rendering is macOS-only"
    )
)]
mod ui_state;
#[cfg(target_os = "macos")]
rust_i18n::i18n!("locales", fallback = "en");

/// Start the native application or its private worker mode.
pub fn run() {
    #[cfg(target_os = "macos")]
    if std::env::args().skip(1).eq(["__platform-helper"]) {
        println!("{}", env!("TAYPEER_PLATFORM_HELPER"));
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
            macos::run(launch::profile(&args).expect("validated UI arguments"))
        }
        launch::LaunchMode::Smoke => unreachable!("smoke mode returned above"),
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("The GUI requires macOS. Use --smoke-test for the portable demo.");
        std::process::exit(2);
    }
}
