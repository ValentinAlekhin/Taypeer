//! Explicit commands and read-only context needed by exchange screens.
use crate::{InvitationAction, SyncStore};
use gpui_kit::*;
use taypeer_core::DatabaseId;
use taypeer_trust::{DeviceId, Digest};
/// A screen's composing owner; database internals never cross this seam.
pub trait SyncHost: Sized + 'static {
    /// Network presentation with its independent lifetime.
    fn sync<'a>(&self, cx: &'a App) -> &'a SyncStore;
    /// Selected database, including a locked database.
    fn selected_database(&self) -> Option<&DatabaseId>;
    /// Whether the selected database has an open session.
    fn is_unlocked(&self) -> bool;
    /// Whether the receive workflow is visible.
    fn is_receiving(&self) -> bool;
    /// Local nonsensitive device label.
    fn device_name<'a>(&self, cx: &'a App) -> &'a str;
    /// Revision invalidating secret-bearing input and stale dialogs.
    fn secret_epoch(&self) -> u64;
    /// Whether authenticated commands may write to the selected database.
    fn writable(&self, cx: &App) -> bool;
    /// Last local application result, independent of transport ACK.
    fn application_status(&self) -> Option<taypeer_runtime_client::ApplicationStatus>;
    /// Return to the database workflow through its pending-edit guard.
    fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>);
    /// Request exchange with one peer or all admitted peers.
    fn sync_now(&mut self, peer: Option<DeviceId>, cx: &mut Context<Self>);
    /// Request an invitation for the selected database.
    fn share_database(&mut self, cx: &mut Context<Self>);
    /// Approve, reject, or revoke a pending invitation.
    fn invitation_action(
        &mut self,
        request: Digest,
        action: InvitationAction,
        cx: &mut Context<Self>,
    );
    /// Cancel active presentation work and erase invitation material.
    fn pause_sync_task(&mut self, cx: &mut Context<Self>);
    /// Receive ciphertext to the explicitly selected new path.
    fn join_database(
        &mut self,
        code: taypeer_runtime::InvitationCode,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    );
    /// Resume receipt using a public request identifier.
    fn resume_join(&mut self, request: Digest, cx: &mut Context<Self>);
    /// Present a localized, content-free failure category.
    fn set_notice(&mut self, notice: &'static str, cx: &mut Context<Self>);
}
