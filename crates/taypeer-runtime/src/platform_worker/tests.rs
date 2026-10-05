//! PUBLIC-only platform adapter: real processes, opaque descriptor leases and private streams.
use super::*;
use crate::{
    Command, RuntimeHost,
    platform::{ProcessConnection, ProcessLauncher},
    profile::{CredentialStore, ProfileError},
    protocol::{read_frame, write_frame},
    session::{LockReason, SessionController, Termination},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, BufRead, BufReader},
    net::{TcpListener, TcpStream},
    process::{Command as Process, Stdio},
    sync::{
        Condvar,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use taypeer_core::{EntryId, OperationId};
use taypeer_services::{DraftSaveOutcome, EditorView, EntryPatch, FieldUpdate};
use taypeer_storage::{
    ArchiveJournal, CiphertextFile, CiphertextIo, Error as StorageError, TemporaryFileProvider,
};
use taypeer_trust::{ControlChain, SignedControl};
use zeroize::Zeroizing;

const PASSWORD: &str = "PUBLIC descriptor password";
const MARKER: &str = "PUBLIC descriptor process ready";

#[derive(Serialize, Deserialize)]
enum Request {
    Allocate,
    Length(u64),
    Read(u64, u64, usize),
    Write(u64, u64, Vec<u8>),
    Snapshot,
    Create {
        controls: Vec<SignedControl>,
        objects: Vec<u64>,
        checkpoint: Digest,
        baseline: Digest,
    },
    Commit {
        expected: Digest,
        control: Digest,
        controls: Vec<SignedControl>,
        objects: Vec<u64>,
        remove: BTreeSet<Digest>,
        checkpoint: Digest,
        baseline: Digest,
        journal: ArchiveJournal,
    },
    Author(Option<Digest>),
    Transport,
    WorkingCopy,
    SaveDraft(u64),
    LoadDraft,
    DiscardDraft,
    OpenSelectedInput(u64),
    ReadSelectedInput(u64, usize),
    OpenSelectedOutput(u64),
    WriteSelectedOutput(u64, Vec<u8>),
    FinishSelectedOutput(u64),
}
#[derive(Serialize, Deserialize)]
enum Reply {
    Done,
    Ticket(u64),
    OptionalTicket(Option<u64>),
    Size(u64),
    Bytes(Vec<u8>),
    Count(usize),
    Snapshot { ticket: u64, root: Digest },
    Author(Zeroizing<[u8; 32]>),
    Public(PublicKey),
    Digest(Digest),
}
struct Rpc(Mutex<TcpStream>);
impl Rpc {
    fn call(&self, request: Request) -> Result<Reply, RuntimeError> {
        let mut stream = self.0.lock().map_err(|_| RuntimeError::Transport)?;
        write_frame(&mut *stream, &request)?;
        read_frame::<Result<Reply, RuntimeError>>(&mut *stream)?
    }
}
fn storage(error: RuntimeError) -> StorageError {
    match error {
        RuntimeError::Service(taypeer_services::ServiceError::Storage(error)) => error,
        _ => StorageError::Io,
    }
}
struct Lease {
    rpc: Arc<Rpc>,
    ticket: u64,
}
impl CiphertextIo for Lease {
    fn length(&self) -> io::Result<u64> {
        match self.rpc.call(Request::Length(self.ticket)) {
            Ok(Reply::Size(size)) => Ok(size),
            _ => Err(io::ErrorKind::Other.into()),
        }
    }
    fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        match self
            .rpc
            .call(Request::Read(self.ticket, offset, bytes.len().min(65536)))
        {
            Ok(Reply::Bytes(data)) if data.len() <= bytes.len() => {
                bytes[..data.len()].copy_from_slice(&data);
                Ok(data.len())
            }
            _ => Err(io::ErrorKind::Other.into()),
        }
    }
    fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
        match self.rpc.call(Request::Write(
            self.ticket,
            offset,
            bytes[..bytes.len().min(65536)].to_vec(),
        )) {
            Ok(Reply::Count(count)) => Ok(count),
            _ => Err(io::ErrorKind::Other.into()),
        }
    }
}
struct Provider(Arc<Rpc>);
impl TemporaryFileProvider for Provider {
    fn create(&self) -> io::Result<CiphertextFile> {
        match self.0.call(Request::Allocate) {
            Ok(Reply::Ticket(ticket)) => Ok(CiphertextFile::new(Arc::new(Lease {
                rpc: Arc::clone(&self.0),
                ticket,
            }))),
            _ => Err(io::ErrorKind::Other.into()),
        }
    }
}
struct Document {
    rpc: Arc<Rpc>,
    temporary: TemporaryStorage,
}
struct SelectedReader {
    rpc: Arc<Rpc>,
    id: u64,
}
impl Read for SelectedReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self
            .rpc
            .call(Request::ReadSelectedInput(self.id, bytes.len().min(65536)))
        {
            Ok(Reply::Bytes(data)) if data.len() <= bytes.len() => {
                bytes[..data.len()].copy_from_slice(&data);
                Ok(data.len())
            }
            _ => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
}
struct SelectedWriter {
    rpc: Arc<Rpc>,
    id: u64,
}
impl Write for SelectedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.rpc.call(Request::WriteSelectedOutput(
            self.id,
            bytes[..bytes.len().min(65536)].to_vec(),
        )) {
            Ok(Reply::Count(count)) if count <= bytes.len() => Ok(count),
            _ => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl SelectedOutput for SelectedWriter {
    fn finish(&mut self) -> Result<(), RuntimeError> {
        match self.rpc.call(Request::FinishSelectedOutput(self.id))? {
            Reply::Done => Ok(()),
            _ => Err(RuntimeError::Protocol),
        }
    }
}
impl Document {
    fn lease(&self, ticket: u64) -> CiphertextFile {
        CiphertextFile::new(Arc::new(Lease {
            rpc: Arc::clone(&self.rpc),
            ticket,
        }))
    }
    fn upload(&self, object: &EncryptedObject) -> Result<u64, StorageError> {
        let ticket = match self.rpc.call(Request::Allocate).map_err(storage)? {
            Reply::Ticket(ticket) => ticket,
            _ => return Err(StorageError::Io),
        };
        std::io::copy(&mut object.reader()?, &mut self.lease(ticket))?;
        Ok(ticket)
    }
    fn snapshot_reply(&self, reply: Reply) -> Result<ArchiveSnapshot, StorageError> {
        let Reply::Snapshot { ticket, root } = reply else {
            return Err(StorageError::Io);
        };
        ArchiveSnapshot::from_source(self.lease(ticket), Some(root), self.temporary.clone())
    }
    fn done(&self, request: Request) -> Result<(), StorageError> {
        match self.rpc.call(request).map_err(storage)? {
            Reply::Done => Ok(()),
            _ => Err(StorageError::Io),
        }
    }
}
impl CipherPersistence for Document {
    fn snapshot(&self) -> Result<ArchiveSnapshot, StorageError> {
        self.snapshot_reply(self.rpc.call(Request::Snapshot).map_err(storage)?)
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, StorageError> {
        let objects = request
            .objects
            .iter()
            .map(|object| self.upload(object))
            .collect::<Result<_, _>>()?;
        self.snapshot_reply(
            self.rpc
                .call(Request::Commit {
                    expected: request.expected,
                    control: request.control,
                    controls: request.controls,
                    objects,
                    remove: request.remove,
                    checkpoint: request.checkpoint,
                    baseline: request.baseline,
                    journal: request.journal,
                })
                .map_err(storage)?,
        )
    }
    fn working_copy(&self) -> Digest {
        match self.rpc.call(Request::WorkingCopy).unwrap() {
            Reply::Digest(id) => id,
            _ => panic!("invalid public working-copy reply"),
        }
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), StorageError> {
        self.done(Request::SaveDraft(self.upload(object)?))
    }
    fn load_draft(&self, chain: &ControlChain) -> Result<Option<EncryptedObject>, StorageError> {
        match self.rpc.call(Request::LoadDraft).map_err(storage)? {
            Reply::OptionalTicket(ticket) => ticket
                .map(|id| {
                    EncryptedObject::open_source(self.lease(id), chain, self.temporary.clone())
                })
                .transpose(),
            _ => Err(StorageError::Io),
        }
    }
    fn discard_draft(&self) -> Result<(), StorageError> {
        self.done(Request::DiscardDraft)
    }
    fn path(&self) -> &Path {
        Path::new("")
    }
}
impl PlatformDocument for Document {
    fn temporary(&self) -> TemporaryStorage {
        self.temporary.clone()
    }
    fn create(&self, seed: ArchiveSeed) -> Result<(), RuntimeError> {
        let objects = seed
            .objects
            .iter()
            .map(|object| self.upload(object))
            .collect::<Result<_, _>>()
            .map_err(crate::cipher_ipc::storage)?;
        self.done(Request::Create {
            controls: seed.controls,
            objects,
            checkpoint: seed.checkpoint,
            baseline: seed.baseline,
        })
        .map_err(crate::cipher_ipc::storage)
    }
    fn author(&self, authenticated: Option<Digest>) -> Result<Option<AuthorKey>, RuntimeError> {
        match self.rpc.call(Request::Author(authenticated))? {
            Reply::Author(seed) => Ok(Some(AuthorKey::from_seed(&seed))),
            _ => Err(RuntimeError::Protocol),
        }
    }
    fn transport_public(&self) -> Result<PublicKey, RuntimeError> {
        match self.rpc.call(Request::Transport)? {
            Reply::Public(key) => Ok(key),
            _ => Err(RuntimeError::Protocol),
        }
    }
    fn selected_input(&self, id: u64) -> Result<Box<dyn Read + Send>, RuntimeError> {
        self.done(Request::OpenSelectedInput(id))
            .map_err(crate::cipher_ipc::storage)?;
        Ok(Box::new(SelectedReader {
            rpc: Arc::clone(&self.rpc),
            id,
        }))
    }
    fn selected_output(&self, id: u64) -> Result<Box<dyn SelectedOutput>, RuntimeError> {
        self.done(Request::OpenSelectedOutput(id))
            .map_err(crate::cipher_ipc::storage)?;
        Ok(Box::new(SelectedWriter {
            rpc: Arc::clone(&self.rpc),
            id,
        }))
    }
}
#[test]
#[ignore = "Private descriptor worker subprocess for PUBLIC adapter scenarios"]
fn child() {
    let Some(address) = std::env::var_os("TAYPEER_PUBLIC_DESCRIPTOR_RPC") else {
        return;
    };
    let stream = TcpStream::connect(address.to_str().unwrap()).unwrap();
    stream.set_nodelay(true).unwrap();
    let rpc = Arc::new(Rpc(Mutex::new(stream)));
    let temporary = TemporaryStorage::new(Arc::new(Provider(Arc::clone(&rpc))));
    println!("\n{MARKER}");
    std::io::stdout().flush().unwrap();
    let result = run_platform_worker(
        std::io::stdin(),
        std::io::stdout(),
        Arc::new(Document { rpc, temporary }),
    );
    std::process::exit(if result.is_ok() { 0 } else { 70 });
}
type AuthHook = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct Credentials {
    values: Mutex<BTreeMap<(String, String), Vec<u8>>>,
    author_prompt: Mutex<Option<AuthHook>>,
}
impl CredentialStore for Credentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        let value = self
            .values
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned()
            .map(Zeroizing::new);
        if account == "author" {
            let hook = self.author_prompt.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        Ok(value)
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        self.values
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), bytes.to_vec());
        Ok(())
    }
}
#[derive(Default)]
struct Faults {
    author_calls: AtomicUsize,
    allocations: AtomicUsize,
    deny_allocate: AtomicBool,
    fail_commit: AtomicBool,
    hold_commit: Mutex<bool>,
    hold_changed: Condvar,
    commit_entered: AtomicBool,
    before_author: Mutex<Option<AuthHook>>,
    selected_inputs: Mutex<BTreeMap<u64, io::Cursor<Vec<u8>>>>,
    selected_outputs: Mutex<BTreeMap<u64, Vec<u8>>>,
    input_reads: AtomicUsize,
    largest_input_read: AtomicUsize,
    fail_input: AtomicBool,
    fail_output_finish: AtomicBool,
    output_finishes: AtomicUsize,
}
struct Server {
    writer: Arc<PlatformCipherWriter>,
    directory: PathBuf,
    faults: Arc<Faults>,
    leases: Vec<File>,
}
impl Server {
    fn allocate(&mut self) -> Result<u64, RuntimeError> {
        self.leases
            .push(tempfile::tempfile_in(&self.directory).map_err(|_| RuntimeError::Transport)?);
        Ok((self.leases.len() - 1) as u64)
    }
    fn snapshot(&mut self, snapshot: ArchiveSnapshot) -> Result<Reply, RuntimeError> {
        let ticket = self.allocate()?;
        snapshot
            .copy_ciphertext(&mut self.leases[ticket as usize])
            .map_err(crate::cipher_ipc::storage)?;
        Ok(Reply::Snapshot {
            ticket,
            root: snapshot
                .chain()
                .root()
                .map_err(|_| RuntimeError::Protocol)?,
        })
    }
    fn object(&self, ticket: u64, chain: &ControlChain) -> Result<EncryptedObject, RuntimeError> {
        EncryptedObject::open_file(
            self.leases[ticket as usize]
                .try_clone()
                .map_err(|_| RuntimeError::Transport)?,
            chain,
            TemporaryStorage::in_directory(self.directory.clone()),
        )
        .map_err(crate::cipher_ipc::storage)
    }
    fn handle(&mut self, request: Request) -> Result<Reply, RuntimeError> {
        match request {
            Request::Allocate => {
                self.faults.allocations.fetch_add(1, Ordering::Relaxed);
                if self.faults.deny_allocate.load(Ordering::Acquire) {
                    return Err(crate::cipher_ipc::storage(StorageError::Io));
                }
                Ok(Reply::Ticket(self.allocate()?))
            }
            Request::Length(id) => Ok(Reply::Size(
                self.leases[id as usize]
                    .metadata()
                    .map_err(|_| RuntimeError::Transport)?
                    .len(),
            )),
            Request::Read(id, offset, length) => {
                let mut bytes = vec![0; length];
                let count = CiphertextIo::read_at(&self.leases[id as usize], &mut bytes, offset)
                    .map_err(|_| RuntimeError::Transport)?;
                bytes.truncate(count);
                Ok(Reply::Bytes(bytes))
            }
            Request::Write(id, offset, bytes) => Ok(Reply::Count(
                CiphertextIo::write_at(&self.leases[id as usize], &bytes, offset)
                    .map_err(|_| RuntimeError::Transport)?,
            )),
            Request::Snapshot => self.snapshot(self.writer.snapshot()?),
            Request::Transport => Ok(Reply::Public(self.writer.transport_public())),
            Request::WorkingCopy => Ok(Reply::Digest(self.writer.working_copy()?)),
            Request::Author(authenticated) => {
                self.faults.author_calls.fetch_add(1, Ordering::Relaxed);
                if authenticated.is_some() {
                    let hook = self.faults.before_author.lock().unwrap().take();
                    if let Some(hook) = hook {
                        hook();
                    }
                }
                Ok(Reply::Author(
                    self.writer.author(authenticated)?.secret_seed(),
                ))
            }
            Request::Create {
                controls,
                objects,
                checkpoint,
                baseline,
            } => {
                let root = controls
                    .first()
                    .ok_or(RuntimeError::Protocol)?
                    .hash()
                    .map_err(|_| RuntimeError::Protocol)?;
                let chain = ControlChain::validate(controls.clone(), root)
                    .map_err(|_| RuntimeError::Protocol)?;
                let objects = objects
                    .into_iter()
                    .map(|id| self.object(id, &chain))
                    .collect::<Result<_, _>>()?;
                self.writer.create(ArchiveSeed {
                    controls,
                    objects,
                    checkpoint,
                    baseline,
                })?;
                Ok(Reply::Done)
            }
            Request::Commit {
                expected,
                control,
                controls,
                objects,
                remove,
                checkpoint,
                baseline,
                journal,
            } => {
                self.faults.commit_entered.store(true, Ordering::Release);
                let mut hold = self.faults.hold_commit.lock().unwrap();
                while *hold {
                    hold = self.faults.hold_changed.wait(hold).unwrap();
                }
                drop(hold);
                if self.faults.fail_commit.load(Ordering::Acquire) {
                    return Err(crate::cipher_ipc::storage(StorageError::Io));
                }
                let root = controls
                    .first()
                    .ok_or(RuntimeError::Protocol)?
                    .hash()
                    .map_err(|_| RuntimeError::Protocol)?;
                let chain = ControlChain::validate(controls.clone(), root)
                    .map_err(|_| RuntimeError::Protocol)?;
                let objects = objects
                    .into_iter()
                    .map(|id| self.object(id, &chain))
                    .collect::<Result<_, _>>()?;
                let snapshot = self.writer.commit(PreparedCommit {
                    expected,
                    control,
                    controls,
                    objects,
                    remove,
                    checkpoint,
                    baseline,
                    journal,
                })?;
                self.snapshot(snapshot)
            }
            Request::SaveDraft(id) => {
                let chain = self.writer.snapshot()?.chain().clone();
                self.writer.save_draft(&self.object(id, &chain)?)?;
                Ok(Reply::Done)
            }
            Request::LoadDraft => {
                let ticket = if let Some(object) = self.writer.load_draft()? {
                    let ticket = self.allocate()?;
                    std::io::copy(
                        &mut object.reader().map_err(crate::cipher_ipc::storage)?,
                        &mut self.leases[ticket as usize],
                    )
                    .map_err(|_| RuntimeError::Transport)?;
                    Some(ticket)
                } else {
                    None
                };
                Ok(Reply::OptionalTicket(ticket))
            }
            Request::DiscardDraft => {
                self.writer.discard_draft()?;
                Ok(Reply::Done)
            }
            Request::OpenSelectedInput(id) => {
                self.faults
                    .selected_inputs
                    .lock()
                    .unwrap()
                    .get_mut(&id)
                    .ok_or(RuntimeError::Protocol)?
                    .set_position(0);
                Ok(Reply::Done)
            }
            Request::ReadSelectedInput(id, length) => {
                if length > 65536 || self.faults.fail_input.load(Ordering::Acquire) {
                    return Err(RuntimeError::Transport);
                }
                self.faults.input_reads.fetch_add(1, Ordering::Relaxed);
                self.faults
                    .largest_input_read
                    .fetch_max(length, Ordering::Relaxed);
                let mut bytes = vec![0; length];
                let count = self
                    .faults
                    .selected_inputs
                    .lock()
                    .unwrap()
                    .get_mut(&id)
                    .ok_or(RuntimeError::Protocol)?
                    .read(&mut bytes)
                    .map_err(|_| RuntimeError::Transport)?;
                bytes.truncate(count);
                Ok(Reply::Bytes(bytes))
            }
            Request::OpenSelectedOutput(id) => {
                self.faults
                    .selected_outputs
                    .lock()
                    .unwrap()
                    .get_mut(&id)
                    .ok_or(RuntimeError::Protocol)?
                    .clear();
                Ok(Reply::Done)
            }
            Request::WriteSelectedOutput(id, bytes) => {
                if bytes.len() > 65536 {
                    return Err(RuntimeError::Protocol);
                }
                let count = bytes.len();
                self.faults
                    .selected_outputs
                    .lock()
                    .unwrap()
                    .get_mut(&id)
                    .ok_or(RuntimeError::Protocol)?
                    .extend(bytes);
                Ok(Reply::Count(count))
            }
            Request::FinishSelectedOutput(id) => {
                if self.faults.fail_output_finish.load(Ordering::Acquire) {
                    return Err(crate::cipher_ipc::storage(StorageError::Io));
                }
                if !self
                    .faults
                    .selected_outputs
                    .lock()
                    .unwrap()
                    .contains_key(&id)
                {
                    return Err(RuntimeError::Protocol);
                }
                self.faults.output_finishes.fetch_add(1, Ordering::Release);
                Ok(Reply::Done)
            }
        }
    }
    fn run(mut self, mut stream: TcpStream) {
        while let Ok(request) = read_frame::<Request>(&mut stream) {
            let result = self.handle(request);
            if write_frame(&mut stream, &result).is_err() {
                break;
            }
        }
    }
}
struct Launcher {
    writer: Arc<PlatformCipherWriter>,
    directory: PathBuf,
    faults: Arc<Faults>,
}
impl ProcessLauncher for Launcher {
    fn launch(&self) -> Result<ProcessConnection, RuntimeError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("PUBLIC callback bind failed: {:?}", error.kind()));
        let mut child = Process::new(std::env::current_exe().map_err(|_| RuntimeError::Transport)?)
            .args([
                "--exact",
                "platform_worker::tests::child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(
                "TAYPEER_PUBLIC_DESCRIPTOR_RPC",
                listener.local_addr().unwrap().to_string(),
            )
            .env(
                "TMPDIR",
                "/dev/null/PUBLIC forbidden global temporary directory",
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::Transport)?;
        let input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (stream, _) = listener.accept().map_err(|_| RuntimeError::Transport)?;
        stream.set_nodelay(true).unwrap();
        let server = Server {
            writer: Arc::clone(&self.writer),
            directory: self.directory.clone(),
            faults: Arc::clone(&self.faults),
            leases: Vec::new(),
        };
        std::thread::spawn(move || server.run(stream));
        loop {
            let mut line = String::new();
            if output
                .read_line(&mut line)
                .map_err(|_| RuntimeError::Transport)?
                == 0
            {
                return Err(RuntimeError::Transport);
            }
            if line.trim() == MARKER {
                break;
            }
        }
        Ok(ProcessConnection::new(
            Box::new(crate::platform::DesktopProcess(Some(child))),
            Box::new(input),
            Box::new(output),
        ))
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    host: RuntimeHost,
    path: PathBuf,
    launcher: Launcher,
    credentials: Arc<Credentials>,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let credentials = Arc::new(Credentials::default());
        let host = RuntimeHost::with_platform_credentials(
            &directory.path().join("profile"),
            SessionController::new(Default::default()),
            credentials.clone(),
        )
        .unwrap();
        let path = directory.path().join("PUBLIC descriptor.taypeer");
        let writer = host.platform_cipher_writer(&path).unwrap();
        let launcher = Launcher {
            writer,
            directory: directory.path().to_owned(),
            faults: Arc::new(Faults::default()),
        };
        Self {
            _directory: directory,
            host,
            path,
            launcher,
            credentials,
        }
    }
    fn open(&self, password: &str, create: bool) -> Result<Worker, RuntimeError> {
        self.host.open_platform_worker(
            &self.launcher,
            password.into(),
            create.then(|| taypeer_services::CreateDatabase {
                name: "PUBLIC descriptor database".into(),
                description: None,
                policy: taypeer_core::DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500)
                    .unwrap(),
            }),
        )
    }
}
fn editor(worker: &mut Worker) -> EditorView {
    serde_json::from_value(worker.request(&Command::EditorView).unwrap()).unwrap()
}
fn edit(worker: &mut Worker, title: &str) {
    worker.request(&Command::BeginCreateUngrouped).unwrap();
    worker
        .request(&Command::PatchDraft(EntryPatch {
            title: FieldUpdate::Set(title.into()),
            password: FieldUpdate::Set("PUBLIC protected descriptor value".into()),
            ..Default::default()
        }))
        .unwrap();
}
fn save(
    worker: &mut Worker,
    view: &EditorView,
    operation: OperationId,
) -> Result<serde_json::Value, RuntimeError> {
    worker.request(&Command::SaveDraftSnapshot {
        draft: view.identity.draft.clone(),
        revision: view.identity.revision,
        operation,
    })
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "public descriptor scenario timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn authority_change(fixture: &Fixture, database: &taypeer_core::DatabaseId) -> AuthHook {
    let mut service = DatabaseService::new();
    let session = fixture
        .host
        .open_local(&mut service, &fixture.path, PASSWORD.as_bytes(), None)
        .unwrap();
    assert_eq!(&session.database, database);
    Box::new(move || {
        service
            .rotate_password(
                &session,
                Digest::of(b"PUBLIC rotation during authentication"),
                b"PUBLIC rotated descriptor password",
                None,
            )
            .unwrap();
    })
}

