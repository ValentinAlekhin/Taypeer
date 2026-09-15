//! Background host and per-database command queues. GPUI never owns a worker or waits for I/O.
use serde::de::DeserializeOwned;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
use taypeer_core::{DatabaseId, EntryId, GroupId};
use taypeer_runtime::{
    Command, RuntimeError, RuntimeHost, Worker, WorkerControl,
    session::{SessionController, SessionSettings},
};
use taypeer_services::{
    CreateDatabase, DatabaseInfo, EntrySummary, EntryView, GroupInfo, PendingDraftSummary,
    RevisionSummary, StorageUsage,
};

pub(crate) type Result<T> = std::result::Result<T, RuntimeError>;
/// The receiver owns only this request's result; dropping it drops the response.
pub(crate) struct Ticket<T>(mpsc::Receiver<Result<T>>, Option<WorkerControl>);
impl<T> Ticket<T> {
    pub fn wait(self) -> Result<T> {
        let result = self.0.recv().map_err(|_| RuntimeError::Transport)?;
        if self.1.as_ref().is_some_and(|c| !c.is_open()) {
            Err(RuntimeError::Closed)
        } else {
            result
        }
    }

    pub fn try_take(&self) -> Option<Result<T>> {
        match self.0.try_recv() {
            Ok(value) => Some(if self.1.as_ref().is_some_and(|c| !c.is_open()) {
                Err(RuntimeError::Closed)
            } else {
                value
            }),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(RuntimeError::Transport)),
        }
    }
}
type Work = Box<dyn FnOnce(&mut Worker) + Send>;
#[derive(Clone)]
pub(crate) struct Connection {
    pub database: DatabaseId,
    pub control: WorkerControl,
    send: mpsc::Sender<Work>,
    updates: Arc<AtomicBool>,
}
impl Connection {
    /// Consume a nonsecret notification that the worker applied incoming changes.
    pub fn take_updates(&self) -> bool {
        self.updates.swap(false, Ordering::AcqRel)
    }
    pub fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Worker) -> Result<T> + Send + 'static,
    ) -> Ticket<T> {
        let (send, receive) = mpsc::channel();
        let control = self.control.clone();
        // Dropped receivers mean their screen/session no longer accepts the response.
        let job = Box::new(move |worker: &mut Worker| {
            let result = if control.is_open() {
                work(worker)
            } else {
                Err(RuntimeError::Closed)
            };
            let result = if control.is_open() {
                result
            } else {
                Err(RuntimeError::Closed)
            };
            let _ = send.send(result);
        });
        // A dead worker drops the closure and disconnects the ticket.
        let _ = self.send.send(job);
        Ticket(receive, Some(self.control.clone()))
    }
    pub fn command<T: DeserializeOwned + Send + 'static>(&self, command: Command) -> Ticket<T> {
        let command = Input(command);
        self.run(move |worker| decode(worker.request(&command.0)))
    }
    pub fn query(&self, query: Query) -> Ticket<Snapshot> {
        self.run(move |worker| Snapshot::read(worker, query))
    }
}
fn request<T: DeserializeOwned>(worker: &mut Worker, mut command: Command) -> Result<T> {
    let value = worker.request(&command);
    command.erase_input();
    decode(value)
}
struct Input(Command);
impl Drop for Input {
    fn drop(&mut self) {
        self.0.erase_input();
    }
}
fn decode<T: DeserializeOwned>(value: Result<serde_json::Value>) -> Result<T> {
    let mut value = value?;
    // Decode by reference so even malformed responses are erased, rather than moved into errors.
    let result = T::deserialize(&value).map_err(|_| RuntimeError::Protocol);
    taypeer_runtime::erase_view(&mut value);
    result
}

