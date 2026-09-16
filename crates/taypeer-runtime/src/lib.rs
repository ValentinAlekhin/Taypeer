//! Private command IPC and independent lifetimes for plaintext workers and ciphertext writers.
mod cipher_ipc;
mod host;
mod network;
mod process;
pub mod profile;
mod protocol;
pub mod session;
mod worker;
pub use host::{RegisteredCompatibility, RuntimeHost};
pub use network::{
    DatabaseExchange, DeviceExchange, InvitationCode, JoinProgress, NetworkCancellation,
    NetworkSnapshot, PeerProgress, PendingJoin,
};
pub use protocol::{Command, RuntimeError, erase_view};
pub use worker::run_worker;

use host::{Callbacks, HostContext};
use process::{Client, EnrollmentCallbacks};
use protocol::Boot;
use serde_json::Value;
use session::{DraftDisposition, LockOutcome, LockReason, SessionController};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use taypeer_core::DatabaseId;
use taypeer_sync::CoordinatorEvent;
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
        profile: &profile::NativeProfile,
        invitation: taypeer_trust::Invitation,
        sessions: &SessionController,
    ) -> Result<taypeer_trust::JoinProof, RuntimeError> {
        let spool = tempfile::tempdir().map_err(|_| RuntimeError::Transport)?;
        let boot = Boot {
            path: profile.directory().to_owned(),
            password: String::new(),
            create_name: None,
            create_form: None,
            profile: profile.directory().to_owned(),
            spool: spool.path().to_owned(),
            invitation: Some(invitation),
        };
        let client = Client::spawn(executable, sessions, Box::new(EnrollmentCallbacks), spool)?;
        let result = client
            .request(&boot, true)
            .and_then(|value| serde_json::from_value(value).map_err(|_| RuntimeError::Protocol));
        client.control.invalidate(LockReason::Manual);
        let closed = client.control.wait_closed();
        // A failed or unconfirmed process shutdown cannot confirm enrollment.
        let outcome = closed?;
        if let Some(error) = outcome.error {
            return Err(error);
        }
        if outcome.draft != DraftDisposition::Preserved {
            return Err(RuntimeError::OperationInterrupted(outcome.reason));
        }
        result
    }
    pub(crate) fn open(
        executable: &Path,
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
        let mut events = context.coordinator.subscribe();
        let client = Client::spawn(
            executable,
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
        let weak = Arc::downgrade(&client);
        let watched_database = database.clone();
        let application = Arc::new(Mutex::new(None));
        let application_revision = Arc::new(AtomicU64::new(0));
        let revision = Arc::clone(&application_revision);
        let updates = Arc::clone(&application);
        let (apply_send, mut apply_receive) = tokio::sync::mpsc::channel(1);
        let apply_client = Arc::downgrade(&client);
        let apply_task = runtime.spawn(async move {
            while apply_receive.recv().await.is_some() {
                let Some(client) = apply_client.upgrade() else {
                    break;
                };
                let updates = Arc::clone(&updates);
                let revision = Arc::clone(&revision);
                let _ = tokio::task::spawn_blocking(move || {
                    let mut result = client.request(&Command::ApplyReceived, false);
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
                })
                .await;
            }
        });
        let watch = runtime.spawn(async move {
            loop {
                let (invalidate, apply) = match events.recv().await {
                    Ok(
                        CoordinatorEvent::ControlChanged {
                            database,
                            lock: true,
                            ..
                        }
                        | CoordinatorEvent::Frozen(database),
                    ) => (database == watched_database, false),
                    Ok(
                        CoordinatorEvent::Received { database, .. }
                        | CoordinatorEvent::ControlChanged {
                            database,
                            lock: false,
                            ..
                        },
                    ) => (false, database == watched_database),
                    Ok(_) => (false, false),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => (true, false),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let Some(client) = weak.upgrade() else {
                    break;
                };
                if invalidate {
                    client.control.invalidate(LockReason::Authority);
                    break;
                }
                if apply {
                    let _ = apply_send.try_send(());
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