#[test]
fn ciphertext_receipt_during_authentication_keeps_exact_snapshot_authority_valid() {
    let fixture = Fixture::new();
    let mut original = fixture.open(PASSWORD, true).unwrap();
    edit(&mut original, "PUBLIC authenticated before receipt");
    let view = editor(&mut original);
    let saved: DraftSaveOutcome = serde_json::from_value(
        save(
            &mut original,
            &view,
            OperationId::new("PUBLIC auth receipt snapshot"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(saved, DraftSaveOutcome::Saved { .. }));
    let entry = editor(&mut original).entry.unwrap();
    let database = original.database_id().clone();
    original.close().unwrap();
    let coordinator = Arc::clone(fixture.host.coordinator());
    let expected = coordinator.snapshot(&database).unwrap().fingerprint();
    *fixture.launcher.faults.before_author.lock().unwrap() = Some(Box::new(move || {
        let snapshot = coordinator.snapshot(&database).unwrap();
        let mut journal = snapshot.metadata().journal.clone();
        journal
            .retained
            .insert(snapshot.metadata().manifest.body.checkpoint);
        let received = coordinator
            .commit(
                &database,
                PreparedCommit {
                    expected: snapshot.fingerprint(),
                    control: snapshot.chain().head_hash().unwrap(),
                    controls: snapshot.chain().records().to_vec(),
                    objects: Vec::new(),
                    remove: BTreeSet::new(),
                    checkpoint: snapshot.metadata().manifest.body.checkpoint,
                    baseline: snapshot.metadata().manifest.body.baseline,
                    journal,
                },
            )
            .unwrap();
        assert_ne!(received.fingerprint(), expected);
        assert_eq!(received.chain(), snapshot.chain());
    }));
    let mut reopened = fixture.open(PASSWORD, false).unwrap();
    assert_eq!(
        reopened.request(&Command::Entry(entry)).unwrap()["title"],
        "PUBLIC authenticated before receipt"
    );
    // Consumed or arbitrary fingerprints cannot become reusable author capabilities.
    assert!(fixture.launcher.writer.author(Some(expected)).is_err());
    assert!(
        fixture
            .launcher
            .writer
            .author(Some(Digest::of(b"PUBLIC arbitrary generation")))
            .is_err()
    );
    reopened.close().unwrap();
}

#[test]
fn rotation_after_snapshot_authentication_does_not_publish_a_session() {
    let fixture = Fixture::new();
    let mut original = fixture.open(PASSWORD, true).unwrap();
    let database = original.database_id().clone();
    original.close().unwrap();
    *fixture.launcher.faults.before_author.lock().unwrap() =
        Some(authority_change(&fixture, &database));
    assert!(matches!(
        fixture.open(PASSWORD, false),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(StorageError::Changed)
        ))
    ));
}

