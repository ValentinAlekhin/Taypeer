//! Device settings, persisted preferences, and their desktop presentation.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod appearance;
pub mod local_settings;
/// Persisted appearance preferences and supported values.
pub mod preferences;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod host;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod view;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use view::SettingsView;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod state;
