//! Private native helper for lifecycle notifications and owned pasteboard writes.
use gpui_kit::Global;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use zeroize::Zeroize;
pub(super) struct Platform {
    child: Option<Child>,
    send: mpsc::SyncSender<zeroize::Zeroizing<Vec<u8>>>,
    event_sender: mpsc::Sender<String>,
    sessions: Arc<Mutex<Option<taypeer_runtime::session::SessionController>>>,
    available: Arc<AtomicBool>,
    events: Mutex<mpsc::Receiver<String>>,
    pub clipboard_seconds: Option<u32>,
}
impl Global for Platform {}
impl Platform {
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
    pub fn copy(&self, mut text: String, secret: bool) {
        #[derive(serde::Serialize)]
        struct Copy<'a> {
            text: &'a str,
            secret: bool,
            seconds: u32,
        }
        let encoded = serde_json::to_vec(&Copy {
            text: &text,
            secret,
            seconds: self.clipboard_seconds.unwrap_or(0),
        });
        text.zeroize();
        if encoded
            .map(zeroize::Zeroizing::new)
            .map_or(true, |bytes| self.send.try_send(bytes).is_err())
        {
            let _ = self.event_sender.send("clipboard_error".into());
        }
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