#[test]
fn rotation_during_native_credential_prompt_does_not_release_author() {
    let fixture = Fixture::new();
    let mut original = fixture.open(PASSWORD, true).unwrap();
    let database = original.database_id().clone();
    original.close().unwrap();
    *fixture.credentials.author_prompt.lock().unwrap() =
        Some(authority_change(&fixture, &database));
    assert!(matches!(
        fixture.open(PASSWORD, false),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(StorageError::Changed)
        ))
    ));
}

fn selected_import(
    worker: &mut Worker,
    draft: &taypeer_core::DraftId,
    input: u64,
    length: u64,
    replacement: Option<taypeer_core::AttachmentId>,
    operation: OperationId,
) -> Result<serde_json::Value, RuntimeError> {
    worker.request(&Command::ImportSelectedAttachment {
        draft: draft.clone(),
        input,
        length,
        name: "PUBLIC descriptor attachment.bin".into(),
        replacement,
        operation,
    })
}

#[test]
fn selected_attachment_streams_retry_encrypt_reopen_and_finish_the_export() {
    let f = Fixture::new();
    let bytes: Vec<_> = (0..150_000).map(|index| (index % 251) as u8).collect();
    f.launcher
        .faults
        .selected_inputs
        .lock()
        .unwrap()
        .insert(1, io::Cursor::new(bytes.clone()));
    f.launcher
        .faults
        .selected_outputs
        .lock()
        .unwrap()
        .insert(2, b"PUBLIC old output".to_vec());
    let mut worker = f.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC streamed descriptor attachment");
    let draft = editor(&mut worker).identity.draft;
    let operation = OperationId::new("PUBLIC descriptor selected input");
    selected_import(
        &mut worker,
        &draft,
        1,
        bytes.len() as u64,
        None,
        operation.clone(),
    )
    .unwrap();
    let first = editor(&mut worker);
    let binary: taypeer_services::BinaryView = serde_json::from_value(
        worker
            .request(&Command::BinaryView(taypeer_services::BinaryTarget::Draft))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(binary.attachments.len(), 1);
    let blob = binary.attachments[0].contents[0].id.clone();
    let reads = f.launcher.faults.input_reads.load(Ordering::Acquire);
    selected_import(
        &mut worker,
        &draft,
        1,
        bytes.len() as u64,
        None,
        operation.clone(),
    )
    .unwrap();
    assert!(f.launcher.faults.input_reads.load(Ordering::Acquire) > reads);
    assert!(f.launcher.faults.largest_input_read.load(Ordering::Acquire) <= 65536);
    assert_eq!(editor(&mut worker).identity, first.identity);
    save(
        &mut worker,
        &first,
        OperationId::new("PUBLIC selected descriptor snapshot"),
    )
    .unwrap();
    let entry = editor(&mut worker).entry.unwrap();
    worker
        .request(&Command::PatchDraft(EntryPatch {
            notes: FieldUpdate::Set("PUBLIC newer stream editor input".into()),
            ..Default::default()
        }))
        .unwrap();
    let newer = editor(&mut worker);
    selected_import(&mut worker, &draft, 1, bytes.len() as u64, None, operation).unwrap();
    assert_eq!(editor(&mut worker).identity, newer.identity);
    worker.close().unwrap();
    assert!(
        !std::fs::read(&f.path)
            .unwrap()
            .windows(64)
            .any(|part| part == &bytes[..64])
    );
    let mut worker = f.open(PASSWORD, false).unwrap();
    worker.request(&Command::ResumeDraft(draft)).unwrap();
    let binary = worker
        .request(&Command::BinaryView(taypeer_services::BinaryTarget::Draft))
        .unwrap();
    assert_eq!(binary["attachments"].as_array().unwrap().len(), 1);
    let export = Command::ExportSelectedBinary {
        target: taypeer_services::BinaryTarget::Entry(entry),
        blob,
        output: 2,
    };
    worker.request(&export).unwrap();
    assert_eq!(
        f.launcher.faults.selected_outputs.lock().unwrap()[&2],
        bytes
    );
    assert_eq!(f.launcher.faults.output_finishes.load(Ordering::Acquire), 1);
    f.launcher
        .faults
        .fail_output_finish
        .store(true, Ordering::Release);
    assert_eq!(
        worker.request(&export).unwrap_err(),
        RuntimeError::Service(taypeer_services::ServiceError::Storage(StorageError::Io))
    );
    assert_eq!(f.launcher.faults.output_finishes.load(Ordering::Acquire), 1);
    worker.close().unwrap();
}

#[test]
fn selected_stream_limits_failure_stale_editor_and_visibility_preserve_state() {
    let f = Fixture::new();
    let bytes = b"PUBLIC original selected content".to_vec();
    f.launcher
        .faults
        .selected_inputs
        .lock()
        .unwrap()
        .insert(1, io::Cursor::new(bytes.clone()));
    f.launcher
        .faults
        .selected_outputs
        .lock()
        .unwrap()
        .insert(2, b"PUBLIC retained output".to_vec());
    let mut worker = f.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC bounded stream");
    let draft = editor(&mut worker).identity.draft;
    let operation = OperationId::new("PUBLIC stream baseline");
    selected_import(
        &mut worker,
        &draft,
        1,
        bytes.len() as u64,
        None,
        operation.clone(),
    )
    .unwrap();
    let before = editor(&mut worker);
    let binary: taypeer_services::BinaryView = serde_json::from_value(
        worker
            .request(&Command::BinaryView(taypeer_services::BinaryTarget::Draft))
            .unwrap(),
    )
    .unwrap();
    let attachment = binary.attachments[0].id.clone();
    let blob = binary.attachments[0].contents[0].id.clone();
    f.launcher
        .faults
        .selected_inputs
        .lock()
        .unwrap()
        .insert(1, io::Cursor::new(vec![b'x'; bytes.len()]));
    assert!(matches!(
        selected_import(&mut worker, &draft, 1, bytes.len() as u64, None, operation),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::InvalidInput
        ))
    ));
    f.launcher.faults.fail_input.store(true, Ordering::Release);
    assert!(
        selected_import(
            &mut worker,
            &draft,
            1,
            bytes.len() as u64,
            Some(attachment),
            OperationId::new("PUBLIC broken replacement")
        )
        .is_err()
    );
    let reads = f.launcher.faults.input_reads.load(Ordering::Acquire);
    assert!(matches!(
        selected_import(
            &mut worker,
            &draft,
            1,
            1024 * 1024 + 1,
            None,
            OperationId::new("PUBLIC oversized selection")
        ),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::AttachmentLimit
        ))
    ));
    assert_eq!(f.launcher.faults.input_reads.load(Ordering::Acquire), reads);
    assert_eq!(editor(&mut worker).identity, before.identity);
    worker
        .request(&Command::ExportSelectedBinary {
            target: taypeer_services::BinaryTarget::Draft,
            blob,
            output: 2,
        })
        .unwrap();
    assert_eq!(
        f.launcher.faults.selected_outputs.lock().unwrap()[&2],
        bytes
    );
    f.launcher
        .faults
        .selected_outputs
        .lock()
        .unwrap()
        .insert(2, b"PUBLIC retained output".to_vec());
    assert!(
        worker
            .request(&Command::ExportSelectedBinary {
                target: taypeer_services::BinaryTarget::Draft,
                blob: taypeer_core::BlobId::new("PUBLIC inaccessible blob"),
                output: 2
            })
            .is_err()
    );
    assert_eq!(
        f.launcher.faults.selected_outputs.lock().unwrap()[&2],
        b"PUBLIC retained output"
    );
    worker.request(&Command::BeginCreateUngrouped).unwrap();
    assert!(matches!(
        selected_import(
            &mut worker,
            &draft,
            1,
            bytes.len() as u64,
            None,
            OperationId::new("PUBLIC stale selected editor")
        ),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::InvalidContext
        ))
    ));
    assert_eq!(f.launcher.faults.input_reads.load(Ordering::Acquire), reads);
    worker.close().unwrap();
}

