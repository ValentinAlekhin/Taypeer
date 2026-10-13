//! PUBLIC real archives and selected-output faults exercise ciphertext-only SAF ownership.
use super::*;
use crate::CredentialPort;
use std::{
    collections::BTreeMap,
    fs,
    io::{Seek, SeekFrom},
    sync::{
        Condvar, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use taypeer_services::{
    AttachmentImport, DatabaseService, EntryPatch, FieldUpdate, SessionToken, new_operation_id,
};

const PASSWORD: &[u8] = b"PUBLIC Native encrypted-copy master";
type CredentialValues = BTreeMap<(String, String), Vec<u8>>;
#[derive(Clone, Default)]
struct Credentials(Arc<Mutex<CredentialValues>>);
impl CredentialPort for Credentials {
    fn get(&self, service: String, account: String) -> Result<Option<Vec<u8>>, AndroidError> {
        Ok(self.0.lock().unwrap().get(&(service, account)).cloned())
    }
    fn set(&self, service: String, account: String, bytes: Vec<u8>) -> Result<(), AndroidError> {
        self.0.lock().unwrap().insert((service, account), bytes);
        Ok(())
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    host: Arc<Host>,
    service: DatabaseService,
    session: SessionToken,
    copy: WorkingCopy,
}
impl Fixture {
    fn new(large: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let host = Host::new(
            directory.path().join("profile").to_str().unwrap().into(),
            Box::new(Credentials::default()),
        )
        .unwrap();
        let path = directory.path().join("PUBLIC.taypeer");
        let mut service = DatabaseService::new();
        let session = host
            .runtime
            .open_local(
                &mut service,
                &path,
                PASSWORD,
                Some("PUBLIC encrypted copy".into()),
            )
            .unwrap();
        if large {
            let group = service
                .create_group(
                    &session,
                    "PUBLIC group".into(),
                    None,
                    &new_operation_id().unwrap(),
                )
                .unwrap()
                .value
                .id;
            let entry = service
                .create_entry(
                    &session,
                    group,
                    EntryPatch {
                        title: FieldUpdate::Set("PUBLIC binary entry".into()),
                        ..Default::default()
                    },
                    &new_operation_id().unwrap(),
                )
                .unwrap()
                .value;
            service.start_edit_entry(&session, &entry).unwrap();
            let draft = service.active_draft(&session).unwrap().value.unwrap();
            let bytes: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
            service
                .import_attachment_stream(
                    &session,
                    &draft.draft,
                    bytes.as_slice(),
                    AttachmentImport {
                        length: bytes.len() as u64,
                        name: "PUBLIC binary.bin".into(),
                        replacement: None,
                    },
                    &new_operation_id().unwrap(),
                )
                .unwrap();
            service
                .save_draft(&session, &new_operation_id().unwrap())
                .unwrap();
        }
        let copy = host.runtime.register_working_copy(&path).unwrap();
        Self {
            directory,
            host,
            service,
            session,
            copy,
        }
    }
    fn capture(&self) -> Arc<EncryptedDatabaseCopy> {
        self.host
            .capture_database_copy(self.copy.clone().into())
            .unwrap()
    }
    fn lock(&mut self) {
        self.service.lock(&self.session).unwrap();
        self.host.runtime.close(&self.session.database).unwrap();
    }
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    Zero,
    Overlong,
    Write,
    FinishIo,
    FinishUncertain,
    CancelAtFinish,
}
struct Gate {
    entered: Mutex<Option<mpsc::Sender<()>>>,
    released: Mutex<bool>,
    changed: Condvar,
}
struct OutputState {
    bytes: Mutex<Vec<u8>>,
    writes: AtomicUsize,
    largest: AtomicUsize,
    finishes: AtomicUsize,
    closes: AtomicUsize,
    fault: Fault,
    owner: Mutex<Weak<EncryptedDatabaseCopy>>,
    gate: Option<Gate>,
}
#[derive(Clone)]
struct Output(Arc<OutputState>);
impl Output {
    fn new(owner: &Arc<EncryptedDatabaseCopy>, fault: Fault) -> Self {
        Self(Arc::new(OutputState {
            bytes: Mutex::new(Vec::new()),
            writes: AtomicUsize::new(0),
            largest: AtomicUsize::new(0),
            finishes: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
            fault,
            owner: Mutex::new(Arc::downgrade(owner)),
            gate: None,
        }))
    }
    fn gated(owner: &Arc<EncryptedDatabaseCopy>) -> (Self, mpsc::Receiver<()>) {
        let (entered, receiver) = mpsc::channel();
        let mut output = Self::new(owner, Fault::None);
        Arc::get_mut(&mut output.0).unwrap().gate = Some(Gate {
            entered: Mutex::new(Some(entered)),
            released: Mutex::new(false),
            changed: Condvar::new(),
        });
        (output, receiver)
    }
    fn run(&self, owner: &EncryptedDatabaseCopy) -> Result<(), AndroidError> {
        owner.export(Box::new(self.clone()))
    }
}
impl SelectedOutput for Output {
    fn write(&self, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError> {
        self.0.writes.fetch_add(1, Ordering::Relaxed);
        self.0.largest.fetch_max(bytes.len(), Ordering::Relaxed);
        assert!(bytes.len() <= crate::descriptors::TRANSFER_CHUNK);
        if let Some(gate) = &self.0.gate {
            if let Some(entered) = gate.entered.lock().unwrap().take() {
                entered.send(()).unwrap();
            }
            let released = gate.released.lock().unwrap();
            let (released, timeout) = gate
                .changed
                .wait_timeout_while(released, Duration::from_secs(10), |value| !*value)
                .unwrap();
            assert!(
                *released && !timeout.timed_out(),
                "bounded output gate was not released"
            );
        }
        match self.0.fault {
            Fault::Zero => return Ok(0),
            Fault::Overlong => return Ok(bytes.len() as u32 + 1),
            Fault::Write => return Err(AndroidError::ProfileIo),
            _ => {}
        }
        let mut stored = self.0.bytes.lock().unwrap();
        assert_eq!(offset, stored.len() as u64);
        // Legitimate partial writes must be retried at the exact next offset.
        let count = bytes.len().min(8191);
        stored.extend_from_slice(&bytes[..count]);
        Ok(count as u32)
    }
    fn finish(&self) -> Result<(), AndroidError> {
        self.0.finishes.fetch_add(1, Ordering::Relaxed);
        match self.0.fault {
            Fault::FinishIo => Err(AndroidError::StorageIo),
            Fault::FinishUncertain => Err(AndroidError::CommitUncertain),
            Fault::CancelAtFinish => {
                let owner = self.0.owner.lock().unwrap().upgrade().unwrap();
                owner.cancel();
                Ok(())
            }
            _ => Ok(()),
        }
    }
    fn close(&self) {
        self.0.closes.fetch_add(1, Ordering::Relaxed);
        if matches!(self.0.fault, Fault::CancelAtFinish) {
            assert!(
                self.0
                    .owner
                    .lock()
                    .unwrap()
                    .upgrade()
                    .unwrap()
                    .state
                    .try_lock()
                    .is_ok(),
                "output close retained the ownership mutex"
            );
        }
        if let Some(gate) = &self.0.gate {
            *gate.released.lock().unwrap() = true;
            gate.changed.notify_all();
        }
    }
}

#[test]
fn bounded_partial_writes_finish_exact_ciphertext_after_the_document_locks() {
    let mut fixture = Fixture::new(true);
    let bytes = fs::read(&fixture.copy.path).unwrap();
    assert!(bytes.len() > crate::descriptors::TRANSFER_CHUNK);
    let owner = fixture.capture();
    fixture.lock();
    let output = Output::new(&owner, Fault::None);
    output.run(&owner).unwrap();
    assert_eq!(*output.0.bytes.lock().unwrap(), bytes);
    assert_eq!(output.0.finishes.load(Ordering::Relaxed), 1);
    assert_eq!(output.0.closes.load(Ordering::Relaxed), 1);
    assert_eq!(
        output.0.largest.load(Ordering::Relaxed),
        crate::descriptors::TRANSFER_CHUNK
    );
    let identity = owner.identity();
    assert_eq!(identity.bytes, bytes.len() as u64);
    assert_eq!(identity.database, fixture.copy.database.to_string());
    let copied = fixture.directory.path().join("PUBLIC exported.taypeer");
    fs::write(&copied, &*output.0.bytes.lock().unwrap()).unwrap();
    let archive = taypeer_storage::ArchiveSnapshot::open(&copied, Some(fixture.copy.root)).unwrap();
    assert_eq!(archive.fingerprint().to_string(), identity.fingerprint);
}

#[test]
fn failed_output_never_acknowledges_and_an_explicit_new_output_uses_the_same_capture() {
    let fixture = Fixture::new(false);
    let owner = fixture.capture();
    let source = fs::read(&fixture.copy.path).unwrap();
    for fault in [
        Fault::Zero,
        Fault::Overlong,
        Fault::Write,
        Fault::FinishIo,
        Fault::FinishUncertain,
    ] {
        let output = Output::new(&owner, fault);
        let error = output.run(&owner).unwrap_err();
        assert!(match fault {
            Fault::Zero | Fault::FinishIo => matches!(error, AndroidError::StorageIo),
            Fault::Overlong => matches!(error, AndroidError::InvalidFile),
            Fault::Write => matches!(error, AndroidError::ProfileIo),
            Fault::FinishUncertain => matches!(error, AndroidError::CommitUncertain),
            _ => false,
        });
        let finishes = usize::from(matches!(fault, Fault::FinishIo | Fault::FinishUncertain));
        assert_eq!(output.0.finishes.load(Ordering::Relaxed), finishes);
        assert_eq!(output.0.closes.load(Ordering::Relaxed), 1);
        let retry = Output::new(&owner, Fault::None);
        retry.run(&owner).unwrap();
        assert_eq!(*retry.0.bytes.lock().unwrap(), source);
        assert_eq!(retry.0.finishes.load(Ordering::Relaxed), 1);
        assert_eq!(retry.0.closes.load(Ordering::Relaxed), 1);
    }
    assert_eq!(fs::read(&fixture.copy.path).unwrap(), source);
}

#[test]
fn cancellation_before_output_or_during_finish_is_irreversible_and_never_an_ack() {
    let fixture = Fixture::new(false);
    let owner = fixture.capture();
    owner.cancel();
    owner.cancel();
    let output = Output::new(&owner, Fault::None);
    assert!(matches!(
        output.run(&owner),
        Err(AndroidError::ExportCancelled)
    ));
    assert_eq!(output.0.writes.load(Ordering::Relaxed), 0);
    assert_eq!(output.0.finishes.load(Ordering::Relaxed), 0);
    assert_eq!(output.0.closes.load(Ordering::Relaxed), 1);
    let owner = fixture.capture();
    let output = Output::new(&owner, Fault::CancelAtFinish);
    assert!(matches!(
        output.run(&owner),
        Err(AndroidError::ExportCancelled)
    ));
    assert_eq!(output.0.finishes.load(Ordering::Relaxed), 1);
    assert_eq!(
        *output.0.bytes.lock().unwrap(),
        fs::read(&fixture.copy.path).unwrap()
    );
    let late = Output::new(&owner, Fault::None);
    assert!(matches!(
        late.run(&owner),
        Err(AndroidError::ExportCancelled)
    ));
    assert_eq!(late.0.writes.load(Ordering::Relaxed), 0);
    assert_eq!(late.0.finishes.load(Ordering::Relaxed), 0);
    assert_eq!(late.0.closes.load(Ordering::Relaxed), 1);
}

#[test]
fn concurrent_output_is_refused_and_cancellation_releases_gated_io_without_finish() {
    let fixture = Fixture::new(false);
    let owner = fixture.capture();
    let (output, entered) = Output::gated(&owner);
    std::thread::scope(|scope| {
        let pending = scope.spawn(|| output.run(&owner));
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let second = Output::new(&owner, Fault::None);
        assert!(matches!(second.run(&owner), Err(AndroidError::StorageBusy)));
        assert_eq!(second.0.writes.load(Ordering::Relaxed), 0);
        assert_eq!(second.0.closes.load(Ordering::Relaxed), 1);
        owner.cancel();
        assert!(matches!(
            pending.join().unwrap(),
            Err(AndroidError::ExportCancelled)
        ));
    });
    assert_eq!(output.0.finishes.load(Ordering::Relaxed), 0);
    assert!(output.0.closes.load(Ordering::Relaxed) >= 1);
    let retry = Output::new(&owner, Fault::None);
    assert!(matches!(
        retry.run(&owner),
        Err(AndroidError::ExportCancelled)
    ));
    assert_eq!(retry.0.writes.load(Ordering::Relaxed), 0);
}

#[test]
fn modified_or_truncated_retained_inode_is_not_finished() {
    let mut fixture = Fixture::new(false);
    fixture.lock();
    let original = fs::read(&fixture.copy.path).unwrap();
    let owner = fixture.capture();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(&fixture.copy.path)
        .unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[*original.last().unwrap() ^ 1]).unwrap();
    let modified = Output::new(&owner, Fault::None);
    assert!(matches!(
        modified.run(&owner),
        Err(AndroidError::StorageChanged)
    ));
    assert_eq!(modified.0.finishes.load(Ordering::Relaxed), 0);
    assert_eq!(modified.0.closes.load(Ordering::Relaxed), 1);
    file.set_len(original.len() as u64 - 1).unwrap();
    let truncated = Output::new(&owner, Fault::None);
    assert!(matches!(
        truncated.run(&owner),
        Err(AndroidError::StorageChanged)
    ));
    assert_eq!(truncated.0.writes.load(Ordering::Relaxed), 0);
    assert_eq!(truncated.0.finishes.load(Ordering::Relaxed), 0);
    assert_eq!(truncated.0.closes.load(Ordering::Relaxed), 1);
}
