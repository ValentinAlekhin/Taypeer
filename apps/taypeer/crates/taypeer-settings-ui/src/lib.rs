//! Device settings, persisted preferences, and their desktop presentation.
#[cfg(target_os = "macos")]
pub mod appearance;
pub mod local_settings;
/// Persisted appearance preferences and supported values.
pub mod preferences;

#[cfg(target_os = "macos")]
pub mod host;
#[cfg(target_os = "macos")]
mod view;
#[cfg(target_os = "macos")]
pub use view::SettingsView;

#[cfg(target_os = "macos")]
pub mod state;
