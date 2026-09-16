//! Database capability: sessions, browsing, editing, and their desktop views.
#[cfg(any(target_os = "macos", test))]
use taypeer_runtime_client as backend;
#[cfg(target_os = "macos")]
use taypeer_settings_ui::local_settings;
#[cfg(target_os = "macos")]
pub mod actions;
#[cfg(target_os = "macos")]
mod ui;
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
pub use ui::*;
#[cfg(any(target_os = "macos", test))]
pub use ui_state::{Destination, Route};
