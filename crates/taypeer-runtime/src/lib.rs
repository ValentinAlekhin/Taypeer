//! Private command IPC and independent lifetimes for plaintext workers and ciphertext writers.
mod cipher_ipc;
mod host;
mod network;
pub mod platform;
mod process;
pub mod profile;
mod protocol;
pub mod session;
mod worker;
pub use host::{RegisteredCompatibility, RelocatedWorkingCopy, RuntimeHost, WorkingCopy};
pub use network::{
    DatabaseExchange, DeviceExchange, InvitationCode, JoinProgress, NetworkCancellation,
    NetworkSnapshot, PeerProgress, PendingJoin, PendingJoinSummary,
};
pub use protocol::{Command, RuntimeError, erase_view};
#[cfg(feature = "ui-test-support")]
pub use worker::run_test_worker;
pub use worker::run_worker;

use host::{Callbacks, HostContext};
use process::Client;
use protocol::Boot;
use serde_json::Value;
use session::{DraftDisposition, LockOutcome, LockReason, SessionController};
use std::time::Duration;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use taypeer_core::DatabaseId;
use taypeer_sync::{CoordinatorEvent, CoordinatorState};

#[cfg(all(test, feature = "ui-test-support"))]
mod automation_sync_tests;
use zeroize::Zeroize;

