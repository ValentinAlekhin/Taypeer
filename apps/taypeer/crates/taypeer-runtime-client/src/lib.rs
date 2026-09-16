//! Background host and per-database command queues. GPUI never owns a worker or waits for I/O.
use serde::de::DeserializeOwned;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
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

/// Runtime failures crossing the background client boundary.
pub type Result<T> = std::result::Result<T, RuntimeError>;
/// The receiver owns only this request's result; dropping it drops the response.
pub struct Ticket<T>(mpsc::Receiver<Result<T>>, Option<WorkerControl>);
impl<T> Ticket<T> {
    /// Wait for completion off the UI thread; reject a revoked worker result.
    pub fn wait(self) -> Result<T> {
        let result = self.0.recv().map_err(|_| RuntimeError::Transport)?;
        if self.1.as_ref().is_some_and(|c| !c.is_open()) {
            Err(RuntimeError::Closed)
        } else {
            result
        }
    }

    /// Poll once without blocking; closed workers never return plaintext results.
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
/// A retained command queue for one unlocked database worker.
#[non_exhaustive]
pub struct Connection {
    /// Stable database identity attached to this worker.
    pub database: DatabaseId,
    /// Revocation handle shared with the session controller.
    pub control: WorkerControl,
    send: mpsc::Sender<Work>,
    updates: Arc<AtomicBool>,
    application: Arc<Mutex<ApplicationStatus>>,
}
#[derive(Clone, Copy)]
/// Latest application result for incoming encrypted changes.
pub enum ApplicationStatus {
    /// All received changes were applied.
    Applied,
    /// Number of changes still awaiting resolution.
    Pending(usize),
    /// Application failed; the UI must not report success.
    Failed,
}
impl ApplicationStatus {
    fn read(result: Result<serde_json::Value>) -> Self {
        match decode::<taypeer_services::ApplyReport>(result) {
            Ok(report) if report.pending.is_empty() => Self::Applied,
            Ok(report) => Self::Pending(report.pending.len()),
            Err(_) => Self::Failed,
        }
    }
}
impl Connection {
    /// Read the last application result only while the worker remains open.
    pub fn application(&self) -> Option<ApplicationStatus> {
        self.control.is_open().then(|| {
            self.application
                .lock()
                .map(|s| *s)
                .unwrap_or(ApplicationStatus::Failed)
        })
    }

