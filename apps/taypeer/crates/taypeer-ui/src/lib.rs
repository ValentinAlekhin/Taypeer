//! Product presentation shared by desktop capabilities; no screen dependencies.
#[cfg(target_os = "macos")]
pub mod assets;
#[cfg(target_os = "macos")]
pub mod clipboard;
#[cfg(target_os = "macos")]
mod common;
#[cfg(target_os = "macos")]
pub mod file_picker;
mod form_error;
#[cfg(target_os = "macos")]
pub mod forms;
#[cfg(target_os = "macos")]
pub mod icons;
#[cfg(target_os = "macos")]
pub mod style;
pub use form_error::{FormError, require_name};
rust_i18n::i18n!("../../locales", fallback = "en");
/// Explicit profile selected by the application shell.
#[cfg(target_os = "macos")]
#[non_exhaustive]
pub struct LaunchProfile(pub Option<std::path::PathBuf>);
#[cfg(target_os = "macos")]
impl gpui_kit::Global for LaunchProfile {}
#[cfg(target_os = "macos")]
impl LaunchProfile {
    /// Select a profile override; None uses the native adapter default.
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self(path)
    }
}
/// Isolated launch paths used only by synthetic UI scenarios.
#[cfg(all(target_os = "macos", feature = "ui-test-support"))]
#[non_exhaustive]
pub struct TestLaunch {
    /// Preferences file owned by the scenario.
    pub preferences: std::path::PathBuf,
    /// Explicit worker executable.
    pub worker: std::path::PathBuf,
}
#[cfg(all(target_os = "macos", feature = "ui-test-support"))]
impl gpui_kit::Global for TestLaunch {}

#[cfg(target_os = "macos")]
pub mod theme;

#[cfg(all(target_os = "macos", feature = "ui-test-support"))]
impl TestLaunch {
    /// Select isolated fixture files without accessing the user profile.
    pub fn new(preferences: std::path::PathBuf, worker: std::path::PathBuf) -> Self {
        Self {
            preferences,
            worker,
        }
    }
}