/// One supervised plaintext process. Access revocation never waits for command I/O.
pub struct Worker {
    client: Arc<Client>,
    database: DatabaseId,
    watch: tokio::task::JoinHandle<()>,
    apply_task: tokio::task::JoinHandle<()>,
    application: Arc<Mutex<Option<Result<Value, RuntimeError>>>>,
    application_revision: Arc<AtomicU64>,
    _sessions: SessionController,
}
impl Worker {
    pub(crate) fn enroll(
        executable: &Path,
        context: Arc<HostContext>,
        destination: &Path,
        password: String,
        invitation: taypeer_trust::Invitation,
        sessions: &SessionController,
    ) -> Result<(taypeer_trust::JoinProof, Arc<Client>), RuntimeError> {
        let spool = tempfile::tempdir().map_err(|_| RuntimeError::Transport)?;
        let mut boot = Boot {
            path: destination.to_owned(),
            password,
            create_name: None,
            create_form: None,
            profile: context.profile.directory().to_owned(),
            spool: spool.path().to_owned(),
            invitation: Some(invitation),
        };
        let callbacks = Callbacks::new(context, destination.to_owned(), spool.path().to_owned());
        let client = Client::spawn(executable, sessions, Box::new(callbacks), spool)?;
        let result = client
            .request(&boot, true)
            .and_then(|value| serde_json::from_value(value).map_err(|_| RuntimeError::Protocol));
        boot.password.zeroize();
        match result {
            Ok(proof) => {
                client.control.opened(
                    boot.invitation
                        .as_ref()
                        .ok_or(RuntimeError::Protocol)?
                        .database
                        .clone(),
                )?;
                Ok((proof, client))
            }
            Err(error) => {
                client.control.invalidate(LockReason::Transport);
                Err(error)
            }
        }
    }
    pub(crate) fn open(
        launcher: &dyn platform::ProcessLauncher,
        path: &Path,
        password: String,
        create_form: Option<taypeer_services::CreateDatabase>,
        context: Arc<HostContext>,
        runtime: &tokio::runtime::Handle,
        sessions: &SessionController,
    ) -> Result<Self, RuntimeError> {
        let spool = tempfile::tempdir().map_err(|_| RuntimeError::Transport)?;
        let directory = spool
            .path()
            .canonicalize()
            .map_err(|_| RuntimeError::Transport)?;
        let mut boot = Boot {
            path: path.to_owned(),
            password,
            create_name: None,
            create_form,
            profile: context.profile.directory().to_owned(),
            spool: directory.clone(),
            invitation: None,
        };
        let contexts = Arc::clone(&context);
        let client = Client::connect(
            launcher.launch()?,
            sessions,
            Box::new(Callbacks::new(context, path.to_owned(), directory)),
            spool,
        )?;
        let opened = client.request(&boot, true);
        boot.password.zeroize();
        let database = match opened.and_then(|value| {
            serde_json::from_value::<DatabaseId>(value).map_err(|_| RuntimeError::Protocol)
        }) {
            Ok(id) => id,
            Err(error) => {
                client.control.invalidate(LockReason::Transport);
                return Err(error);
            }
        };
        client.control.opened(database.clone())?;
        let watched_context = contexts.for_database(&database).inspect_err(|_| {
            client.control.invalidate(LockReason::Transport);
        })?;
        // Linux creates its per-database context during boot. Subscribe first,
        // then reconcile against the exact worker snapshot rather than treating
        // the broadcast stream as an authoritative inventory.
        let mut events = watched_context.coordinator.subscribe();
        let records: Vec<taypeer_trust::SignedControl> = serde_json::from_value(
            client
                .request(&Command::SessionAuthority, false)
                .inspect_err(|_| {
                    client.control.invalidate(LockReason::Transport);
                })?,
        )
        .map_err(|_| RuntimeError::Protocol)
        .inspect_err(|_| {
            client.control.invalidate(LockReason::Transport);
        })?;
        let pinned = watched_context
            .coordinator
            .snapshot(&database)
            .map_err(host::sync_error)
            .and_then(|snapshot| snapshot.chain().root().map_err(|_| RuntimeError::Protocol))
            .inspect_err(|_| client.control.invalidate(LockReason::Transport))?;
        let authority = taypeer_trust::ControlChain::validate(records, pinned)
            .map_err(|_| RuntimeError::Protocol)
            .inspect_err(|_| client.control.invalidate(LockReason::Transport))?;
        let mut epoch = authority.head().epoch;
        let admitted = authority
            .admit_transport(watched_context.transport.public())
            .is_ok();
        let initial = watched_context
            .coordinator
            .state(&database)
            .map_err(host::sync_error)
            .inspect_err(|_| client.control.invalidate(LockReason::Transport))?;
        if authority_changed(&initial, epoch, admitted) {
            client.control.invalidate(LockReason::Authority);
            return Err(RuntimeError::SessionClosed(LockReason::Authority));
        }
        let linux_credentials = contexts.profile.is_linux_lazy();
        let application = Arc::new(Mutex::new(None));
        let application_revision = Arc::new(AtomicU64::new(0));
        // Cover every durable delivery between the child's initial apply and
        // this subscription before exposing an unlocked Worker to the caller.
        let result = client.request(&Command::ApplyReceived, false);
        let retry = retry_application(&result);
        record_application(&client, result, &application, &application_revision);
        client.control.check(false)?;
        let (apply_send, mut apply_receive) = tokio::sync::mpsc::channel(1);
        let apply_client = Arc::downgrade(&client);
        let updates = Arc::clone(&application);
        let revision = Arc::clone(&application_revision);
        let apply_task = runtime.spawn(async move {
            let mut retry = retry;
            let mut backoff = Duration::from_millis(100);
            loop {
                if retry {
                    tokio::select! {
                        received = apply_receive.recv() => if received.is_none() { break; },
                        _ = tokio::time::sleep(backoff) => {},
                    }
                } else if apply_receive.recv().await.is_none() {
                    break;
                }
                let Some(client) = apply_client.upgrade() else {
                    break;
                };
                if client.control.check(false).is_err() {
                    break;
                }
                let updates = Arc::clone(&updates);
                let revision = Arc::clone(&revision);
                retry = tokio::task::spawn_blocking(move || {
                    let result = client.request(&Command::ApplyReceived, false);
                    let retry = retry_application(&result);
                    record_application(&client, result, &updates, &revision);
                    retry && client.control.check(false).is_ok()
                })
                .await
                .unwrap_or(false);
                backoff = if retry {
                    (backoff * 2).min(Duration::from_secs(3))
                } else {
                    Duration::from_millis(100)
                };
            }
        });
        let weak = Arc::downgrade(&client);
        let watched_database = database.clone();
        let watch = runtime.spawn(async move {
            let mut fingerprint = initial.fingerprint;
            let mut pending_rotation = None;
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let event = tokio::select! {
                    event = events.recv() => Some(event),
                    _ = interval.tick() => None,
                };
                let Some(client) = weak.upgrade() else {
                    break;
                };
                if client.control.check(false).is_err() {
                    break;
                }
                match event {
                    Some(Ok(CoordinatorEvent::Frozen(database)))
                        if database == watched_database =>
                    {
                        // A fork signal revokes immediately, even when its disk
                        // transaction is still running or cannot retain evidence.
                        client.control.invalidate(LockReason::Authority);
                        break;
                    }
                    Some(Ok(CoordinatorEvent::ControlChanged {
                        database,
                        lock: true,
                        ..
                    })) if database == watched_database && !linux_credentials => {
                        client.control.invalidate(LockReason::Authority);
                        break;
                    }
                    Some(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                        client.control.invalidate(LockReason::Transport);
                        break;
                    }
                    Some(Ok(
                        CoordinatorEvent::Received { database, .. }
                        | CoordinatorEvent::ControlChanged { database, .. }
                        | CoordinatorEvent::Committed(database),
                    )) if database != watched_database => continue,
                    Some(Ok(
                        CoordinatorEvent::JoinRequested { .. } | CoordinatorEvent::Frozen(_),
                    )) => continue,
                    _ => {}
                }
                let state = match watched_context.coordinator.state(&watched_database) {
                    Ok(state) => state,
                    Err(_) => {
                        client.control.invalidate(LockReason::Transport);
                        break;
                    }
                };
                if state.frozen || (admitted && !state.admitted) {
                    client.control.invalidate(LockReason::Authority);
                    break;
                }
                if state.epoch != epoch {
                    if linux_credentials && state.admitted {
                        let prepared = watched_context
                            .profile
                            .load_state::<Option<u64>>("credential_prepared")
                            .ok()
                            .flatten()
                            .flatten();
                        let finalized = watched_context
                            .profile
                            .load_state::<u64>("credential_final")
                            .ok()
                            .flatten();
                        if finalized == Some(state.epoch) && prepared.is_none() {
                            epoch = state.epoch;
                            pending_rotation = None;
                        } else if prepared == Some(state.epoch) {
                            if pending_rotation != Some(state.epoch) {
                                pending_rotation = Some(state.epoch);
                                await_rotation_credentials(
                                    Arc::clone(&watched_context),
                                    Arc::downgrade(&client),
                                    state.epoch,
                                );
                            }
                            continue;
                        } else {
                            client.control.invalidate(LockReason::Authority);
                            break;
                        }
                    } else {
                        client.control.invalidate(LockReason::Authority);
                        break;
                    }
                }
                if state.fingerprint != fingerprint {
                    // One queued wakeup is enough: the serialized service scans
                    // all durable packets, including reordered dependencies.
                    if apply_send.try_send(()).is_ok() {
                        fingerprint = state.fingerprint;
                    }
                }
            }
        });
        Ok(Self {
            client,
            database,
            watch,
            apply_task,
            application,
            application_revision,
            _sessions: sessions.clone(),
        })
    }
    /// Independent control handle; does not wait for command I/O or own plaintext.
    pub fn control(&self) -> WorkerControl {
        WorkerControl(Arc::clone(&self.client.control))
    }
    /// Logical identity authenticated during opening.
    pub fn database_id(&self) -> &DatabaseId {
        &self.database
    }
    /// Check access without waiting for a running command.
    pub fn is_open(&self) -> bool {
        self.client.control.check(false).is_ok()
    }
    /// Latest background application result; never returned after invalidation.
    pub fn application_progress(&self) -> Option<Result<Value, RuntimeError>> {
        if !self.is_open() {
            return None;
        }
        let mut result = self.application.lock().ok().and_then(|value| value.clone());
        if !self.is_open() {
            if let Some(Ok(value)) = &mut result {
                erase_view(value);
            }
            return None;
        }
        result
    }
    /// Nonsecret change counter for background application, including identical reports.
    /// A view must still check its session before accepting a subsequent query result.
    pub fn application_revision(&self) -> u64 {
        self.application_revision.load(Ordering::Acquire)
    }
    /// Nonsecret generation, phase and shutdown outcome.
    pub fn session_status(&self) -> session::SessionStatus {
        self.client.control.activity.expire();
        self.client.control.status()
    }
    /// Execute a command. It may return interrupted before an already-started ciphertext write ends.
    pub fn request(&mut self, command: &Command) -> Result<Value, RuntimeError> {
        if matches!(command, Command::Lock) {
            self.close()?;
            return Ok(Value::Null);
        }
        self.client.request(command, false)
    }
    /// Revoke access without waiting. All workers can start their shutdown at the same instant.
    pub fn invalidate(&self, reason: LockReason) {
        self.client.control.invalidate(reason);
    }
    /// Await a bounded shutdown and distinguish draft confirmation from forced termination.
    pub fn close_report(&mut self) -> Result<LockOutcome, RuntimeError> {
        self.watch.abort();
        self.apply_task.abort();
        self.invalidate(LockReason::Manual);
        let result = self.client.control.wait_closed();
        if let Ok(mut application) = self.application.lock() {
            if let Some(Ok(value)) = application.as_mut() {
                erase_view(value);
            }
            *application = None;
        }
        result
    }
    /// Compatibility API: unconfirmed/failed draft preservation is an error, even though access is closed.
    pub fn close(&mut self) -> Result<(), RuntimeError> {
        let outcome = self.close_report()?;
        if let Some(error) = outcome.error {
            return Err(error);
        }
        if outcome.draft != DraftDisposition::Preserved {
            return Err(RuntimeError::OperationInterrupted(outcome.reason));
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.watch.abort();
        self.apply_task.abort();
        self.invalidate(LockReason::HostExited);
        if let Ok(mut value) = self.application.lock()
            && let Some(Ok(value)) = value.as_mut()
        {
            erase_view(value);
        }
    }
}