    /// Consume a nonsecret notification that the worker applied incoming changes.
    pub fn take_updates(&self) -> bool {
        self.updates.swap(false, Ordering::AcqRel)
    }
    /// Enqueue work serially; check revocation before and after execution.
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
    /// Send a typed worker command, erasing its input when the queued job drops.
    pub fn command<T: DeserializeOwned + Send + 'static>(&self, command: Command) -> Ticket<T> {
        let command = Input(command);
        self.run(move |worker| decode(worker.request(&command.0)))
    }
    /// Read a coherent database projection through the worker queue.
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

/// An opened connection and its initial projection, owned by the receiving feature.
#[non_exhaustive]
pub struct Opened {
    /// Live worker command queue.
    pub connection: Connection,
    /// Initial database projection.
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
/// Device runtime client; owns background host work and session revocation.
#[non_exhaustive]
pub struct Backend {
    send: mpsc::Sender<HostJob>,
    /// Session lifetime and inactivity policy shared with platform capabilities.
    pub sessions: SessionController,
    /// Explicit device profile directory supplied by the shell.
    pub profile: PathBuf,
    host: Arc<Mutex<Option<Arc<RuntimeHost>>>>,
    closed: Arc<AtomicBool>,
    /// Executable used for isolated database workers.
    pub executable: PathBuf,
    fixture: bool,
}
impl Backend {
    /// Run a client operation against the existing network host without acquiring credentials.
    pub fn with_network<T: Send + 'static>(
        &self,
        operation: impl FnOnce(Option<Arc<RuntimeHost>>) -> Result<T> + Send + 'static,
    ) -> Ticket<T> {
        let shared = Arc::clone(&self.host);
        background(move || {
            let host = shared.lock().map_err(|_| RuntimeError::Transport)?.clone();
            operation(host)
        })
    }
    /// Start a client using the current executable and an explicit profile.
    pub fn new(profile: PathBuf) -> Result<Self> {
        Self::configured(
            profile,
            std::env::current_exe().map_err(|_| RuntimeError::Transport)?,
            false,
        )
    }
    /// Start a client with an explicit worker; fixture mode is for synthetic tests.
    pub fn configured(profile: PathBuf, executable: PathBuf, fixture: bool) -> Result<Self> {
        let worker_executable = executable.clone();

        let policy = SessionSettings::load(&profile)?;
        let sessions = SessionController::new(policy);
        let controller = sessions.clone();
        let directory = profile.clone();
        let (send, receive) = mpsc::channel();
        let host = Arc::new(Mutex::new(None));
        let shared_host = Arc::clone(&host);
        let closed = Arc::new(AtomicBool::new(false));
        let host_closed = Arc::clone(&closed);
        thread::spawn(move || {
            // Acquiring a profile/Keychain is deferred until an explicit file/network operation.
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
                            let host = acquire_host(
                                &shared_host,
                                &directory,
                                &controller,
                                &host_closed,
                                fixture,
                            )?;
                            let executable = &worker_executable;
                            let mut worker = host.open_configured(
                                executable,
                                &path,
                                password.to_string(),
                                form,
                            )?;
                            if controller.activity().epoch() != epoch {
                                worker.invalidate(taypeer_runtime::session::LockReason::Manual);
                                return Err(RuntimeError::Closed);
                            }
                            let database = worker.database_id().clone();
                            let application = Arc::new(Mutex::new(ApplicationStatus::read(
                                worker.request(&Command::ApplyReceived),
                            )));
                            let snapshot = Snapshot::read(&mut worker, Query::default())?;
                            let (jobs, pending) = mpsc::channel::<Work>();
                            let updates = Arc::new(AtomicBool::new(false));
                            let connection = Connection {
                                database,
                                control: worker.control(),
                                send: jobs,
                                updates: Arc::clone(&updates),
                                application: Arc::clone(&application),
                            };
                            thread::spawn(move || {
                                run_worker(worker, pending, updates, application)
                            });
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
                            let host = shared_host
                                .lock()
                                .map_err(|_| RuntimeError::Transport)?
                                .clone()
                                .ok_or(RuntimeError::Closed)?;
                            host.close(&database)
                        });
                        let _ = reply.send(result);
                    }
                }
            }
            controller.lock_all(taypeer_runtime::session::LockReason::HostExited);
            if let Ok(mut host) = shared_host.lock()
                && let Some(host) = host.take()
            {
                host.stop_network();
            }
        });
        Ok(Self {
            send,
            sessions,
            profile,
            host,
            closed,
            executable,
            fixture,
        })
    }
    /// Run network work in the background, initializing the host when required.
    pub fn network<T: Send + 'static>(
        &self,
        work: impl FnOnce(&RuntimeHost, &taypeer_runtime::NetworkCancellation) -> Result<T>
        + Send
        + 'static,
    ) -> NetworkTicket<T> {
        let host = Arc::clone(&self.host);
        let profile = self.profile.clone();
        let sessions = self.sessions.clone();
        let cancellation = taypeer_runtime::NetworkCancellation::default();
        let task_cancel = cancellation.clone();
        let closed = Arc::clone(&self.closed);
        let fixture = self.fixture;
        let ticket = background(move || {
            let host = acquire_host(&host, &profile, &sessions, &closed, fixture)?;
            work(&host, &task_cancel)
        });
        NetworkTicket {
            ticket,
            cancellation,
        }
    }
    /// Queue database creation or unlock; the password is erased after use.
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
    /// Revoke and close a database worker through its owning host.
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
fn acquire_host(
    slot: &Mutex<Option<Arc<RuntimeHost>>>,
    directory: &Path,
    sessions: &SessionController,
    closed: &AtomicBool,
    fixture: bool,
) -> Result<Arc<RuntimeHost>> {
    let mut slot = slot.lock().map_err(|_| RuntimeError::Transport)?;
    if closed.load(Ordering::Acquire) {
        return Err(RuntimeError::Closed);
    }
    if slot.is_none() {
        #[cfg(feature = "ui-test-support")]
        let host = if fixture {
            RuntimeHost::with_test_sessions(directory, sessions.clone())?
        } else {
            RuntimeHost::with_sessions(directory, sessions.clone())?
        };
        #[cfg(not(feature = "ui-test-support"))]
        let host = {
            if fixture {
                return Err(RuntimeError::Protocol);
            }
            RuntimeHost::with_sessions(directory, sessions.clone())?
        };
        *slot = Some(Arc::new(host));
    }
    slot.as_ref().cloned().ok_or(RuntimeError::Closed)
}
/// A network request whose cancellation flag follows the receiving feature.
pub struct NetworkTicket<T> {
    ticket: Ticket<T>,
    cancellation: taypeer_runtime::NetworkCancellation,
}
impl<T> NetworkTicket<T> {
    /// Poll completion without blocking the application context.
    pub fn try_take(&self) -> Option<Result<T>> {
        self.ticket.try_take()
    }
}
impl<T> Drop for NetworkTicket<T> {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
fn run_worker(
    mut worker: Worker,
    jobs: mpsc::Receiver<Work>,
    updates: Arc<AtomicBool>,
    application: Arc<Mutex<ApplicationStatus>>,
) {
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
            if let Some(result) = worker.application_progress()
                && let Ok(mut status) = application.lock()
            {
                *status = ApplicationStatus::read(result);
            }
            updates.store(true, Ordering::Release);
        }
    }
    // Drop revokes access; the shared supervisor finishes the bounded shutdown.
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        self.sessions
            .lock_all(taypeer_runtime::session::LockReason::HostExited);
    }
}

