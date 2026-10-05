//! Linux capabilities stay outside feature code and revoke sessions off the UI thread.
mod clipboard;
mod lifecycle;
mod proxies;
use crate::{Event, state::PlatformState};
use gpui_kit::Global;
use std::sync::mpsc;
use taypeer_runtime::session::{LockReason, SessionController};

/// Native Wayland clipboard and D-Bus lifecycle owner.
pub struct Platform {
    pub(super) state: PlatformState,
    clipboard: mpsc::SyncSender<clipboard::Command>,
    lifecycle: Option<lifecycle::Monitor>,
}
impl Global for Platform {}
impl Platform {
    /// Start native sources; unsupported Wayland capability fails rather than using an unprotected clipboard.
    pub fn start() -> std::io::Result<Self> {
        let state = PlatformState::new(false);
        let clipboard = clipboard::start(state.source())?;
        let lifecycle = lifecycle::Monitor::start(state.source()).map_err(std::io::Error::other)?;
        Ok(Self {
            state,
            clipboard,
            lifecycle: Some(lifecycle),
        })
    }
    /// Set the timeout for future secret copies. None disables timed clearing.
    pub fn set_clipboard_seconds(&mut self, seconds: Option<u32>) {
        self.state.set_clipboard_seconds(seconds);
    }
    /// Attach the application session owner for immediate background revocation.
    pub fn attach(&self, controller: SessionController) {
        self.state.attach(controller);
    }
    /// Whether confirmed native activity permits a fresh authentication.
    pub fn available(&self) -> bool {
        self.state.available()
    }
    /// Transfer clipboard contents into the native owner; completion follows compositor acknowledgement.
    pub fn copy(&self, text: String, secret: bool, notify: bool) {
        let Some(text) = self.state.prepare_copy(text, notify) else {
            return;
        };
        if self
            .clipboard
            .try_send(clipboard::Command {
                text,
                secret,
                notify,
                seconds: self.state.clipboard_seconds(),
            })
            .is_err()
        {
            self.state.source().clipboard(false, notify);
        }
    }
    /// Drain typed native events without exposing any clipboard content.
    pub fn drain(&self) -> Vec<Event> {
        self.state.drain()
    }
    #[cfg(feature = "ui-test-support")]
    /// Create isolated UI capabilities without connecting to D-Bus or Wayland.
    pub fn fixture() -> Self {
        let (clipboard, _) = mpsc::sync_channel(8);
        Self {
            state: PlatformState::fixture(),
            clipboard,
            lifecycle: None,
        }
    }
    #[cfg(feature = "ui-test-support")]
    /// Read only the synthetic clipboard owned by this scenario.
    pub fn fixture_clipboard(&self) -> String {
        self.state.fixture_clipboard()
    }
    #[cfg(feature = "ui-test-support")]
    /// Confirm the synthetic desktop has returned to an active, unlocked state.
    pub fn fixture_active(&self) {
        self.state.fixture_active();
    }
    #[cfg(feature = "ui-test-support")]
    /// Immediately revoke fixture sessions and publish the lifecycle event.
    pub fn fixture_lock(&self) {
        self.state.fixture_lock();
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        self.state.source().suspend(LockReason::HostExited);
        self.lifecycle.take();
    }
}
/// Linux has an in-process native adapter and does not bundle an additional helper.
pub fn helper_path() -> &'static str {
    ""
}
