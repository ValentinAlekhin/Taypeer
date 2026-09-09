//! Synthetic, process-local Taypeer vertical slice. Never use real secrets.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod preferences;
mod smoke;
#[cfg(target_os = "macos")]
rust_i18n::i18n!("locales", fallback = "en");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--smoke-test") {
        smoke::run();
        return;
    }
    if args.iter().any(|arg| arg != "--demo") {
        eprintln!("Usage: taypeer [--demo | --smoke-test]");
        std::process::exit(2);
    }
    #[cfg(target_os = "macos")]
    macos::run();
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("The GUI requires macOS. Use --smoke-test for the portable demo.");
        std::process::exit(2);
    }
}
