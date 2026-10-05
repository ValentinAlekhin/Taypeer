//! Session policy above native lifecycle and clipboard protocols.

use crate::Event;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use taypeer_runtime::session::{LockReason, SessionController};
use zeroize::Zeroizing;

/// The desktop owns the receiver, preferences and synthetic clipboard. Native
/// workers retain only the source needed to revoke access and publish events.
pub(crate) struct PlatformState {
    source: Arc<EventSource>,
    events: Mutex<mpsc::Receiver<Event>>,
    clipboard_seconds: Option<u32>,
    #[cfg(feature = "ui-test-support")]
    fixture: Option<Mutex<Zeroizing<String>>>,
}

pub(crate) struct EventSource {
    available: AtomicBool,
    failed: AtomicBool,
    transition: Mutex<()>,
    sessions: Mutex<Option<SessionController>>,
    send: mpsc::Sender<Event>,
}

impl EventSource {
    pub(crate) fn suspend(&self, reason: LockReason) {
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
        // A closed UI receiver cannot prevent background revocation.
        let _ = self.send.send(Event::Suspended(reason));
    }

    pub(crate) fn active(&self) {
        let Ok(_transition) = self.transition.lock() else {
            return;
        };
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        self.available.store(true, Ordering::Release);
        let _ = self.send.send(Event::Active);
    }

    pub(crate) fn available(&self) -> bool {
        !self.failed.load(Ordering::Acquire) && self.available.load(Ordering::Acquire)
    }

    pub(crate) fn clipboard(&self, success: bool, notify: bool) {
        let _ = self.send.send(Event::Clipboard { success, notify });
    }
}

impl PlatformState {
    pub(crate) fn new(available: bool) -> Self {
        let (send, events) = mpsc::channel();
        Self {
            source: Arc::new(EventSource {
                available: AtomicBool::new(available),
                failed: AtomicBool::new(false),
                transition: Mutex::new(()),
                sessions: Mutex::new(None),
                send,
            }),
            events: Mutex::new(events),
            clipboard_seconds: Some(30),
            #[cfg(feature = "ui-test-support")]
            fixture: None,
        }
    }

    pub(crate) fn source(&self) -> Arc<EventSource> {
        Arc::clone(&self.source)
    }

