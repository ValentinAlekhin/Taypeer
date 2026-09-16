//! Desktop capabilities. Native protocols remain private to the OS adapter.
//!
//! macOS is the only implemented adapter. Linux and Windows need native lifecycle
//! and protected clipboard implementations before enabling their GUI builds.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Platform, activity, helper_path};

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

/// Locate the device profile without leaking OS directory conventions into features.
pub fn profile_path() -> std::io::Result<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "home unavailable"))?;
        Ok(std::path::PathBuf::from(home).join("Library/Application Support/Taypeer"))
    }
    #[cfg(not(target_os = "macos"))]
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "desktop adapter not implemented",
    ))
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
