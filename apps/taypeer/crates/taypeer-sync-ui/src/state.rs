//! Retained exchange owner, independent from the selected plaintext session.
use crate::{InvitationAction, SyncStore};
use gpui_kit::*;
use std::{path::PathBuf, sync::Arc, time::Duration};
use taypeer_core::DatabaseId;
use taypeer_runtime_client::{Backend, Connection};
use taypeer_settings_ui::state::DeviceSettingsState;
use taypeer_trust::{DeviceId, Digest};

/// A ciphertext working file has been durably received; it still needs unlocking.
#[non_exhaustive]
pub struct ReceivedDatabase(pub PathBuf);

/// Owns network polling and invitation material across database lock/unlock transitions.
pub struct SyncState {
    model: SyncStore,
    backend: Option<Arc<Backend>>,
    settings: Entity<DeviceSettingsState>,
    _poll: Task<()>,
}
impl EventEmitter<ReceivedDatabase> for SyncState {}
impl SyncState {
    /// Create the exchange owner without opening credentials or starting the network.
    pub fn new(
        backend: Option<Arc<Backend>>,
        settings: Entity<DeviceSettingsState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let poll = cx.spawn(async move |state, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if state.update(cx, |state, cx| state.poll(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            model: SyncStore::default(),
            backend,
            settings,
            _poll: poll,
        }
    }
    /// Read-only presentation owned by this capability.
    pub fn model(&self) -> &SyncStore {
        &self.model
    }
    /// Start automatic exchange after explicit database or admission intent.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.settings.read(cx).is_ready()
            && let Some(backend) = &self.backend
        {
            self.model
                .start(backend, self.settings.read(cx).values().relay.clone());
            cx.notify();
        }
    }
    /// Request exchange for an explicitly selected database.
    pub fn exchange(
        &mut self,
        database: DatabaseId,
        peer: Option<DeviceId>,
        cx: &mut Context<Self>,
    ) {
        if let Some(backend) = &self.backend {
            self.model.exchange(
                backend,
                database,
                peer,
                self.settings.read(cx).values().relay.clone(),
            );
            cx.notify();
        }
    }
    /// Create an invitation using an authenticated connection.
    pub fn share(&mut self, connection: Connection, cx: &mut Context<Self>) {
        if let Some(backend) = &self.backend {
            self.model.share(
                backend,
                connection,
                self.settings.read(cx).values().relay.clone(),
            );
            cx.notify();
        }
    }
    /// Apply the requested admission decision through the runtime.
    pub fn invitation_action(
        &mut self,
        connection: Connection,
        request: Digest,
        action: InvitationAction,
        cx: &mut Context<Self>,
    ) {
        if let Some(backend) = &self.backend {
            self.model
                .invitation_action(backend, connection, request, action);
            cx.notify();
        }
    }
    /// Release secret invitation UI and cancel its current presentation operation.
    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.model.pause();
        cx.notify();
    }
    /// Receive an invited database into a new file.
    pub fn join(
        &mut self,
        code: taypeer_runtime::InvitationCode,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if let Some(backend) = &self.backend {
            self.model.join(
                backend,
                code,
                path,
                self.settings.read(cx).values().relay.clone(),
            );
            cx.notify();
        }
    }
    /// Resume a persisted ciphertext receipt without retaining the invitation code.
    pub fn resume(&mut self, request: Digest, cx: &mut Context<Self>) {
        self.model.resume(request);
        cx.notify();
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        if let Some(backend) = &self.backend
            && self
                .model
                .poll(backend, &self.settings.read(cx).values().relay)
        {
            cx.notify();
        }
        if let Some(path) = self.model.take_received() {
            cx.emit(ReceivedDatabase(path));
        }
    }
}
