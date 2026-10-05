//! Linux capabilities stay outside feature code and revoke sessions off the UI thread.
mod clipboard;
mod lifecycle;
use crate::Event;
use gpui_kit::Global;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use taypeer_runtime::session::{LockReason, SessionController};
use zeroize::Zeroizing;

/// Native Wayland clipboard and D-Bus lifecycle owner.
pub struct Platform {
    shared: Arc<Shared>,
    events: Mutex<mpsc::Receiver<Event>>,
    clipboard: mpsc::SyncSender<clipboard::Command>,
    lifecycle: Option<lifecycle::Monitor>,
    seconds: Option<u32>,
    #[cfg(feature = "ui-test-support")]
    fixture: Option<Mutex<Zeroizing<String>>>,
}
impl Global for Platform {}
struct Shared {
    available: AtomicBool,
    failed: AtomicBool,
    transition: Mutex<()>,
    sessions: Mutex<Option<SessionController>>,
    send: mpsc::Sender<Event>,
}
impl Shared {
    fn suspend(&self, reason: LockReason) {
        let Ok(_transition) = self.transition.lock() else {
            self.available.store(false, Ordering::Release);
            return;
        };
        if reason == LockReason::HostExited {
            self.failed.store(true, Ordering::Release);
        }
        self.available.store(false, Ordering::Release);
        if let Ok(sessions) = self.sessions.lock()
            && let Some(sessions) = sessions.as_ref()
        {
            sessions.lock_all(reason);
        }
        // A closed UI event receiver does not undo background revocation.
        let _ = self.send.send(Event::Suspended(reason));
    }
    fn active(&self) {
        let Ok(_transition) = self.transition.lock() else {
            return;
        };
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        self.available.store(true, Ordering::Release);
        let _ = self.send.send(Event::Active);
    }
}
impl Platform {
    /// Start native sources; unsupported Wayland capability fails rather than using an unprotected clipboard.
    pub fn start() -> std::io::Result<Self> {
        let (send, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            available: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            transition: Mutex::new(()),
            sessions: Mutex::new(None),
            send,
        });
        let clipboard = clipboard::start(Arc::clone(&shared))?;
        let lifecycle =
            lifecycle::Monitor::start(Arc::clone(&shared)).map_err(std::io::Error::other)?;
        Ok(Self {
            shared,
            events: Mutex::new(events),
            clipboard,
            lifecycle: Some(lifecycle),
            seconds: Some(30),
            #[cfg(feature = "ui-test-support")]
            fixture: None,
        })
    }
    /// Set the timeout for future secret copies. None disables timed clearing.
    pub fn set_clipboard_seconds(&mut self, seconds: Option<u32>) {
        self.seconds = seconds;
    }
    /// Attach the application session owner for immediate background revocation.
    pub fn attach(&self, controller: SessionController) {
        if let Ok(mut sessions) = self.shared.sessions.lock() {
            if !self.available() {
                controller.lock_all(LockReason::SystemLocked);
            }
            *sessions = Some(controller);
        }
    }
    /// Whether confirmed native activity permits a fresh authentication.
    pub fn available(&self) -> bool {
        !self.shared.failed.load(Ordering::Acquire) && self.shared.available.load(Ordering::Acquire)
    }
    /// Transfer clipboard contents into the native owner; completion follows compositor acknowledgement.
    pub fn copy(&self, text: String, secret: bool, notify: bool) {
        let text = Zeroizing::new(text);
        if !self.available() {
            let _ = self.shared.send.send(Event::Clipboard {
                success: false,
                notify,
            });
            return;
        }
        #[cfg(feature = "ui-test-support")]
        if let Some(fixture) = &self.fixture {
            *fixture.lock().expect("fixture clipboard") = text;
            let _ = self.shared.send.send(Event::Clipboard {
                success: true,
                notify,
            });
            return;
        }
        if self
            .clipboard
            .try_send(clipboard::Command {
                text,
                secret,
                notify,
                seconds: self.seconds,
            })
            .is_err()
        {
            let _ = self.shared.send.send(Event::Clipboard {
                success: false,
                notify,
            });
        }
    }
    /// Drain typed native events without exposing any clipboard content.
    pub fn drain(&self) -> Vec<Event> {
        self.events
            .lock()
            .map(|events| events.try_iter().collect())
            .unwrap_or_default()
    }
    #[cfg(feature = "ui-test-support")]
    /// Create isolated UI capabilities without connecting to D-Bus or Wayland.
    pub fn fixture() -> Self {
        let (send, events) = mpsc::channel();
        let (clipboard, _) = mpsc::sync_channel(8);
        Self {
            shared: Arc::new(Shared {
                available: AtomicBool::new(true),
                failed: AtomicBool::new(false),
                transition: Mutex::new(()),
                sessions: Mutex::new(None),
                send,
            }),
            events: Mutex::new(events),
            clipboard,
            lifecycle: None,
            seconds: Some(30),
            fixture: Some(Mutex::new(Zeroizing::new(String::new()))),
        }
    }
    #[cfg(feature = "ui-test-support")]
    /// Read only the synthetic clipboard owned by this scenario.
    pub fn fixture_clipboard(&self) -> String {
        self.fixture
            .as_ref()
            .expect("fixture platform")
            .lock()
            .expect("fixture clipboard")
            .to_string()
    }
    #[cfg(feature = "ui-test-support")]
    /// Confirm the synthetic desktop has returned to an active, unlocked state.
    pub fn fixture_active(&self) {
        self.shared.active();
    }
    #[cfg(feature = "ui-test-support")]
    /// Immediately revoke fixture sessions and publish the lifecycle event.
    pub fn fixture_lock(&self) {
        self.shared.suspend(LockReason::SystemLocked);
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        self.shared.suspend(LockReason::HostExited);
        self.lifecycle.take();
    }
}
/// Record real user activity without treating background work as input.
pub fn activity(cx: &gpui_kit::App) {
    if let Some(platform) = cx.try_global::<Platform>()
        && let Ok(sessions) = platform.shared.sessions.lock()
        && let Some(sessions) = sessions.as_ref()
    {
        sessions.activity().touch();
    }
}
/// Linux has an in-process native adapter and does not bundle an additional helper.
pub fn helper_path() -> &'static str {
    ""
}