#[derive(Clone, Default)]
/// Owned query parameters for a database projection.
#[non_exhaustive]
pub struct Query {
    /// Optional group restriction.
    pub group: Option<GroupId>,
    /// Search text evaluated by the service.
    pub search: String,
    /// Entry whose detail projection is requested.
    pub selected: Option<EntryId>,
}
#[derive(serde::Deserialize)]
/// Pending draft metadata returned without secret draft contents.
#[non_exhaustive]
pub struct DraftStatus {
    /// An available draft summary, if present.
    pub pending: Option<PendingDraftSummary>,
}
/// A coherent worker projection; its owner must discard it on revocation.
#[non_exhaustive]
pub struct Snapshot {
    /// Query that produced this projection.
    pub query: Query,
    /// Database identity and metadata.
    pub info: DatabaseInfo,
    /// Group hierarchy returned by the service.
    pub groups: Vec<GroupInfo>,
    /// Rows in service order; identities are independent of position.
    pub rows: Vec<EntrySummary>,
    /// Bounded image previews for the current result.
    pub previews: Vec<taypeer_services::IconPreview>,
    /// Binary database content, when applicable.
    pub binary: Option<taypeer_services::BinaryView>,
    /// Selected entry detail, when available.
    pub entry: Option<EntryView>,
    /// Selected entry revision summaries.
    pub history: Vec<RevisionSummary>,
    /// Recoverable draft summary, without its plaintext.
    pub pending: Option<PendingDraftSummary>,
    /// Storage accounting from the same worker.
    pub usage: StorageUsage,
    /// Database security policy.
    pub policy: taypeer_core::DatabasePolicy,
    /// Whether this session may commit changes.
    pub writable: bool,
}
impl Query {
    /// Capture the filter and selected entry for one worker request.
    pub fn new(group: Option<GroupId>, search: String, selected: Option<EntryId>) -> Self {
        Self {
            group,
            search,
            selected,
        }
    }
}
impl Snapshot {
    /// Construct a synthetic empty projection from explicit public metadata.
    #[cfg(feature = "ui-test-support")]
    pub fn fixture(info: DatabaseInfo, usage: StorageUsage) -> Self {
        Self {
            query: Query::default(),
            info,
            groups: Vec::new(),
            rows: Vec::new(),
            previews: Vec::new(),
            binary: None,
            entry: None,
            history: Vec::new(),
            pending: None,
            usage,
            policy: Default::default(),
            writable: true,
        }
    }

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

/// Inspect only the public file identity on a background thread.
pub fn inspect_path(path: PathBuf) -> Ticket<DatabaseId> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let result = RuntimeHost::inspect_compatibility(&path).map(|(id, _)| id);
        let _ = send.send(result);
    });
    Ticket(receive, None)
}

/// Run a finite local settings operation without blocking rendering.
pub fn background<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Ticket<T> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(job());
    });
    Ticket(receive, None)
}

/// Translate content-free failure categories without exposing debug payloads.
pub fn error_key(error: &RuntimeError) -> &'static str {
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