    pub(crate) fn attach(&self, controller: SessionController) {
        let Ok(_transition) = self.source.transition.lock() else {
            controller.lock_all(LockReason::SystemLocked);
            return;
        };
        if let Ok(mut sessions) = self.source.sessions.lock() {
            if !self.available() {
                controller.lock_all(LockReason::SystemLocked);
            }
            *sessions = Some(controller);
        } else {
            controller.lock_all(LockReason::SystemLocked);
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.source.available()
    }

    pub(crate) fn activity(&self) {
        if let Ok(sessions) = self.source.sessions.lock()
            && let Some(sessions) = sessions.as_ref()
        {
            sessions.activity().touch();
        }
    }

    pub(crate) fn set_clipboard_seconds(&mut self, seconds: Option<u32>) {
        self.clipboard_seconds = seconds;
    }

    pub(crate) fn clipboard_seconds(&self) -> Option<u32> {
        self.clipboard_seconds
    }

    /// A rejected or synthetic copy consumes the zeroizing buffer. Otherwise
    /// transfer it to the native adapter, which owns acknowledgement and clearing.
    pub(crate) fn prepare_copy(&self, text: String, notify: bool) -> Option<Zeroizing<String>> {
        let text = Zeroizing::new(text);
        if !self.available() {
            self.source.clipboard(false, notify);
            return None;
        }
        #[cfg(feature = "ui-test-support")]
        if let Some(fixture) = &self.fixture {
            *fixture.lock().expect("fixture clipboard") = text;
            self.source.clipboard(true, notify);
            return None;
        }
        Some(text)
    }

    pub(crate) fn drain(&self) -> Vec<Event> {
        self.events
            .lock()
            .map(|events| events.try_iter().collect())
            .unwrap_or_default()
    }

    #[cfg(feature = "ui-test-support")]
    pub(crate) fn fixture() -> Self {
        let mut state = Self::new(true);
        state.fixture = Some(Mutex::new(Zeroizing::new(String::new())));
        state
    }

    #[cfg(feature = "ui-test-support")]
    pub(crate) fn fixture_clipboard(&self) -> String {
        self.fixture
            .as_ref()
            .expect("fixture platform")
            .lock()
            .expect("fixture clipboard")
            .to_string()
    }

    #[cfg(feature = "ui-test-support")]
    pub(crate) fn fixture_active(&self) {
        self.source.active();
    }

    #[cfg(feature = "ui-test-support")]
    pub(crate) fn fixture_lock(&self) {
        self.source.suspend(LockReason::SystemLocked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taypeer_runtime::session::SessionPolicy;

    #[test]
    fn suspension_revokes_before_ui_drains_and_activity_does_not_reopen_access() {
        let state = PlatformState::new(true);
        let controller = SessionController::new(SessionPolicy::default());
        let activity = controller.activity();
        state.attach(controller);
        let before = activity.epoch();
        state.source.suspend(LockReason::Sleep);
        assert!(!state.available());
        assert!(activity.epoch() > before);
        assert_eq!(activity.reason(), Some(LockReason::Sleep));
        state.activity();
        assert!(!state.available());
        assert_eq!(activity.reason(), Some(LockReason::Sleep));
        assert!(matches!(
            state.drain().as_slice(),
            [Event::Suspended(LockReason::Sleep)]
        ));
    }

    #[test]
    fn unavailable_attachment_and_closed_ui_receiver_still_revoke() {
        let state = PlatformState::new(false);
        let controller = SessionController::new(SessionPolicy::default());
        let activity = controller.activity();
        state.attach(controller);
        assert_eq!(activity.reason(), Some(LockReason::SystemLocked));
        let source = state.source();
        // Keep the session owner alive while dropping only the desktop receiver.
        let controller = source.sessions.lock().unwrap().as_ref().unwrap().clone();
        drop(state);
        source.suspend(LockReason::HostExited);
        assert_eq!(controller.activity().reason(), Some(LockReason::HostExited));
        source.active();
        assert!(!source.available());
    }

    #[test]
    fn late_active_event_cannot_recover_a_lost_native_source() {
        let state = PlatformState::new(true);
        state.source.suspend(LockReason::HostExited);
        state.source.active();
        assert!(!state.available());
        assert!(matches!(
            state.drain().as_slice(),
            [Event::Suspended(LockReason::HostExited)]
        ));
        assert!(state.prepare_copy("PUBLIC copy".into(), false).is_none());
        assert!(matches!(
            state.drain().as_slice(),
            [Event::Clipboard {
                success: false,
                notify: false
            }]
        ));
    }

    #[cfg(feature = "ui-test-support")]
    #[test]
    fn fixture_preserves_clipboard_while_locked_and_reports_each_owned_write() {
        let mut state = PlatformState::fixture();
        state.set_clipboard_seconds(None);
        assert_eq!(state.clipboard_seconds(), None);
        assert!(state.prepare_copy("PUBLIC original".into(), true).is_none());
        assert_eq!(state.fixture_clipboard(), "PUBLIC original");
        assert!(matches!(
            state.drain().as_slice(),
            [Event::Clipboard {
                success: true,
                notify: true
            }]
        ));
        state.fixture_lock();
        state.prepare_copy("PUBLIC rejected".into(), false);
        assert_eq!(state.fixture_clipboard(), "PUBLIC original");
        assert!(matches!(
            state.drain().as_slice(),
            [
                Event::Suspended(LockReason::SystemLocked),
                Event::Clipboard {
                    success: false,
                    notify: false
                }
            ]
        ));
        state.fixture_active();
        state.prepare_copy("PUBLIC following".into(), false);
        assert_eq!(state.fixture_clipboard(), "PUBLIC following");
        assert!(matches!(
            state.drain().as_slice(),
            [
                Event::Active,
                Event::Clipboard {
                    success: true,
                    notify: false
                }
            ]
        ));
    }
}
