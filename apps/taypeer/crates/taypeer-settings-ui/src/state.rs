//! One retained owner for device settings and their asynchronous persistence.
use crate::local_settings::LocalSettings;
use gpui_kit::*;
use std::{path::PathBuf, sync::Arc, time::Duration};
use taypeer_core::{DatabaseId, SessionPolicy};
use taypeer_runtime::session::SessionSettings;
use taypeer_runtime_client::{Backend, Ticket, background};
use taypeer_ui::{FormError, forms::Done};

enum Write {
    Settings(Ticket<LocalSettings>),
    Idle(Ticket<()>),
}

/// Device preferences shared by capabilities; this entity serializes writes.
pub struct DeviceSettingsState {
    values: LocalSettings,
    backend: Option<Arc<Backend>>,
    pending: Option<Write>,
    completion: Option<Done>,
    ready: bool,
    error: Option<&'static str>,
    recent: Vec<(DatabaseId, PathBuf)>,
    rejected: Vec<(Done, FormError)>,
    _poll: Task<()>,
}

impl DeviceSettingsState {
    /// Load nonsensitive preferences off the UI thread, using an explicit backend profile.
    pub fn new(backend: Option<Arc<Backend>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pending = backend.as_ref().map(|backend| {
            let profile = backend.profile.clone();
            Write::Settings(background(move || LocalSettings::load(&profile)))
        });
        let poll = cx.spawn_in(window, async move |state, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if state
                    .update_in(cx, |state, window, cx| state.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            values: LocalSettings::default(),
            backend,
            pending,
            completion: None,
            ready: false,
            error: None,
            recent: Vec::new(),
            rejected: Vec::new(),
            _poll: poll,
        }
    }

    /// Last successfully loaded or saved settings.
    pub fn values(&self) -> &LocalSettings {
        &self.values
    }
    /// Loading or an in-flight write prevents overlapping submissions.
    pub fn is_busy(&self) -> bool {
        !self.ready || self.pending.is_some()
    }
    /// Whether the device file has been read successfully.
    pub fn is_ready(&self) -> bool {
        self.ready
    }
    /// Localized failure category, never file contents.
    pub fn error(&self) -> Option<&'static str> {
        self.error
    }
    /// Applied inactivity policy in seconds.
    pub fn idle_seconds(&self) -> u32 {
        self.backend
            .as_ref()
            .map_or(300, |b| b.sessions.policy().idle_seconds())
    }

    /// Save one coherent candidate. The callback runs only after persistence completes.
    pub fn save(&mut self, values: LocalSettings, done: Option<Done>, cx: &mut Context<Self>) {
        if self.is_busy() {
            if let Some(done) = done {
                self.rejected.push((done, FormError::Backend));
            }
            return;
        }
        let Some(backend) = &self.backend else {
            return;
        };
        let profile = backend.profile.clone();
        self.pending = Some(Write::Settings(background(move || {
            values.save(&profile)?;
            Ok(values)
        })));
        self.completion = done;
        cx.notify();
    }

    /// Change inactivity only after the new policy is persisted.
    pub fn set_idle(&mut self, seconds: u32, done: Option<Done>, cx: &mut Context<Self>) {
        let Some(policy) = SessionPolicy::new(seconds).filter(|_| !self.is_busy()) else {
            if let Some(done) = done {
                self.rejected.push((done, FormError::InvalidNumber));
            }
            return;
        };
        let Some(backend) = &self.backend else {
            return;
        };
        let profile = backend.profile.clone();
        let sessions = backend.sessions.clone();
        self.pending = Some(Write::Idle(background(move || {
            SessionSettings::save(&profile, policy)?;
            sessions.set_policy(policy);
            Ok(())
        })));
        self.completion = done;
        cx.notify();
    }

    /// Adopt a relay workflow's persistence result under the same write gate.
    pub fn accept_relay(
        &mut self,
        ticket: Ticket<LocalSettings>,
        done: Option<Done>,
        cx: &mut Context<Self>,
    ) {
        debug_assert!(!self.is_busy());
        self.pending = Some(Write::Settings(ticket));
        self.completion = done;
        cx.notify();
    }

    /// Queue a recent file without dropping it when another settings write is pending.
    pub fn remember(&mut self, id: DatabaseId, path: PathBuf, cx: &mut Context<Self>) {
        self.recent.push((id, path));
        cx.notify();
    }

    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (done, error) in std::mem::take(&mut self.rejected) {
            done(Err(error), window, cx);
        }
        let result = self.pending.as_ref().and_then(|pending| match pending {
            Write::Settings(ticket) => ticket.try_take().map(|result| result.map(Some)),
            Write::Idle(ticket) => ticket.try_take().map(|result| result.map(|()| None)),
        });
        if let Some(result) = result {
            self.pending = None;
            let outcome = match result {
                Ok(values) => {
                    if let Some(values) = values {
                        self.values = values;
                    }
                    self.ready = true;
                    self.error = None;
                    Ok(())
                }
                Err(error) => {
                    self.error = Some("ui.settings_not_saved");
                    Err(FormError::Runtime(error))
                }
            };
            if let Some(done) = self.completion.take() {
                done(outcome, window, cx);
            }
            cx.notify();
        }
        if !self.is_busy() && !self.recent.is_empty() {
            let mut candidate = self.values.clone();
            for (id, path) in self.recent.drain(..) {
                candidate.remember(id, path);
            }
            self.save(candidate, None, cx);
        }
    }
}