#[test]
fn clean_picker_pin_survives_process_background_without_a_document_revision() {
    let f = Fixture::new();
    let mut worker = f.open(PASSWORD, true).unwrap();
    worker.request(&Command::BeginCreateUngrouped).unwrap();
    let clean = editor(&mut worker);
    assert!(!clean.dirty);
    let before = f.launcher.writer.snapshot().unwrap().fingerprint();
    let pinned: taypeer_services::DraftIdentity =
        serde_json::from_value(worker.request(&Command::PinActiveForm).unwrap()).unwrap();
    assert_eq!(pinned, clean.identity);
    worker.request(&Command::PersistDrafts).unwrap();
    worker.close().unwrap();
    let mut worker = f.open(PASSWORD, false).unwrap();
    let forms = worker.request(&Command::Drafts).unwrap();
    assert_eq!(forms.as_array().unwrap().len(), 1);
    assert_eq!(forms[0]["dirty"], false);
    worker
        .request(&Command::ResumeDraft(pinned.draft.clone()))
        .unwrap();
    let resumed = editor(&mut worker);
    assert_eq!(resumed.identity, pinned);
    assert!(!resumed.dirty);
    worker.request(&Command::BeginCreateUngrouped).unwrap();
    let replacement: taypeer_services::DraftIdentity =
        serde_json::from_value(worker.request(&Command::PinActiveForm).unwrap()).unwrap();
    assert_ne!(replacement.draft, pinned.draft);
    worker
        .request(&Command::UnpinForm(pinned.draft.clone()))
        .unwrap();
    assert_eq!(editor(&mut worker).identity, replacement);
    let forms = worker.request(&Command::Drafts).unwrap();
    assert_eq!(forms.as_array().unwrap().len(), 1);
    assert_eq!(
        forms[0]["identity"]["draft"],
        serde_json::to_value(&replacement.draft).unwrap()
    );
    worker.close().unwrap();
    let mut worker = f.open(PASSWORD, false).unwrap();
    worker
        .request(&Command::ResumeDraft(replacement.draft.clone()))
        .unwrap();
    assert!(matches!(
        worker.request(&Command::ResumeDraft(pinned.draft)),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::NoDraft
        ))
    ));
    worker
        .request(&Command::UnpinForm(replacement.draft.clone()))
        .unwrap();
    assert_eq!(editor(&mut worker).identity, replacement);
    assert_eq!(f.launcher.writer.snapshot().unwrap().fingerprint(), before);
    worker.close().unwrap();
    let mut worker = f.open(PASSWORD, false).unwrap();
    assert!(
        worker
            .request(&Command::Drafts)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(worker.request(&Command::ActiveDraft).unwrap().is_null());
    worker.close().unwrap();
}

