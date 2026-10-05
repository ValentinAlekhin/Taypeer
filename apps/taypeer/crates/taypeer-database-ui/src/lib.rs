//! Database capability: sessions, browsing, editing, and their desktop views.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
use taypeer_runtime_client as backend;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use taypeer_settings_ui::local_settings;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod actions;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod ui;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux")),
    allow(
        dead_code,
        reason = "UI state is exercised by portable tests; rendering requires a desktop target"
    )
)]
mod ui_state;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use ui::*;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub use ui_state::{Destination, Route};