pub(crate) struct Opened {
    pub connection: Connection,
    pub snapshot: Snapshot,
}
enum HostJob {
    Open {
        path: PathBuf,
        password: zeroize::Zeroizing<String>,
        form: Option<CreateDatabase>,
        epoch: u64,
        reply: mpsc::Sender<Result<Opened>>,
    },
    Close {
        database: DatabaseId,
        control: WorkerControl,
        reply: mpsc::Sender<Result<()>>,
    },
}
pub(crate) struct Backend {
    send: mpsc::Sender<HostJob>,
    pub sessions: SessionController,
    pub profile: PathBuf,
}
impl Backend {
    pub fn new() -> Result<Self> {
        let profile = RuntimeHost::default_profile_path()?;
        let policy = SessionSettings::load(&profile)?;
        let sessions = SessionController::new(policy);
        let controller = sessions.clone();
        let directory = profile.clone();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            // Acquiring a profile/Keychain is deferred until the user's first file operation.
            let mut host = None;
            while let Ok(job) = receive.recv() {
                match job {
                    HostJob::Open {
                        path,
                        password,
                        form,
                        epoch,
                        reply,
                    } => {
                        let result = (|| {
                            if controller.activity().epoch() != epoch {
                                return Err(RuntimeError::Closed);
                            }
                            if host.is_none() {
                                host = Some(RuntimeHost::with_sessions(
                                    &directory,
                                    controller.clone(),
                                )?);
                            }
                            let executable =
                                std::env::current_exe().map_err(|_| RuntimeError::Transport)?;
                            let mut worker = host
                                .as_ref()
                                .ok_or(RuntimeError::Closed)?
                                .open_configured(&executable, &path, password.to_string(), form)?;
                            if controller.activity().epoch() != epoch {
                                worker.invalidate(taypeer_runtime::session::LockReason::Manual);
                                return Err(RuntimeError::Closed);
                            }
                            let database = worker.database_id().clone();
                            let snapshot = Snapshot::read(&mut worker, Query::default())?;
                            let (jobs, pending) = mpsc::channel::<Work>();
                            let updates = Arc::new(AtomicBool::new(false));
                            let connection = Connection {
                                database,
                                control: worker.control(),
                                send: jobs,
                                updates: Arc::clone(&updates),
                            };
                            thread::spawn(move || run_worker(worker, pending, updates));
                            Ok(Opened {
                                connection,
                                snapshot,
                            })
                        })();
                        let _ = reply.send(result);
                    }
                    HostJob::Close {
                        database,
                        control,
                        reply,
                    } => {
                        let result = control.wait_closed().and_then(|outcome| {
                            if let Some(error) = outcome.error {
                                return Err(error);
                            }
                            host.as_ref().ok_or(RuntimeError::Closed)?.close(&database)
                        });
                        let _ = reply.send(result);
                    }
                }
            }
            controller.lock_all(taypeer_runtime::session::LockReason::HostExited);
        });
        Ok(Self {
            send,
            sessions,
            profile,
        })
    }
    pub fn open(
        &self,
        path: &Path,
        password: String,
        form: Option<CreateDatabase>,
    ) -> Ticket<Opened> {
        let (reply, receive) = mpsc::channel();
        let _ = self.send.send(HostJob::Open {
            path: path.into(),
            password: zeroize::Zeroizing::new(password),
            form,
            epoch: self.sessions.activity().epoch(),
            reply,
        });
        Ticket(receive, None)
    }
    pub fn close(&self, database: DatabaseId, control: WorkerControl) -> Ticket<()> {
        let (reply, receive) = mpsc::channel();
        let _ = self.send.send(HostJob::Close {
            database,
            control,
            reply,
        });
        Ticket(receive, None)
    }
}
fn run_worker(mut worker: Worker, jobs: mpsc::Receiver<Work>, updates: Arc<AtomicBool>) {
    let mut previous = 0;
    loop {
        match jobs.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(work) => work(&mut worker),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let revision = worker.application_revision();
        if revision != previous {
            previous = revision;
            updates.store(true, Ordering::Release);
        }
    }
    // Drop revokes access; the shared supervisor finishes the bounded shutdown.
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.sessions
            .lock_all(taypeer_runtime::session::LockReason::HostExited);
    }
}