#[test]
fn invitation_only_process_returns_public_proof_without_opening_a_document() {
    let manager = Fixture::new();
    let mut opened = manager.open(PASSWORD, true).unwrap();
    let mut encoded = opened.request(&Command::CreateInvitation).unwrap();
    let invitation: taypeer_trust::Invitation = serde_json::from_value(encoded[0].take()).unwrap();
    opened.close().unwrap();
    let mut receiver = Fixture::new();
    let request = invitation.id().unwrap();
    receiver.path = receiver
        .host
        .creation_path(&OperationId::new(format!("join:{request}")))
        .unwrap();
    receiver.launcher.writer = receiver
        .host
        .platform_cipher_writer(&receiver.path)
        .unwrap();
    let proof = receiver
        .host
        .platform_join_proof(&receiver.launcher, invitation.clone())
        .unwrap();
    proof
        .verify(&invitation, receiver.host.profile().transport_public())
        .unwrap();
    assert!(
        receiver
            .host
            .profile()
            .identity()
            .unwrap()
            .is_some_and(|identity| identity == proof.recipient)
    );
    assert_eq!(
        receiver
            .launcher
            .faults
            .author_calls
            .load(Ordering::Acquire),
        1
    );
    assert_eq!(
        receiver.launcher.faults.allocations.load(Ordering::Acquire),
        0
    );
    assert!(!receiver.path.exists());
    assert!(receiver.host.working_copies().unwrap().is_empty());
    assert!(receiver.host.pending_joins().unwrap().is_empty());
    let statuses = receiver.host.sessions().statuses();
    assert_eq!(statuses.len(), 1);
    assert!(
        statuses[0]
            .outcome
            .as_ref()
            .is_some_and(|outcome| outcome.termination == Termination::Graceful)
    );
}