fn authority_changed(state: &CoordinatorState, epoch: u64, admitted: bool) -> bool {
    state.frozen || state.epoch != epoch || (admitted && !state.admitted)
}

fn retry_application(result: &Result<Value, RuntimeError>) -> bool {
    matches!(
        result,
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(
                taypeer_storage::Error::Changed
                    | taypeer_storage::Error::Io
                    | taypeer_storage::Error::CommitUncertain
            )
        ))
    )
}

fn record_application(
    client: &Client,
    mut result: Result<Value, RuntimeError>,
    updates: &Mutex<Option<Result<Value, RuntimeError>>>,
    revision: &AtomicU64,
) {
    if matches!(
        result,
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::ExpiredSession
        ))
    ) {
        client.control.invalidate(LockReason::Authority);
    }
    // A readable session can intentionally lack author/write capabilities.
    if matches!(
        result,
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::ReadOnly
                | taypeer_services::ServiceError::WriteCompatibility
        ))
    ) {
        return;
    }
    if client.control.check(false).is_ok()
        && let Ok(mut status) = updates.lock()
    {
        if let Some(Ok(value)) = status.as_mut() {
            erase_view(value);
        }
        *status = Some(result);
        revision.fetch_add(1, Ordering::Release);
    } else if let Ok(value) = &mut result {
        erase_view(value);
    }
}

