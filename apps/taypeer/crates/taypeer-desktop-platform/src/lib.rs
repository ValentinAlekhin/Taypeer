//! Desktop capabilities. Native protocols remain private to the OS adapter.
//!
//! Native macOS and Linux adapters own lifecycle and protected clipboard resources.
#[cfg(target_os = "macos")]
mod macos;
mod state;
#[cfg(target_os = "macos")]
pub use macos::{Platform, helper_path};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Platform, helper_path};

/// Typed events consumed by the session owner, never native protocol strings.
pub enum Event {
    /// The desktop is active and can accept protected interaction.
    Active,
    /// Access must be revoked immediately for this lifecycle transition.
    Suspended(taypeer_runtime::session::LockReason),
    /// Completion of an owned clipboard write, without clipboard contents.
    Clipboard {
        /// Whether the native adapter acknowledged the write.
        success: bool,
        /// Whether the caller requested a visible success notification.
        notify: bool,
    },
}

/// Record genuine user input against the attached desktop session controller.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn activity(cx: &gpui_kit::App) {
    if let Some(platform) = cx.try_global::<Platform>() {
        platform.state.activity();
    }
}

/// Locate the device profile without leaking OS directory conventions into features.
pub fn profile_path() -> std::io::Result<std::path::PathBuf> {
    taypeer_runtime::RuntimeHost::default_profile_path()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "profile unavailable"))
}

/// Resolve the platform primary command modifier; feature commands supply the chord.
pub fn primary_shortcut(chord: &str) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    format!("{modifier}-{chord}")
}
