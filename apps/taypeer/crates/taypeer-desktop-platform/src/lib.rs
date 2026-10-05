//! Desktop capabilities. Native protocols remain private to the OS adapter.
//!
//! Native macOS and Linux adapters own lifecycle and protected clipboard resources.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Platform, activity, helper_path};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Platform, activity, helper_path};

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
    #[cfg(target_os = "linux")]
    {
        let data = match std::env::var_os("XDG_DATA_HOME") {
            Some(path) if std::path::Path::new(&path).is_absolute() => {
                std::path::PathBuf::from(path)
            }
            _ => std::path::PathBuf::from(std::env::var_os("HOME").ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "home unavailable")
            })?)
            .join(".local/share"),
        };
        Ok(data.join("taypeer/profiles/default"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