fn await_rotation_credentials(
    context: Arc<HostContext>,
    weak: std::sync::Weak<Client>,
    epoch: u64,
) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let final_epoch = context.profile.load_state::<u64>("credential_final");
            let prepared = context
                .profile
                .load_state::<Option<u64>>("credential_prepared");
            if matches!(final_epoch, Ok(Some(value)) if value == epoch)
                && matches!(prepared, Ok(Some(None)))
            {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Some(client) = weak.upgrade() {
            client.control.invalidate(LockReason::Authority);
        }
    });
}

/// Nonsecret control of one process generation, independent of its command queue.
#[derive(Clone)]
pub struct WorkerControl(Arc<session::ProcessControl>);
impl WorkerControl {
    /// Wait off the UI thread for the supervisor's bounded process shutdown.
    pub fn wait_closed(&self) -> Result<LockOutcome, RuntimeError> {
        self.0.wait_closed()
    }

    /// Immediately revoke this generation, even during a running command.
    pub fn invalidate(&self, reason: LockReason) {
        self.0.invalidate(reason);
    }
    /// Observe the current phase and durable draft outcome.
    pub fn status(&self) -> session::SessionStatus {
        self.0.activity.expire();
        self.0.status()
    }
    /// Check that this generation still accepts access.
    pub fn is_open(&self) -> bool {
        self.0.check(false).is_ok()
    }
}