#[derive(Clone, Default)]
pub(crate) struct Query {
    pub group: Option<GroupId>,
    pub search: String,
    pub selected: Option<EntryId>,
}
#[derive(serde::Deserialize)]
pub(crate) struct DraftStatus {
    pub pending: Option<PendingDraftSummary>,
}
pub(crate) struct Snapshot {
    pub query: Query,
    pub info: DatabaseInfo,
    pub groups: Vec<GroupInfo>,
    pub rows: Vec<EntrySummary>,
    pub previews: Vec<taypeer_services::IconPreview>,
    pub binary: Option<taypeer_services::BinaryView>,
    pub entry: Option<EntryView>,
    pub history: Vec<RevisionSummary>,
    pub pending: Option<PendingDraftSummary>,
    pub usage: StorageUsage,
    pub policy: taypeer_core::DatabasePolicy,
    pub writable: bool,
}
impl Snapshot {
    fn read(worker: &mut Worker, query: Query) -> Result<Self> {
        let info: DatabaseInfo = request(worker, Command::DatabaseInfo)?;
        let groups: Vec<GroupInfo> = request(worker, Command::GroupInfo)?;
        let requested = query.clone();
        let rows: Vec<EntrySummary> = request(
            worker,
            Command::Entries {
                group: query.group,
                query: query.search,
            },
        )?;
        let mut previews = Vec::new();
        let mut loaded = std::collections::BTreeSet::new();
        let targets = groups
            .iter()
            .map(|g| {
                (
                    &g.group.icon,
                    taypeer_services::BinaryTarget::Group(g.group.id.clone()),
                )
            })
            .chain(rows.iter().map(|row| {
                (
                    &row.appearance.icon,
                    taypeer_services::BinaryTarget::Entry(row.id.clone()),
                )
            }));
        for (icon, target) in targets {
            if let Some(blob) = icon.blob()
                && loaded.insert(blob.clone())
                && let Some(preview) = request::<Option<taypeer_services::IconPreview>>(
                    worker,
                    Command::IconPreview(target),
                )?
            {
                previews.push(preview);
            }
        }
        let binary = query
            .selected
            .as_ref()
            .map(|entry| {
                request(
                    worker,
                    Command::BinaryView(taypeer_services::BinaryTarget::Entry(entry.clone())),
                )
            })
            .transpose()?;
        let (entry, history) = if let Some(id) = query.selected {
            (
                Some(request(worker, Command::Entry(id.clone()))?),
                request(worker, Command::History(id))?,
            )
        } else {
            (None, Vec::new())
        };
        let pending = request::<DraftStatus>(worker, Command::DraftStatus)?.pending;
        let compatibility: taypeer_services::SessionValue<taypeer_core::CompatibilityReport> =
            request(worker, Command::Compatibility)?;
        let writable = info.writable && compatibility.value.write.is_supported();
        Ok(Self {
            query: requested,
            info,
            groups,
            rows,
            previews,
            binary,
            entry,
            history,
            pending,
            usage: request(worker, Command::StorageUsage)?,
            policy: request(worker, Command::DatabasePolicy)?,
            writable,
        })
    }
}

pub(crate) fn inspect_path(path: PathBuf) -> Ticket<DatabaseId> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let result = RuntimeHost::inspect_compatibility(&path).map(|(id, _)| id);
        let _ = send.send(result);
    });
    Ticket(receive, None)
}

/// Run a finite local settings operation without blocking rendering.
pub(crate) fn background<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Ticket<T> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(job());
    });
    Ticket(receive, None)
}

/// Translate content-free failure categories without exposing debug payloads.
pub(crate) fn error_key(error: &RuntimeError) -> &'static str {
    use taypeer_services::ServiceError as E;
    use taypeer_storage::Error as S;
    match error {
        RuntimeError::Service(E::Storage(S::Authentication)) => "ui.authentication_failed",
        RuntimeError::Service(E::Storage(S::AlreadyExists)) => "ui.file_exists",
        RuntimeError::Service(E::Storage(S::Busy)) => "ui.file_busy",
        RuntimeError::Service(E::Storage(S::CommitUncertain | S::Changed)) => "ui.commit_uncertain",
        RuntimeError::Service(E::Storage(S::Io)) => "ui.file_error",
        RuntimeError::Service(
            E::Storage(S::InvalidFile | S::UnsupportedVersion) | E::ReadCompatibility,
        ) => "ui.incompatible_file",
        RuntimeError::Service(E::AttachmentLimit) => "ui.attachment_limit_exceeded",
        RuntimeError::Service(E::AwaitingData | E::Storage(S::MissingBlob)) => {
            "ui.attachment_unavailable"
        }
        RuntimeError::Service(E::Icon(_)) => "ui.icon_failed",
        RuntimeError::Service(E::InvalidInput) => "ui.invalid_input",
        RuntimeError::Service(E::Unauthorized | E::ReadOnly | E::WriteCompatibility) => {
            "ui.read_only"
        }
        RuntimeError::Service(E::Conflict) => "ui.conflict",
        RuntimeError::Service(E::EditorAlreadyOpen) => "ui.finish_editing",
        RuntimeError::Service(E::DraftNeedsRestore) => "ui.restore_first",
        RuntimeError::Closed
        | RuntimeError::SessionClosed(_)
        | RuntimeError::OperationInterrupted(_)
        | RuntimeError::Service(E::Locked | E::ExpiredSession) => "ui.session_closed",
        RuntimeError::Profile(_) | RuntimeError::Service(E::Credentials) => "ui.credentials_error",
        _ => "ui.operation_failed",
    }
}
