//! PUBLIC ciphertext fixtures exercise the real host coordinator across the FFI reply boundary.
use super::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
};
use taypeer_runtime::{
    RuntimeHost,
    profile::{CredentialStore, ProfileError},
    session::SessionController,
};
use taypeer_storage::Error as StorageError;
use zeroize::Zeroizing;

#[derive(Default)]
struct Credentials(Mutex<BTreeMap<(String, String), Vec<u8>>>);
impl CredentialStore for Credentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned()
            .map(Zeroizing::new))
    }
    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), ProfileError> {
        self.0
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), value.to_vec());
        Ok(())
    }
}

#[derive(Default)]
struct Files {
    next: AtomicU64,
    bytes: Mutex<BTreeMap<u64, Vec<u8>>>,
    allocation_failure: AtomicBool,
    write_failure: AtomicBool,
    read_failure: AtomicBool,
}
impl CiphertextFiles for Files {
    fn allocate(&self) -> Result<u64, AndroidError> {
        if self.allocation_failure.load(Ordering::Relaxed) {
            return Err(AndroidError::StorageIo);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.bytes.lock().unwrap().insert(id, Vec::new());
        Ok(id)
    }
    fn length(&self, file: u64) -> Result<u64, AndroidError> {
        self.bytes
            .lock()
            .unwrap()
            .get(&file)
            .map(|bytes| bytes.len() as u64)
            .ok_or(AndroidError::InvalidFile)
    }
    fn read(&self, file: u64, offset: u64, count: u32) -> Result<Vec<u8>, AndroidError> {
        if self.read_failure.load(Ordering::Relaxed) {
            return Err(AndroidError::StorageIo);
        }
        assert!(count as usize <= descriptors::TRANSFER_CHUNK);
        let files = self.bytes.lock().unwrap();
        let bytes = files.get(&file).ok_or(AndroidError::InvalidFile)?;
        let start = (offset as usize).min(bytes.len());
        Ok(bytes[start..(start + count as usize).min(bytes.len())].to_vec())
    }
    fn write(&self, file: u64, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError> {
        if self.write_failure.load(Ordering::Relaxed) {
            return Err(AndroidError::StorageIo);
        }
        assert!(bytes.len() <= descriptors::TRANSFER_CHUNK);
        let mut files = self.bytes.lock().unwrap();
        let output = files.get_mut(&file).ok_or(AndroidError::InvalidFile)?;
        let start = offset as usize;
        output.resize(output.len().max(start + bytes.len()), 0);
        output[start..start + bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len() as u32)
    }
    fn release(&self, file: u64) {
        self.bytes.lock().unwrap().remove(&file);
    }
}

struct Fixture {
    host: RuntimeHost,
    writer: Arc<CipherWriter>,
    files: Arc<Files>,
    directory: PathBuf,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "taypeer-PUBLIC-cipher-reply-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("PUBLIC ciphertext.taypeer");
        let host = RuntimeHost::with_platform_credentials(
            &directory.join("profile"),
            SessionController::new(Default::default()),
            Arc::new(Credentials::default()),
        )
        .unwrap();
        let platform = host.platform_cipher_writer(&path).unwrap();
        let author = platform.author(None).unwrap();
        let identity =
            taypeer_trust::Identity::new(author.public(), platform.transport_public()).unwrap();
        let seed = taypeer_services::DatabaseService::prepare_managed(
            "PUBLIC reply uncertainty".into(),
            b"PUBLIC synthetic password",
            &author,
            identity,
            1,
            taypeer_core::DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap(),
        )
        .unwrap();
        platform.create(seed).unwrap();
        let files = Arc::new(Files::default());
        let writer = Arc::new(CipherWriter {
            writer: platform,
            files: files.clone(),
        });
        Self {
            host,
            writer,
            files,
            directory,
            path,
        }
    }
    fn snapshot(&self) -> ArchiveSnapshot {
        ArchiveSnapshot::open(&self.path, None).unwrap()
    }
    fn request(&self) -> PreparedCommit {
        let snapshot = self.snapshot();
        let metadata = snapshot.metadata();
        PreparedCommit {
            expected: snapshot.fingerprint(),
            control: snapshot.chain().head_hash().unwrap(),
            controls: metadata.controls.clone(),
            objects: Vec::new(),
            remove: Default::default(),
            checkpoint: metadata.manifest.body.checkpoint,
            baseline: metadata.manifest.body.baseline,
            journal: metadata.journal.clone(),
        }
    }
    fn remote(&self, port: Arc<dyn DocumentPersistence>) -> RemoteDocument {
        RemoteDocument {
            port,
            files: self.files.clone(),
            copy: Mutex::new(None),
            selected: Arc::new(UnusedSelection),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // The retained host owns its profile and writer until this fixture drops.
        self.host
            .sessions()
            .lock_all(taypeer_runtime::session::LockReason::Manual);
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn encoded(request: PreparedCommit) -> CipherCommit {
    CipherCommit {
        expected: request.expected.to_string(),
        control: request.control.to_string(),
        controls: encode(&request.controls).unwrap(),
        objects: Vec::new(),
        remove: Vec::new(),
        checkpoint: request.checkpoint.to_string(),
        baseline: request.baseline.to_string(),
        journal: encode(&request.journal).unwrap(),
    }
}

struct UnusedSelection;
impl crate::document::SelectedTransfersRemote for UnusedSelection {
    fn read(&self, _: u64, _: u64, _: u32) -> Result<Vec<u8>, AndroidError> {
        panic!("Ciphertext tests never acquire selected plaintext")
    }
    fn write(&self, _: u64, _: u64, _: Vec<u8>) -> Result<u32, AndroidError> {
        panic!("Ciphertext tests never acquire selected plaintext")
    }
    fn finish(&self, _: u64) -> Result<(), AndroidError> {
        panic!("Ciphertext tests never acquire selected plaintext")
    }
}

const VALID: u8 = 0;
const INVALID_REPLY: u8 = 1;
const UNREADABLE_REPLY: u8 = 2;
const REJECT_CHANGED: u8 = 3;
const REJECT_IO: u8 = 4;
const REJECT_INVALID: u8 = 5;

struct Port {
    writer: Arc<CipherWriter>,
    files: Arc<Files>,
    mode: AtomicU8,
}
impl Port {
    fn new(fixture: &Fixture) -> Arc<Self> {
        Arc::new(Self {
            writer: Arc::clone(&fixture.writer),
            files: Arc::clone(&fixture.files),
            mode: AtomicU8::new(VALID),
        })
    }
    fn rejection(&self) -> Result<(), AndroidError> {
        match self.mode.load(Ordering::Relaxed) {
            REJECT_CHANGED => Err(AndroidError::StorageChanged),
            REJECT_IO => Err(AndroidError::StorageIo),
            REJECT_INVALID => Err(AndroidError::InvalidFile),
            _ => Ok(()),
        }
    }
    fn reply(&self, mut snapshot: CipherSnapshot) -> CipherSnapshot {
        match self.mode.load(Ordering::Relaxed) {
            INVALID_REPLY => snapshot.fingerprint = "PUBLIC malformed digest".into(),
            UNREADABLE_REPLY => self.files.read_failure.store(true, Ordering::Relaxed),
            _ => {}
        }
        snapshot
    }
}
impl DocumentPersistence for Port {
    fn snapshot(&self) -> Result<CipherSnapshot, AndroidError> {
        self.rejection()?;
        Ok(self.reply(self.writer.snapshot()?))
    }
    fn commit(&self, request: CipherCommit) -> Result<CipherSnapshot, AndroidError> {
        self.rejection()?;
        Ok(self.reply(self.writer.commit(request)?))
    }
    fn create(&self, _: CipherSeed) -> Result<(), AndroidError> {
        panic!("Fixture genesis is already published")
    }
    fn author(&self, _: Option<String>) -> Result<Vec<u8>, AndroidError> {
        panic!("Ciphertext reply tests never request author credentials")
    }
    fn transport_public(&self) -> Result<String, AndroidError> {
        Ok(self.writer.transport_public())
    }
    fn save_draft(&self, _: u64) -> Result<(), AndroidError> {
        panic!("Ciphertext reply tests never mutate local forms")
    }
    fn load_draft(&self) -> Result<Option<u64>, AndroidError> {
        panic!("Ciphertext reply tests never load local forms")
    }
    fn discard_draft(&self) -> Result<(), AndroidError> {
        panic!("Ciphertext reply tests never discard local forms")
    }
}

#[test]
fn host_commit_reply_allocation_or_write_failure_is_uncertain_after_durable_publication() {
    let fixture = Fixture::new();
    for fail_write in [false, true] {
        let before = fixture.snapshot();
        let request = fixture.request();
        fixture
            .files
            .allocation_failure
            .store(!fail_write, Ordering::Relaxed);
        fixture
            .files
            .write_failure
            .store(fail_write, Ordering::Relaxed);
        assert!(matches!(
            fixture.writer.snapshot(),
            Err(AndroidError::StorageIo) | Err(AndroidError::Runtime)
        ));
        assert!(matches!(
            fixture.writer.commit(encoded(request)),
            Err(AndroidError::CommitUncertain)
        ));
        let after = fixture.snapshot();
        assert_ne!(after.fingerprint(), before.fingerprint());
        assert_eq!(
            after.metadata().manifest.body.generation,
            before.metadata().manifest.body.generation + 1
        );
        assert!(
            fixture.files.bytes.lock().unwrap().is_empty(),
            "Failed reply must release its ciphertext lease"
        );
        fixture
            .files
            .allocation_failure
            .store(false, Ordering::Relaxed);
        fixture.files.write_failure.store(false, Ordering::Relaxed);
    }

    let before = fixture.snapshot();
    let mut stale = fixture.request();
    stale.expected = Digest::of(b"PUBLIC stale generation");
    fixture
        .files
        .allocation_failure
        .store(true, Ordering::Relaxed);
    assert!(matches!(
        fixture.writer.commit(encoded(stale)),
        Err(AndroidError::StorageChanged)
    ));
    let mut invalid = encoded(fixture.request());
    invalid.controls = b"PUBLIC malformed controls".to_vec();
    assert!(matches!(
        fixture.writer.commit(invalid),
        Err(AndroidError::InvalidFile)
    ));
    assert_eq!(fixture.snapshot().fingerprint(), before.fingerprint());
}

#[test]
fn remote_commit_invalid_or_unreadable_success_reply_is_uncertain_and_reconcilable() {
    let fixture = Fixture::new();
    let port = Port::new(&fixture);
    let remote = fixture.remote(port.clone());
    for mode in [INVALID_REPLY, UNREADABLE_REPLY] {
        let before = fixture.snapshot();
        port.mode.store(mode, Ordering::Relaxed);
        assert!(matches!(
            remote.commit(fixture.request()),
            Err(StorageError::CommitUncertain)
        ));
        let after = fixture.snapshot();
        assert_ne!(after.fingerprint(), before.fingerprint());
        assert_eq!(
            after.metadata().manifest.body.generation,
            before.metadata().manifest.body.generation + 1
        );
        assert!(
            fixture.files.bytes.lock().unwrap().is_empty(),
            "Rejected reply must release its incoming lease"
        );
        fixture.files.read_failure.store(false, Ordering::Relaxed);
        port.mode.store(VALID, Ordering::Relaxed);
        assert_eq!(
            remote.snapshot().unwrap().fingerprint(),
            after.fingerprint()
        );
        assert_eq!(
            remote.working_copy(),
            fixture.writer.writer.working_copy().unwrap()
        );
    }
}

#[test]
fn readonly_reply_and_explicit_precommit_rejections_preserve_their_categories() {
    let fixture = Fixture::new();
    let port = Port::new(&fixture);
    let remote = fixture.remote(port.clone());
    let before = fixture.snapshot().fingerprint();
    for (mode, expected) in [
        (REJECT_CHANGED, StorageError::Changed),
        (REJECT_IO, StorageError::Io),
        (REJECT_INVALID, StorageError::InvalidFile),
    ] {
        port.mode.store(mode, Ordering::Relaxed);
        assert_eq!(remote.commit(fixture.request()).err(), Some(expected));
        assert_eq!(fixture.snapshot().fingerprint(), before);
    }
    port.mode.store(INVALID_REPLY, Ordering::Relaxed);
    assert!(matches!(remote.snapshot(), Err(StorageError::InvalidFile)));
    port.mode.store(UNREADABLE_REPLY, Ordering::Relaxed);
    assert!(matches!(remote.snapshot(), Err(StorageError::Io)));
    assert_eq!(fixture.snapshot().fingerprint(), before);
    assert!(fixture.files.bytes.lock().unwrap().is_empty());
}
