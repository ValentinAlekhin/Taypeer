//! Real transport regressions for host-owned ciphertext staging and failure cleanup.
use super::*;
use std::{
    fs::File,
    io,
    path::PathBuf,
    sync::{
        Condvar,
        atomic::{AtomicUsize, Ordering},
    },
};
use taypeer_storage::{CiphertextIo, TemporaryFileProvider};

#[derive(Default)]
struct Allocations {
    attempts: AtomicUsize,
    created: AtomicUsize,
    released: AtomicUsize,
    written: AtomicUsize,
    largest_write: AtomicUsize,
    changed: tokio::sync::Notify,
}
impl Allocations {
    async fn all_released(&self) {
        timeout(Duration::from_secs(5), async {
            loop {
                let changed = self.changed.notified();
                if self.released.load(Ordering::Acquire) == self.created.load(Ordering::Acquire) {
                    break;
                }
                changed.await;
            }
        })
        .await
        .expect("ciphertext capabilities must be released after completion/failure");
    }
}
#[derive(Clone, Default)]
enum AllocationBehavior {
    #[default]
    Allowed,
    Denied,
    WriteFailure,
    Paused(Arc<WriteGate>),
}
#[derive(Default)]
struct WriteGate {
    opened: Mutex<bool>,
    ready: Condvar,
    entered: tokio::sync::Notify,
}
impl WriteGate {
    fn wait(&self) {
        let mut opened = self.opened.lock().unwrap();
        self.entered.notify_one();
        while !*opened {
            opened = self.ready.wait(opened).unwrap();
        }
    }
}
struct ReleaseWriteGate(Arc<WriteGate>);
impl Drop for ReleaseWriteGate {
    fn drop(&mut self) {
        *self.0.opened.lock().unwrap() = true;
        self.0.ready.notify_all();
    }
}
struct RecordingStorage {
    directory: PathBuf,
    allocations: Arc<Allocations>,
    behavior: AllocationBehavior,
}
impl TemporaryFileProvider for RecordingStorage {
    fn create(&self) -> io::Result<CiphertextFile> {
        self.allocations.attempts.fetch_add(1, Ordering::AcqRel);
        if matches!(self.behavior, AllocationBehavior::Denied) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let file = tempfile::tempfile_in(&self.directory)?;
        self.allocations.created.fetch_add(1, Ordering::AcqRel);
        Ok(CiphertextFile::new(Arc::new(RecordingFile {
            file,
            allocations: Arc::clone(&self.allocations),
            behavior: self.behavior.clone(),
        })))
    }
}
struct RecordingFile {
    file: File,
    allocations: Arc<Allocations>,
    behavior: AllocationBehavior,
}
impl CiphertextIo for RecordingFile {
    fn length(&self) -> io::Result<u64> {
        CiphertextIo::length(&self.file)
    }
    fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        CiphertextIo::read_at(&self.file, bytes, offset)
    }
    fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
        match &self.behavior {
            AllocationBehavior::WriteFailure => return Err(io::ErrorKind::PermissionDenied.into()),
            AllocationBehavior::Paused(gate) => gate.wait(),
            AllocationBehavior::Allowed | AllocationBehavior::Denied => {}
        }
        let count = CiphertextIo::write_at(&self.file, bytes, offset)?;
        self.allocations.written.fetch_add(count, Ordering::AcqRel);
        self.allocations
            .largest_write
            .fetch_max(bytes.len(), Ordering::AcqRel);
        Ok(count)
    }
}
impl Drop for RecordingFile {
    fn drop(&mut self) {
        self.allocations.released.fetch_add(1, Ordering::AcqRel);
        self.allocations.changed.notify_one();
    }
}
fn recording_storage(
    fixture: &PublicFixture,
    behavior: AllocationBehavior,
) -> (TemporaryStorage, Arc<Allocations>) {
    let allocations = Arc::new(Allocations::default());
    let temporary = TemporaryStorage::new(Arc::new(RecordingStorage {
        directory: fixture.directory.path().to_owned(),
        allocations: Arc::clone(&allocations),
        behavior,
    }));
    (temporary, allocations)
}
fn verification_storage(fixture: &PublicFixture, index: usize, temporary: TemporaryStorage) {
    fixture.coordinators[index]
        .unregister(&fixture.database)
        .unwrap();
    let store = ArchiveStore::open(
        &fixture
            .directory
            .path()
            .join(format!("PUBLIC-{index}.taypeer")),
        None,
        None,
    )
    .unwrap()
    .with_temporary_storage(temporary);
    fixture.coordinators[index].register(store).unwrap();
}
async fn explicit_node(fixture: &PublicFixture, index: usize, temporary: TemporaryStorage) -> Node {
    Node::bind(
        &fixture.keys[index],
        &RelaySetting::Disabled,
        fixture.coordinators[index].clone(),
        temporary,
    )
    .await
    .unwrap()
}
fn blob_descriptor(fixture: &PublicFixture) -> CipherObject {
    fixture.coordinators[0]
        .snapshot(&fixture.database)
        .unwrap()
        .metadata()
        .manifest
        .body
        .objects[&fixture.blob]
        .clone()
}
async fn announce(fixture: &PublicFixture, sender: &Node, recipient: &Node) {
    let metadata = fixture.coordinators[0]
        .snapshot(&fixture.database)
        .unwrap()
        .metadata()
        .clone();
    let reply = sender
        .request(recipient.address(), Command::Offer(Box::new(metadata)))
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Needed(ids) if ids.contains(&fixture.blob)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn put_uses_transport_and_snapshot_allocators_before_a_durable_receipt() {
    let f = fixture();
    let (source_storage, source) = recording_storage(&f, AllocationBehavior::Allowed);
    let (transport_storage, transport) = recording_storage(&f, AllocationBehavior::Allowed);
    let (verification, verified) = recording_storage(&f, AllocationBehavior::Allowed);
    verification_storage(&f, 1, verification);
    let a = explicit_node(&f, 0, source_storage).await;
    let b = explicit_node(&f, 1, transport_storage).await;
    let descriptor = blob_descriptor(&f);
    let report = a.exchange(b.address(), f.database.clone()).await.unwrap();
    assert_eq!(report.sent, 1);
    assert_eq!(report.received, 0);
    assert_eq!(transport.attempts.load(Ordering::Acquire), 1);
    assert_eq!(verified.attempts.load(Ordering::Acquire), 1);
    assert_eq!(
        transport.written.load(Ordering::Acquire) as u64,
        descriptor.length
    );
    assert!(transport.largest_write.load(Ordering::Acquire) <= wire::CHUNK);
    transport.all_released().await;
    verified.all_released().await;
    assert_eq!(source.attempts.load(Ordering::Acquire), 0);
    let durable = f.coordinators[1].snapshot(&f.database).unwrap();
    assert!(
        durable
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&f.blob)
    );
    let again = a.exchange(b.address(), f.database.clone()).await.unwrap();
    assert_eq!(again, ExchangeReport::default());
    assert_eq!(transport.attempts.load(Ordering::Acquire), 1);
    assert_eq!(verified.attempts.load(Ordering::Acquire), 1);
    // The verification policy survives the received generation's durable commit.
    drop(durable.object(f.blob).unwrap());
    assert_eq!(verified.attempts.load(Ordering::Acquire), 2);
    verified.all_released().await;
    a.close().await;
    b.close().await;
    f.coordinators[1].unregister(&f.database).unwrap();
    let reopened =
        taypeer_storage::ArchiveSnapshot::open(&f.directory.path().join("PUBLIC-1.taypeer"), None)
            .unwrap();
    assert!(
        reopened
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&f.blob)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initial_download_returns_immediately_readable_host_allocated_ciphertext() {
    let f = fixture();
    let a = test_node(&f.keys[0], f.coordinators[0].clone(), None).await;
    let (storage, allocations) = recording_storage(&f, AllocationBehavior::Allowed);
    let b = explicit_node(&f, 1, storage).await;
    let before = f.coordinators[1]
        .snapshot(&f.database)
        .unwrap()
        .fingerprint();
    let descriptor = blob_descriptor(&f);
    let mut incoming = b
        .download_object(a.address(), f.database.clone(), descriptor.clone())
        .await
        .unwrap();
    let mut bytes = Vec::new();
    incoming.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes.len() as u64, descriptor.length);
    assert!(
        Digest::of(&bytes) == descriptor.digest,
        "received ciphertext must match its descriptor"
    );
    assert_eq!(allocations.attempts.load(Ordering::Acquire), 1);
    assert_eq!(allocations.released.load(Ordering::Acquire), 0);
    assert!(allocations.largest_write.load(Ordering::Acquire) <= wire::CHUNK);
    assert_eq!(
        before,
        f.coordinators[1]
            .snapshot(&f.database)
            .unwrap()
            .fingerprint()
    );
    drop(incoming);
    allocations.all_released().await;
    a.close().await;
    b.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initial_download_never_falls_back_after_allocation_or_write_failure() {
    for behavior in [AllocationBehavior::Denied, AllocationBehavior::WriteFailure] {
        let f = fixture();
        let a = test_node(&f.keys[0], f.coordinators[0].clone(), None).await;
        let (storage, allocations) = recording_storage(&f, behavior);
        let b = explicit_node(&f, 1, storage).await;
        let before = f.coordinators[1]
            .snapshot(&f.database)
            .unwrap()
            .fingerprint();
        let result = b
            .download_object(a.address(), f.database.clone(), blob_descriptor(&f))
            .await;
        assert!(matches!(
            result,
            Err(Error::Storage(taypeer_storage::Error::Io))
        ));
        assert_eq!(allocations.attempts.load(Ordering::Acquire), 1);
        allocations.all_released().await;
        assert_eq!(
            before,
            f.coordinators[1]
                .snapshot(&f.database)
                .unwrap()
                .fingerprint()
        );
        a.close().await;
        b.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_put_staging_never_publishes_or_acknowledges_ciphertext() {
    for behavior in [AllocationBehavior::Denied, AllocationBehavior::WriteFailure] {
        let f = fixture();
        let a = test_node(&f.keys[0], f.coordinators[0].clone(), None).await;
        let (storage, allocations) = recording_storage(&f, behavior);
        let (verification, verified) = recording_storage(&f, AllocationBehavior::Allowed);
        verification_storage(&f, 1, verification);
        let b = explicit_node(&f, 1, storage).await;
        announce(&f, &a, &b).await;
        let before = f.coordinators[1]
            .snapshot(&f.database)
            .unwrap()
            .fingerprint();
        assert!(
            a.exchange(b.address(), f.database.clone()).await.is_err(),
            "failed staging must not receive a success receipt"
        );
        assert_eq!(allocations.attempts.load(Ordering::Acquire), 1);
        allocations.all_released().await;
        assert_eq!(verified.attempts.load(Ordering::Acquire), 0);
        let after = f.coordinators[1].snapshot(&f.database).unwrap();
        assert_eq!(before, after.fingerprint());
        assert!(!after.metadata().manifest.body.objects.contains_key(&f.blob));
        a.close().await;
        b.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_snapshot_allocation_has_no_successful_durable_receipt() {
    let f = fixture();
    let a = test_node(&f.keys[0], f.coordinators[0].clone(), None).await;
    let (storage, staged) = recording_storage(&f, AllocationBehavior::Allowed);
    let (verification, verified) = recording_storage(&f, AllocationBehavior::Denied);
    verification_storage(&f, 1, verification);
    let b = explicit_node(&f, 1, storage).await;
    announce(&f, &a, &b).await;
    let before = f.coordinators[1]
        .snapshot(&f.database)
        .unwrap()
        .fingerprint();
    assert_eq!(
        a.exchange(b.address(), f.database.clone()).await,
        Err(Error::Storage(taypeer_storage::Error::Io))
    );
    assert_eq!(staged.attempts.load(Ordering::Acquire), 1);
    assert_eq!(verified.attempts.load(Ordering::Acquire), 1);
    staged.all_released().await;
    let after = f.coordinators[1].snapshot(&f.database).unwrap();
    assert_eq!(before, after.fingerprint());
    assert!(!after.metadata().manifest.body.objects.contains_key(&f.blob));
    a.close().await;
    b.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_or_unauthorized_put_drops_staging_without_a_receipt() {
    let f = fixture();
    let a = test_node(&f.keys[0], f.coordinators[0].clone(), None).await;
    let (storage, staged) = recording_storage(&f, AllocationBehavior::Allowed);
    let (verification, verified) = recording_storage(&f, AllocationBehavior::Allowed);
    verification_storage(&f, 1, verification);
    let b = explicit_node(&f, 1, storage).await;
    let descriptor = blob_descriptor(&f);
    // An admitted peer still cannot allocate staging for an unannounced object.
    let (_, reader) = f.coordinators[0]
        .object(f.keys[1].public(), &f.database, f.blob)
        .unwrap();
    let connection = a.connect(b.address()).await.unwrap();
    assert!(
        put(&connection, f.database.clone(), descriptor.clone(), reader)
            .await
            .is_err()
    );
    connection.close(0_u8.into(), b"test complete");
    assert_eq!(staged.attempts.load(Ordering::Acquire), 0);
    announce(&f, &a, &b).await;
    let before = f.coordinators[1]
        .snapshot(&f.database)
        .unwrap()
        .fingerprint();
    let (_, mut reader) = f.coordinators[0]
        .object(f.keys[1].public(), &f.database, f.blob)
        .unwrap();
    let mut ciphertext = Vec::new();
    reader.read_to_end(&mut ciphertext).unwrap();
    for extra_byte in [false, true] {
        let connection = a.connect(b.address()).await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        wire::write(
            &mut send,
            &Request::Put {
                database: f.database.clone(),
                descriptor: descriptor.clone(),
            },
        )
        .await
        .unwrap();
        let body = if extra_byte {
            ciphertext.as_slice()
        } else {
            &ciphertext[..10]
        };
        send.write_all(body).await.unwrap();
        if extra_byte {
            send.write_all(&[0]).await.unwrap();
        }
        send.finish().unwrap();
        let receipt = wire::read::<Result<Digest, Error>>(&mut recv).await;
        assert!(
            !matches!(receipt, Ok(Ok(_))),
            "truncated/overlong object must not receive a success receipt"
        );
        connection.close(0_u8.into(), b"test complete");
        staged.all_released().await;
        assert_eq!(
            before,
            f.coordinators[1]
                .snapshot(&f.database)
                .unwrap()
                .fingerprint()
        );
    }
    assert_eq!(staged.attempts.load(Ordering::Acquire), 2);
    assert_eq!(verified.attempts.load(Ordering::Acquire), 0);
    a.close().await;
    b.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_download_releases_its_bounded_writer_capability() {
    let f = fixture();
    let a = test_node(&f.keys[0], f.coordinators[0].clone(), None).await;
    let gate = Arc::new(WriteGate::default());
    let release = ReleaseWriteGate(Arc::clone(&gate));
    let (storage, allocations) =
        recording_storage(&f, AllocationBehavior::Paused(Arc::clone(&gate)));
    let b = Arc::new(explicit_node(&f, 1, storage).await);
    let recipient = Arc::clone(&b);
    let address = a.address();
    let database = f.database.clone();
    let descriptor = blob_descriptor(&f);
    let download = tokio::spawn(async move {
        recipient
            .download_object(address, database, descriptor)
            .await
    });
    timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    download.abort();
    assert!(matches!(download.await, Err(error) if error.is_cancelled()));
    // Even on a failed assertion, the guard lets the blocking writer terminate.
    drop(release);
    allocations.all_released().await;
    assert_eq!(allocations.created.load(Ordering::Acquire), 1);
    assert_eq!(allocations.released.load(Ordering::Acquire), 1);
    assert!(
        !f.coordinators[1]
            .snapshot(&f.database)
            .unwrap()
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&f.blob)
    );
    a.close().await;
    b.close().await;
}
