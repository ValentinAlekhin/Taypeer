//! Product presentation shared by desktop capabilities; no screen dependencies.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod assets;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod clipboard;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod common;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod file_picker;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod fonts;
mod form_error;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod forms;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod icons;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod style;
pub use form_error::{FormError, require_name};
rust_i18n::i18n!("../../locales", fallback = "en");
/// Explicit profile selected by the application shell.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[non_exhaustive]
pub struct LaunchProfile(pub Option<std::path::PathBuf>);
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl gpui_kit::Global for LaunchProfile {}
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl LaunchProfile {
    /// Select a profile override; None uses the native adapter default.
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self(path)
    }
}
/// Isolated launch paths used only by synthetic UI scenarios.
#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    feature = "ui-test-support"
))]
#[non_exhaustive]
pub struct TestLaunch {
    /// Preferences file owned by the scenario.
    pub preferences: std::path::PathBuf,
    /// Explicit worker executable.
    pub worker: std::path::PathBuf,
}
#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    feature = "ui-test-support"
))]
impl gpui_kit::Global for TestLaunch {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod theme;

#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    feature = "ui-test-support"
))]
impl TestLaunch {
    /// Select isolated fixture files without accessing the user profile.
    pub fn new(preferences: std::path::PathBuf, worker: std::path::PathBuf) -> Self {
        Self {
            preferences,
            worker,
        }
    }
}
