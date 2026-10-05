//! Private native helper for lifecycle notifications and owned pasteboard writes.
use crate::{
    Event,
    state::{EventSource, PlatformState},
};
use gpui_kit::Global;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
use taypeer_runtime::session::LockReason;
use zeroize::Zeroize;
/// macOS lifecycle and protected clipboard adapter, retained as an application global.
pub struct Platform {
    child: Option<Child>,
    pub(super) state: PlatformState,
    next_copy: AtomicU64,
    pending_copies: Arc<Mutex<BTreeMap<u64, bool>>>,
    send: mpsc::SyncSender<zeroize::Zeroizing<Vec<u8>>>,
}

enum HelperEvent {
    Active,
    Suspended(LockReason),
    Clipboard { id: Option<u64>, success: bool },
}

impl HelperEvent {
    fn parse(message: &str) -> Option<Self> {
        match message {
            "ready" | "active" => Some(Self::Active),
            "sleep" => Some(Self::Suspended(LockReason::Sleep)),
            "locked" => Some(Self::Suspended(LockReason::SystemLocked)),
            "platform_closed" => Some(Self::Suspended(LockReason::HostExited)),
            "clipboard_error" => Some(Self::Clipboard {
                id: None,
                success: false,
            }),
            _ => {
                let (kind, id) = message.split_once(':')?;
                let success = match kind {
                    "clipboard_ok" => true,
                    "clipboard_error" => false,
                    _ => return None,
                };
                Some(Self::Clipboard {
                    id: Some(id.parse().ok()?),
                    success,
                })
            }
        }
    }

    fn publish(self, source: &EventSource, pending: &Mutex<BTreeMap<u64, bool>>) {
        match self {
            Self::Active => source.active(),
            Self::Suspended(reason) => source.suspend(reason),
            Self::Clipboard { id, success } => {
                let notify = match id {
                    Some(id) => pending
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.remove(&id)),
                    None => Some(true),
                };
                if let Some(notify) = notify {
                    source.clipboard(success, notify);
                }
            }
        }
    }
}

impl Global for Platform {}
impl Platform {
    #[cfg(feature = "ui-test-support")]
    /// Create an isolated synthetic clipboard and lifecycle source for UI scenarios.
    pub fn fixture() -> Self {
        let (send, _) = mpsc::sync_channel(8);
        Self {
            child: None,
            state: PlatformState::fixture(),
            next_copy: AtomicU64::new(1),
            pending_copies: Arc::new(Mutex::new(BTreeMap::new())),
            send,
        }
    }
    #[cfg(feature = "ui-test-support")]
    /// Read the synthetic clipboard; never access the real system pasteboard.
    pub fn fixture_clipboard(&self) -> String {
        self.state.fixture_clipboard()
    }
    #[cfg(feature = "ui-test-support")]
    /// Confirm the synthetic desktop has returned to an active, unlocked state.
    pub fn fixture_active(&self) {
        self.state.fixture_active();
    }
    #[cfg(feature = "ui-test-support")]
    /// Revoke fixture sessions and enqueue the corresponding lifecycle event.
    pub fn fixture_lock(&self) {
        self.state.fixture_lock();
    }
    /// Spawn the bundled native helper; report startup I/O failures to the shell.
    pub fn start() -> std::io::Result<Self> {
        let sibling = std::env::current_exe()?.with_file_name("taypeer-platform");
        let path = if sibling.is_file() {
            sibling
        } else {
            env!("TAYPEER_PLATFORM_HELPER").into()
        };
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("native helper input unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("native helper output unavailable"))?;
        let (send, commands) = mpsc::sync_channel::<zeroize::Zeroizing<Vec<u8>>>(8);
        std::thread::spawn(move || {
            while let Ok(mut bytes) = commands.recv() {
                let written = input
                    .write_all(&bytes)
                    .and_then(|()| input.write_all(b"\n"));
                bytes.zeroize();
                if written.is_err() {
                    break;
                }
            }
        });
        let state = PlatformState::new(false);
        let source = state.source();
        let pending_copies = Arc::new(Mutex::new(BTreeMap::new()));
        let pending = Arc::clone(&pending_copies);
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Some(event) = HelperEvent::parse(&line) {
                    event.publish(&source, &pending);
                }
            }
            source.suspend(LockReason::HostExited);
        });
        Ok(Self {
            child: Some(child),
            state,
            next_copy: AtomicU64::new(1),
            pending_copies,
            send,
        })
    }
    /// Set the timeout for future owned clipboard writes; None disables timed clearing.
    pub fn set_clipboard_seconds(&mut self, seconds: Option<u32>) {
        self.state.set_clipboard_seconds(seconds);
    }
    /// Attach the session controller so revocation does not wait for a UI frame.
    pub fn attach(&self, controller: taypeer_runtime::session::SessionController) {
        self.state.attach(controller);
    }
    /// Whether the native helper currently permits protected interaction.
    pub fn available(&self) -> bool {
        self.state.available()
    }
    /// Queue an owned clipboard write and erase the supplied buffer after encoding.
    pub fn copy(&self, text: String, secret: bool, notify: bool) {
        #[derive(serde::Serialize)]
        struct Copy<'a> {
            id: u64,
            text: &'a str,
            secret: bool,
            seconds: u32,
        }
        let Some(mut text) = self.state.prepare_copy(text, notify) else {
            return;
        };
        let id = self.next_copy.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut pending) = self.pending_copies.lock() {
            pending.insert(id, notify);
        }
        let encoded = serde_json::to_vec(&Copy {
            id,
            text: &text,
            secret,
            seconds: self.state.clipboard_seconds().unwrap_or(0),
        });
        text.zeroize();
        if encoded
            .map(zeroize::Zeroizing::new)
            .map_or(true, |bytes| self.send.try_send(bytes).is_err())
        {
            HelperEvent::Clipboard {
                id: Some(id),
                success: false,
            }
            .publish(&self.state.source(), &self.pending_copies);
        }
    }
    /// Drain typed lifecycle and clipboard completions without exposing helper messages.
    pub fn drain(&self) -> Vec<Event> {
        self.state.drain()
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        self.state.source().suspend(LockReason::HostExited);
        if let Some(mut child) = self.child.take() {
            // Sender drop closes stdin. The helper clears its own timed secret before exiting.
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                loop {
                    if child.try_wait().ok().flatten().is_some() {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            });
        }
    }
}

