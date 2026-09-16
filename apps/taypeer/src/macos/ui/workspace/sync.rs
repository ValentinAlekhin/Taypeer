//! Bridge between window navigation and the independent network presentation owner.
use super::*;
use crate::local_settings::RelayPreference;
use taypeer_trust::{DeviceId, Digest};
impl WorkspaceStore {
    pub fn application_status(&self) -> Option<crate::backend::ApplicationStatus> {
        self.connection().and_then(Connection::application)
    }
    pub fn sync(&self) -> &SyncStore {
        &self.sync
    }
    pub fn start_sync(&mut self) {
        if self.local_ready
            && let Some(backend) = &self.backend
        {
            self.sync.start(backend, self.local.relay.clone());
        }
    }
    pub fn sync_now(&mut self, peer: Option<DeviceId>, cx: &mut Context<Self>) {
        if let (Some(backend), Some(database)) = (&self.backend, self.state.database.clone()) {
            self.sync
                .exchange(backend, database, peer, self.local.relay.clone());
            cx.notify();
        }
    }
    pub fn share_database(&mut self, cx: &mut Context<Self>) {
        if let (Some(backend), Some(connection)) = (&self.backend, self.connection().cloned()) {
            self.sync
                .share(backend, connection, self.local.relay.clone());
            cx.notify();
        }
    }
    pub fn invitation_action(
        &mut self,
        request: Digest,
        action: InvitationAction,
        cx: &mut Context<Self>,
    ) {
        if let (Some(backend), Some(connection)) = (&self.backend, self.connection().cloned()) {
            self.sync
                .invitation_action(backend, connection, request, action);
            cx.notify();
        }
    }
    pub fn pause_sync_task(&mut self, cx: &mut Context<Self>) {
        self.sync.pause();
        cx.notify();
    }
    pub fn join_database(
        &mut self,
        code: taypeer_runtime::InvitationCode,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if let Some(backend) = &self.backend {
            self.sync
                .join(backend, code, path, self.local.relay.clone());
            cx.notify();
        }
    }
    pub fn resume_join(&mut self, request: Digest, cx: &mut Context<Self>) {
        self.sync.resume(request);
        cx.notify();
    }
    pub fn set_relay(
        &mut self,
        relay: RelayPreference,
        done: Option<super::super::forms::Done>,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy() {
            return;
        }
        let Some(backend) = &self.backend else {
            return;
        };
        let mut settings = self.local.clone();
        settings.relay = relay;
        self.local_busy = true;
        cx.notify();
        self.watch(
            backend.configure_relay(settings),
            move |store, result, window, cx| {
                store.local_busy = false;
                let outcome = match result {
                    Ok(settings) => {
                        store.local = settings;
                        Ok(())
                    }
                    Err(error) => {
                        store.notice = Some("ui.settings_not_saved");
                        Err(crate::ui_state::FormError::Runtime(error))
                    }
                };
                if let Some(done) = done {
                    done(outcome, window, cx);
                }
                cx.notify();
            },
        );
    }
}