#[test]
fn descriptor_process_creates_autosaves_and_reopens_the_confirmed_file() {
    let f = Fixture::new();
    let mut worker = f.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC descriptor entry");
    let captured = editor(&mut worker);
    assert!(captured.fields.password.is_none());
    let result: DraftSaveOutcome = serde_json::from_value(
        save(
            &mut worker,
            &captured,
            taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(result, DraftSaveOutcome::Saved { .. }));
    let entry = editor(&mut worker).entry.unwrap();
    worker.request(&Command::BeginEdit(entry.clone())).unwrap();
    worker
        .request(&Command::PatchDraft(EntryPatch {
            username: FieldUpdate::Set("PUBLIC edited descriptor username".into()),
            ..Default::default()
        }))
        .unwrap();
    let edited = editor(&mut worker);
    save(
        &mut worker,
        &edited,
        taypeer_services::new_operation_id().unwrap(),
    )
    .unwrap();
    assert_eq!(
        worker
            .request(&Command::History(entry.clone()))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    worker.close().unwrap();
    let snapshot = ArchiveSnapshot::open(&f.path, None).unwrap();
    assert_eq!(
        snapshot.chain().head().database,
        worker.database_id().clone()
    );
    let bytes = std::fs::read(&f.path).unwrap();
    assert!(
        !bytes
            .windows(b"PUBLIC protected descriptor value".len())
            .any(|w| w == b"PUBLIC protected descriptor value")
    );
    let mut reopened = f.open(PASSWORD, false).unwrap();
    let row = reopened.request(&Command::Entry(entry)).unwrap();
    assert_eq!(row["title"], "PUBLIC descriptor entry");
    assert_eq!(row["username"], "PUBLIC edited descriptor username");
    assert!(row["password"].is_null());
    reopened.close().unwrap();
}
#[test]
fn wrong_password_never_acquires_author_and_encrypted_drafts_resume_in_a_new_generation() {
    let f = Fixture::new();
    let mut first = f.open(PASSWORD, true).unwrap();
    edit(&mut first, "PUBLIC unfinished descriptor draft");
    let before = editor(&mut first);
    first.close().unwrap();
    let calls = f.launcher.faults.author_calls.load(Ordering::Acquire);
    assert!(matches!(
        f.open("PUBLIC incorrect password", false),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(StorageError::Authentication)
        ))
    ));
    assert_eq!(
        f.launcher.faults.author_calls.load(Ordering::Acquire),
        calls
    );
    let draft = f.path.with_file_name(format!(
        "{}.{}.draft",
        f.path.file_name().unwrap().to_str().unwrap(),
        f.launcher.writer.working_copy().unwrap()
    ));
    let bytes = std::fs::read(draft).unwrap();
    assert!(
        !bytes
            .windows(b"PUBLIC protected descriptor value".len())
            .any(|w| w == b"PUBLIC protected descriptor value")
    );
    let mut next = f.open(PASSWORD, false).unwrap();
    assert_ne!(
        first.session_status().generation,
        next.session_status().generation
    );
    next.request(&Command::ResumeDraft(before.identity.draft.clone()))
        .unwrap();
    let restored = editor(&mut next);
    assert_eq!(restored.identity, before.identity);
    assert!(restored.dirty);
    save(
        &mut next,
        &restored,
        taypeer_services::new_operation_id().unwrap(),
    )
    .unwrap();
    next.close().unwrap();
}
#[test]
fn failed_descriptor_allocator_has_no_global_temporary_fallback() {
    let denied = Fixture::new();
    denied
        .launcher
        .faults
        .deny_allocate
        .store(true, Ordering::Release);
    assert!(denied.open(PASSWORD, true).is_err());
    assert!(!denied.path.exists());
    assert!(denied.launcher.faults.allocations.load(Ordering::Acquire) > 0);
    let f = Fixture::new();
    let mut worker = f.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC allocation failure");
    let captured = editor(&mut worker);
    let before = f.launcher.writer.snapshot().unwrap().fingerprint();
    let attempts = f.launcher.faults.allocations.load(Ordering::Acquire);
    f.launcher
        .faults
        .deny_allocate
        .store(true, Ordering::Release);
    assert!(
        save(
            &mut worker,
            &captured,
            taypeer_services::new_operation_id().unwrap()
        )
        .is_err()
    );
    assert!(f.launcher.faults.allocations.load(Ordering::Acquire) > attempts);
    assert_eq!(f.launcher.writer.snapshot().unwrap().fingerprint(), before);
    f.launcher
        .faults
        .deny_allocate
        .store(false, Ordering::Release);
    assert!(editor(&mut worker).dirty);
    worker.close().unwrap();
}
#[test]
fn failed_commit_remains_dirty_and_retries_without_an_extra_history_row() {
    let f = Fixture::new();
    let mut worker = f.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC failed descriptor commit");
    let captured = editor(&mut worker);
    let operation = taypeer_services::new_operation_id().unwrap();
    let before = f.launcher.writer.snapshot().unwrap().fingerprint();
    f.launcher.faults.fail_commit.store(true, Ordering::Release);
    assert!(matches!(
        save(&mut worker, &captured, operation.clone()),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(StorageError::Io)
        ))
    ));
    assert_eq!(f.launcher.writer.snapshot().unwrap().fingerprint(), before);
    assert!(editor(&mut worker).dirty);
    f.launcher
        .faults
        .fail_commit
        .store(false, Ordering::Release);
    save(&mut worker, &captured, operation).unwrap();
    let entry: EntryId = editor(&mut worker).entry.unwrap();
    assert_eq!(
        worker
            .request(&Command::History(entry))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    worker.close().unwrap();
}
struct HeldCommit(Arc<Faults>);
impl HeldCommit {
    fn new(faults: Arc<Faults>) -> Self {
        *faults.hold_commit.lock().unwrap() = true;
        faults.commit_entered.store(false, Ordering::Release);
        Self(faults)
    }
}
impl Drop for HeldCommit {
    fn drop(&mut self) {
        self.0.fail_commit.store(true, Ordering::Release);
        *self.0.hold_commit.lock().unwrap() = false;
        self.0.hold_changed.notify_all();
    }
}
#[test]
fn a_blocked_descriptor_commit_cannot_prevent_revocation_and_forced_process_exit() {
    let f = Fixture::new();
    let mut worker = f.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC blocked descriptor commit");
    let captured = editor(&mut worker);
    let hold = HeldCommit::new(Arc::clone(&f.launcher.faults));
    let client = Arc::clone(&worker.client);
    let command = Command::SaveDraftSnapshot {
        draft: captured.identity.draft,
        revision: captured.identity.revision,
        operation: taypeer_services::new_operation_id().unwrap(),
    };
    let pending = std::thread::spawn(move || client.request(&command, false));
    wait(|| f.launcher.faults.commit_entered.load(Ordering::Acquire));
    let now = Instant::now();
    worker.invalidate(LockReason::Background);
    assert!(!worker.is_open());
    assert!(now.elapsed() < Duration::from_millis(100));
    assert!(pending.join().unwrap().is_err());
    let closed = worker.control().wait_closed().unwrap();
    assert_eq!(closed.termination, Termination::Forced);
    assert!(now.elapsed() < Duration::from_secs(3));
    drop(hold);
    let _closed = worker.close_report();
}
