//! Private native helper for lifecycle notifications and owned pasteboard writes.
use gpui_kit::Global;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};
use zeroize::Zeroize;
pub(super) struct Platform {
    child: Option<Child>,
    #[cfg(feature = "ui-test-support")]
    clipboard: Option<Mutex<zeroize::Zeroizing<String>>>,
    next_copy: AtomicU64,
    pending_copies: Mutex<BTreeMap<u64, bool>>,
    send: mpsc::SyncSender<zeroize::Zeroizing<Vec<u8>>>,
    event_sender: mpsc::Sender<String>,
    sessions: Arc<Mutex<Option<taypeer_runtime::session::SessionController>>>,
    available: Arc<AtomicBool>,
    events: Mutex<mpsc::Receiver<String>>,
    pub clipboard_seconds: Option<u32>,
}
impl Global for Platform {}
impl Platform {
    #[cfg(feature = "ui-test-support")]
    pub fn fixture() -> Self {
        let (send, _) = mpsc::sync_channel(8);
        let (event_sender, events) = mpsc::channel();
        Self {
            child: None,
            clipboard: Some(Mutex::new(zeroize::Zeroizing::new(String::new()))),
            next_copy: AtomicU64::new(1),
            pending_copies: Mutex::new(BTreeMap::new()),
            send,
            event_sender,
            sessions: Arc::new(Mutex::new(None)),
            available: Arc::new(AtomicBool::new(true)),
            events: Mutex::new(events),
            clipboard_seconds: Some(30),
        }
    }
    #[cfg(feature = "ui-test-support")]
    pub fn fixture_clipboard(&self) -> String {
        self.clipboard
            .as_ref()
            .expect("fixture platform")
            .lock()
            .expect("fixture clipboard")
            .to_string()
    }
    #[cfg(feature = "ui-test-support")]
    pub fn fixture_lock(&self) {
        if let Some(sessions) = self.sessions.lock().expect("fixture sessions").as_ref() {
            sessions.lock_all(taypeer_runtime::session::LockReason::SystemLocked);
        }
        let _ = self.event_sender.send("locked".into());
    }
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
        let (events, receive) = mpsc::channel();
        let event_sender = events.clone();
        let sessions = Arc::new(Mutex::new(
            None::<taypeer_runtime::session::SessionController>,
        ));
        let active_sessions = sessions.clone();
        let available = Arc::new(AtomicBool::new(false));
        let is_available = available.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                match line.as_str() {
                    "sleep" | "locked" => {
                        is_available.store(false, Ordering::Release);
                        if let Ok(sessions) = active_sessions.lock()
                            && let Some(sessions) = sessions.as_ref()
                        {
                            sessions.lock_all(if line == "sleep" {
                                taypeer_runtime::session::LockReason::Sleep
                            } else {
                                taypeer_runtime::session::LockReason::SystemLocked
                            });
                        }
                    }
                    "ready" | "active" => is_available.store(true, Ordering::Release),
                    _ => {}
                }
                if events.send(line).is_err() {
                    break;
                }
            }
            is_available.store(false, Ordering::Release);
            if let Ok(sessions) = active_sessions.lock()
                && let Some(sessions) = sessions.as_ref()
            {
                sessions.lock_all(taypeer_runtime::session::LockReason::HostExited);
            }
            let _ = events.send("platform_closed".into());
        });
        Ok(Self {
            child: Some(child),
            #[cfg(feature = "ui-test-support")]
            clipboard: None,
            next_copy: AtomicU64::new(1),
            pending_copies: Mutex::new(BTreeMap::new()),
            event_sender,
            sessions,
            available,
            send,
            events: Mutex::new(receive),
            clipboard_seconds: Some(30),
        })
    }
    pub fn attach(&self, controller: taypeer_runtime::session::SessionController) {
        if let Ok(mut sessions) = self.sessions.lock() {
            *sessions = Some(controller);
        }
    }
    pub fn available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }
    pub fn copy(&self, mut text: String, secret: bool, notify: bool) {
        #[derive(serde::Serialize)]
        struct Copy<'a> {
            id: u64,
            text: &'a str,
            secret: bool,
            seconds: u32,
        }
        let id = self.next_copy.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut pending) = self.pending_copies.lock() {
            pending.insert(id, notify);
        }
        #[cfg(feature = "ui-test-support")]
        if let Some(clipboard) = &self.clipboard {
            *clipboard.lock().expect("fixture clipboard") = zeroize::Zeroizing::new(text);
            let _ = self.event_sender.send(format!("clipboard_ok:{id}"));
            return;
        }
        let encoded = serde_json::to_vec(&Copy {
            id,
            text: &text,
            secret,
            seconds: self.clipboard_seconds.unwrap_or(0),
        });
        text.zeroize();
        if encoded
            .map(zeroize::Zeroizing::new)
            .map_or(true, |bytes| self.send.try_send(bytes).is_err())
        {
            let _ = self.event_sender.send(format!("clipboard_error:{id}"));
        }
    }
    pub fn copy_result(&self, event: &str) -> Option<(bool, bool)> {
        let (kind, id) = event.split_once(':')?;
        let success = match kind {
            "clipboard_ok" => true,
            "clipboard_error" => false,
            _ => return None,
        };
        let id = id.parse::<u64>().ok()?;
        let notify = self.pending_copies.lock().ok()?.remove(&id)?;
        Some((success, notify))
    }
    pub fn drain(&self) -> Vec<String> {
        self.events
            .lock()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default()
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
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

pub(super) fn activity(cx: &gpui_kit::App) {
    if let Some(platform) = cx.try_global::<Platform>()
        && let Ok(sessions) = platform.sessions.lock()
        && let Some(sessions) = sessions.as_ref()
    {
        sessions.activity().touch();
    }
}
