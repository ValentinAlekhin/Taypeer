//! Taypeer UI prototype and portable headless service smoke.

mod launch;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod preferences;
mod smoke;
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

fn main() {
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
        launch::LaunchMode::Ui => macos::run(),
        launch::LaunchMode::Smoke => unreachable!("smoke mode returned above"),
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("The GUI requires macOS. Use --smoke-test for the portable demo.");
        std::process::exit(2);
    }
}