/// Build-time path used by local packaging to copy the native helper.
pub fn helper_path() -> &'static str {
    env!("TAYPEER_PLATFORM_HELPER")
}

#[cfg(test)]
mod tests {
    use super::*;
    use taypeer_runtime::session::{SessionController, SessionPolicy};

    #[test]
    fn typed_helper_event_revokes_without_waiting_for_drain() {
        let state = PlatformState::new(true);
        let controller = SessionController::new(SessionPolicy::default());
        let activity = controller.activity();
        state.attach(controller);
        let pending = Mutex::new(BTreeMap::new());
        HelperEvent::parse("sleep")
            .unwrap()
            .publish(&state.source(), &pending);
        assert_eq!(activity.reason(), Some(LockReason::Sleep));
        assert!(!state.available());
        assert!(matches!(
            state.drain().as_slice(),
            [Event::Suspended(LockReason::Sleep)]
        ));
        HelperEvent::parse("platform_closed")
            .unwrap()
            .publish(&state.source(), &pending);
        HelperEvent::parse("active")
            .unwrap()
            .publish(&state.source(), &pending);
        assert!(!state.available());
    }

    #[test]
    fn clipboard_completions_preserve_notify_and_only_consume_an_owned_id_once() {
        let state = PlatformState::new(true);
        let pending = Mutex::new(BTreeMap::from([(7, false), (8, true)]));
        for message in [
            "clipboard_ok:7",
            "clipboard_ok:7",
            "clipboard_error:8",
            "clipboard_ok:999",
        ] {
            HelperEvent::parse(message)
                .unwrap()
                .publish(&state.source(), &pending);
        }
        assert!(matches!(
            state.drain().as_slice(),
            [
                Event::Clipboard {
                    success: true,
                    notify: false
                },
                Event::Clipboard {
                    success: false,
                    notify: true
                },
            ]
        ));
        for malformed in [
            "clipboard_ok",
            "clipboard_error:invalid",
            "clipboard_ok:7:8",
            "unknown:7",
        ] {
            assert!(HelperEvent::parse(malformed).is_none());
        }
    }
}
