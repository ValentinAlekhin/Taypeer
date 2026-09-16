//! Public exchange commands; state and polling belong to SyncState.
use super::*;
use crate::local_settings::RelayPreference;
use taypeer_trust::{DeviceId, Digest};
impl WorkspaceStore {
    pub(crate) fn application_status(&self) -> Option<crate::backend::ApplicationStatus> {
        self.connection().and_then(Connection::application)
    }
    /// Read exchange presentation without exposing mutable state.
    pub fn sync<'a>(&self, cx: &'a App) -> &'a SyncStore {
        self.sync.read(cx).model()
    }
    pub(crate) fn start_sync(&mut self, cx: &mut Context<Self>) {
        self.sync.update(cx, |sync, cx| sync.start(cx));
    }
    pub(crate) fn sync_now(&mut self, peer: Option<DeviceId>, cx: &mut Context<Self>) {
        if let Some(database) = self.state.database.clone() {
            self.sync
                .update(cx, |sync, cx| sync.exchange(database, peer, cx));
        }
    }
    pub(crate) fn share_database(&mut self, cx: &mut Context<Self>) {
        if let Some(connection) = self.connection().cloned() {
            self.sync.update(cx, |sync, cx| sync.share(connection, cx));
        }
    }
    pub(crate) fn invitation_action(
        &mut self,
        request: Digest,
        action: InvitationAction,
        cx: &mut Context<Self>,
    ) {
        if let Some(connection) = self.connection().cloned() {
            self.sync.update(cx, |sync, cx| {
                sync.invitation_action(connection, request, action, cx)
            });
        }
    }
    pub(crate) fn pause_sync_task(&mut self, cx: &mut Context<Self>) {
        self.sync.update(cx, |sync, cx| sync.pause(cx));
    }
    pub(crate) fn join_database(
        &mut self,
        code: taypeer_runtime::InvitationCode,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.sync.update(cx, |sync, cx| sync.join(code, path, cx));
    }
    pub(crate) fn resume_join(&mut self, request: Digest, cx: &mut Context<Self>) {
        self.sync.update(cx, |sync, cx| sync.resume(request, cx));
    }
    pub(crate) fn set_relay(
        &mut self,
        relay: RelayPreference,
        done: Option<super::super::forms::Done>,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy(cx) || self.sync(cx).busy() {
            return;
        }
        let Some(backend) = &self.backend else {
            return;
        };
        let mut settings = self.local(cx).clone();
        settings.relay = relay;
        let ticket = taypeer_sync_ui::configure_relay(backend, settings);
        self.settings
            .update(cx, |settings, cx| settings.accept_relay(ticket, done, cx));
    }
}
