//! Network presentation has an independent lifetime from plaintext database sessions.
use crate::{
    backend::{Backend, Connection, NetworkTicket, error_key},
    local_settings::RelayPreference,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
use taypeer_core::DatabaseId;
use taypeer_runtime::{
    Command, InvitationCode, JoinProgress, NetworkSnapshot, PendingJoin, RuntimeError,
    WorkerControl,
};
use taypeer_trust::{DeviceId, Digest, Invitation};
use zeroize::Zeroizing;

pub(crate) struct IssuedInvitation {
    pub database: DatabaseId,
    pub id: Digest,
    pub expires: u64,
    pub code: Zeroizing<String>,
    pub qr: Option<(usize, Zeroizing<Vec<u8>>)>,
    control: WorkerControl,
}
struct Overview {
    network: NetworkSnapshot,
    joins: BTreeMap<Digest, PendingJoin>,
}
enum Outcome {
    Started,
    Shared(IssuedInvitation),
    Joined {
        path: PathBuf,
        progress: JoinProgress,
    },
    Exchanged,
    InvitationChanged,
}
pub(crate) struct SyncStore {
    pub snapshot: NetworkSnapshot,
    pub joins: BTreeMap<Digest, PendingJoin>,
    pub invitation: Option<IssuedInvitation>,
    pub error: Option<&'static str>,
    pub status: &'static str,
    active: bool,
    operation: Option<NetworkTicket<Outcome>>,
    refresh: Option<NetworkTicket<Overview>>,
    refresh_at: Instant,
    continuation: Option<(Digest, PathBuf)>,
    resume_at: Instant,
    received: Option<PathBuf>,
}
impl Default for SyncStore {
    fn default() -> Self {
        Self {
            snapshot: Default::default(),
            joins: BTreeMap::new(),
            invitation: None,
            error: None,
            status: "sync.stopped",
            active: false,
            operation: None,
            refresh: None,
            refresh_at: Instant::now(),
            continuation: None,
            resume_at: Instant::now(),
            received: None,
        }
    }
}
impl SyncStore {
    pub fn database_status(&self, database: Option<&DatabaseId>) -> &'static str {
        if !self.snapshot.running {
            return "sync.stopped";
        }
        let Some(info) = self
            .snapshot
            .databases
            .iter()
            .find(|d| Some(&d.database) == database)
        else {
            return "sync.waiting_peer";
        };
        if info
            .invitations
            .iter()
            .any(|(_, s)| matches!(s, taypeer_trust::InvitationStatus::Requested(_)))
        {
            return "sync.requests";
        }
        if info
            .devices
            .iter()
            .any(|d| d.progress.as_ref().is_some_and(|p| p.result.is_err()))
        {
            return "sync.network_error";
        }
        "sync.automatic"
    }
    pub fn busy(&self) -> bool {
        self.operation.is_some()
    }
    pub fn waiting(&self) -> bool {
        self.continuation.is_some()
    }
    pub fn start(&mut self, backend: &Backend, relay: RelayPreference) {
        if self.busy() || self.snapshot.running {
            return;
        }
        self.active = true;
        self.error = None;
        self.status = "sync.connecting";
        self.operation = Some(backend.network(move |host, _| {
            host.start_network(relay.setting()?)?;
            Ok(Outcome::Started)
        }));
    }
    pub fn share(&mut self, backend: &Backend, connection: Connection, relay: RelayPreference) {
        if self.busy() {
            return;
        }
        self.active = true;
        self.error = None;
        self.invitation = None;
        self.status = "sync.creating_invitation";
        self.operation = Some(backend.network(move |host, _| {
            let address = host.start_network(relay.setting()?)?;
            let (invitation, secret): (Invitation, Zeroizing<[u8; 32]>) =
                connection.command(Command::CreateInvitation).wait()?;
            let id = invitation.id().map_err(|_| RuntimeError::Protocol)?;
            let expires = invitation.expires_at;
            let code = InvitationCode {
                invitation,
                secret,
                address,
            };
            let code =
                Zeroizing::new(serde_json::to_string(&code).map_err(|_| RuntimeError::Protocol)?);
            let qr = qrcode::QrCode::new(code.as_bytes()).ok().map(|qr| {
                (
                    qr.width(),
                    Zeroizing::new(
                        qr.to_colors()
                            .into_iter()
                            .map(|color| u8::from(color == qrcode::Color::Dark))
                            .collect(),
                    ),
                )
            });
            Ok(Outcome::Shared(IssuedInvitation {
                database: connection.database,
                id,
                expires,
                code,
                qr,
                control: connection.control,
            }))
        }));
    }
    pub fn invitation_action(
        &mut self,
        backend: &Backend,
        connection: Connection,
        request: Digest,
        action: InvitationAction,
    ) {
        if self.busy() {
            return;
        }
        self.error = None;
        self.operation = Some(backend.network(move |_, _| {
            match action {
                InvitationAction::Approve => {
                    connection
                        .command::<DeviceId>(Command::ApproveInvitation(request))
                        .wait()?;
                }
                InvitationAction::Reject | InvitationAction::Cancel => {
                    connection
                        .command::<()>(Command::CloseInvitation {
                            request,
                            reject: action == InvitationAction::Reject,
                        })
                        .wait()?;
                }
            }
            Ok(Outcome::InvitationChanged)
        }));
    }
    pub fn exchange(
        &mut self,
        backend: &Backend,
        database: DatabaseId,
        device: Option<DeviceId>,
        relay: RelayPreference,
    ) {
        if self.busy() {
            return;
        }
        self.active = true;
        self.error = None;
        self.status = "sync.exchanging";
        self.operation = Some(backend.network(move |host, cancellation| {
            host.start_network(relay.setting()?)?;
            let reports = host.exchange_database(&database, device, cancellation)?;
            if reports.iter().any(|r| r.result.is_err()) {
                return Err(RuntimeError::Transport);
            }
            Ok(Outcome::Exchanged)
        }));
    }
    pub fn join(
        &mut self,
        backend: &Backend,
        code: InvitationCode,
        path: PathBuf,
        relay: RelayPreference,
    ) {
        if self.busy() {
            return;
        }
        self.active = true;
        self.error = None;
        self.status = "sync.connecting";
        let executable = backend.executable.clone();
        self.operation = Some(backend.network(move |host, cancellation| {
            host.start_network(relay.setting()?)?;

            let progress = host.join_cancellable(&executable, code, &path, cancellation)?;
            Ok(Outcome::Joined { path, progress })
        }));
    }
    pub fn resume(&mut self, request: Digest) {
        if self.busy() {
            return;
        }
        if let Some(pending) = self.joins.get(&request) {
            self.continuation = Some((request, pending.path.clone()));
            self.resume_at = Instant::now();
            self.error = None;
        }
    }
    pub fn pause(&mut self) {
        self.operation = None;
        self.continuation = None;
        self.invitation = None;
        self.status = "sync.paused";
        self.refresh_at = Instant::now();
    }
    pub fn take_received(&mut self) -> Option<PathBuf> {
        self.received.take()
    }
    pub fn poll(&mut self, backend: &Backend, relay: &RelayPreference) -> bool {
        let mut changed = false;
        if self
            .invitation
            .as_ref()
            .is_some_and(|i| !i.control.is_open() || i.expires <= unix_seconds())
        {
            self.invitation = None;
            changed = true;
        }
        if let Some(result) = self.operation.as_ref().and_then(NetworkTicket::try_take) {
            self.operation = None;
            changed = true;
            self.refresh_at = Instant::now();
            match result {
                Ok(Outcome::Started) => self.status = "sync.waiting_peer",
                Ok(Outcome::Shared(invitation)) => {
                    if invitation.control.is_open() {
                        self.invitation = Some(invitation);
                    }
                    self.status = "sync.waiting_request";
                }
                Ok(Outcome::Joined { path, progress }) => match progress {
                    JoinProgress::Pending(request) => {
                        self.status = "sync.waiting_approval";
                        self.continuation = Some((request, path));
                        self.resume_at = Instant::now() + Duration::from_secs(2);
                    }
                    JoinProgress::Received(_) => {
                        self.continuation = None;
                        self.status = "sync.received_locked";
                        self.received = Some(path);
                    }
                    JoinProgress::Rejected => {
                        self.continuation = None;
                        self.error = Some("sync.join_rejected");
                        self.status = "sync.failed";
                    }
                },
                Ok(Outcome::Exchanged) => self.status = "sync.exchange_finished",
                Ok(Outcome::InvitationChanged) => {
                    self.invitation = None;
                    self.status = "sync.request_updated";
                }
                Err(error) => {
                    self.continuation = None;
                    self.error = Some(sync_error_key(&error));
                    self.status = "sync.failed";
                }
            }
        }
        if let Some(result) = self.refresh.as_ref().and_then(NetworkTicket::try_take) {
            self.refresh = None;
            self.refresh_at = Instant::now() + Duration::from_secs(1);
            match result {
                Ok(overview) => {
                    self.snapshot = overview.network;
                    self.joins = overview.joins;
                }
                Err(error) => {
                    self.error = Some(sync_error_key(&error));
                }
            }
            changed = true;
        }
        if self.active && self.refresh.is_none() && Instant::now() >= self.refresh_at {
            self.refresh = Some(backend.network(|host, _| {
                Ok(Overview {
                    network: host.network_snapshot()?,
                    joins: host.pending_joins()?,
                })
            }));
        }
        if self.operation.is_none()
            && Instant::now() >= self.resume_at
            && let Some((request, path)) = self.continuation.clone()
        {
            let relay = relay.clone();
            self.status = "sync.downloading";
            self.operation = Some(backend.network(move |host, cancellation| {
                host.start_network(relay.setting()?)?;
                Ok(Outcome::Joined {
                    progress: host.resume_join_cancellable(request, cancellation)?,
                    path,
                })
            }));
            changed = true;
        }
        changed
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvitationAction {
    Approve,
    Reject,
    Cancel,
}
pub(crate) fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn parse_invitation(text: &str) -> Result<InvitationCode, RuntimeError> {
    if text.len() > 64 * 1024 {
        return Err(RuntimeError::TooLarge);
    }
    serde_json::from_str(text).map_err(|_| RuntimeError::Protocol)
}
fn sync_error_key(error: &RuntimeError) -> &'static str {
    match error {
        RuntimeError::Transport => "sync.network_error",
        RuntimeError::Service(taypeer_services::ServiceError::Trust(_)) => {
            "sync.invitation_unavailable"
        }
        RuntimeError::Profile(taypeer_runtime::profile::ProfileError::Busy) => "sync.profile_busy",
        _ => error_key(error),
    }
}
