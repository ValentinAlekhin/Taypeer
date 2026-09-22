//! PUBLIC corpus and artificial bytes only. No process-global temp overrides.
use crate::{ArchiveSnapshot, BlobStore, Error, TemporaryFileProvider, TemporaryStorage};
use std::{
    fs::File,
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct Files {
    directory: tempfile::TempDir,
    denied: AtomicBool,
    allocations: AtomicUsize,
}
impl Files {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            directory: tempfile::tempdir().unwrap(),
            denied: AtomicBool::new(false),
            allocations: AtomicUsize::new(0),
        })
    }
    fn policy(self: &Arc<Self>) -> TemporaryStorage {
        TemporaryStorage::new(self.clone())
    }
}
impl TemporaryFileProvider for Files {
    fn create(&self) -> io::Result<crate::CiphertextFile> {
        if self.denied.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        self.allocations.fetch_add(1, Ordering::SeqCst);
        tempfile::tempfile_in(self.directory.path()).map(crate::CiphertextFile::from)
    }
}

#[test]
fn snapshot_survives_unlink_and_uses_only_its_explicit_allocator() {
    let files = Files::new();
    let source = files.directory.path().join("PUBLIC-copy.taypeer");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/dev5/populated.taypeer"
        ),
        &source,
    )
    .unwrap();
    let snapshot =
        ArchiveSnapshot::from_file(File::open(&source).unwrap(), None, files.policy()).unwrap();
    std::fs::remove_file(source).unwrap();
    let id = snapshot.metadata().manifest.body.checkpoint;
    let object = snapshot.object(id).unwrap();
    let mut first = object.reader().unwrap();
    let mut second = object.reader().unwrap();
    let mut prefix = [0; 11];
    first.read_exact(&mut prefix).unwrap();
    let mut expected = Vec::new();
    second.read_to_end(&mut expected).unwrap();
    let mut rest = Vec::new();
    first.read_to_end(&mut rest).unwrap();
    assert_eq!([prefix.as_slice(), rest.as_slice()].concat(), expected);
    assert_eq!(files.allocations.load(Ordering::SeqCst), 1);
    files.denied.store(true, Ordering::SeqCst);
    assert_eq!(snapshot.object(id).unwrap_err(), Error::Io);
    // Existing immutable objects remain usable after a later allocation failure.
    let key = object.unlock_key(b"PUBLIC_SESSION_DRAFT_PASSWORD").unwrap();
    let (document, _) = object.unlock_bundle(&key).unwrap();
    assert!(!document.is_empty());
}

#[test]
fn blob_readers_have_independent_offsets_and_allocation_failure_preserves_content() {
    let files = Files::new();
    let mut blobs = BlobStore::with_temporary_storage(files.policy()).unwrap();
    let public = b"PUBLIC descriptor-backed binary content";
    let id = blobs
        .insert(public.as_slice(), public.len() as u64, 1024)
        .unwrap();
    let mut first = blobs.reader(&id).unwrap();
    let mut prefix = [0; 3];
    first.read_exact(&mut prefix).unwrap();
    let mut second = Vec::new();
    blobs.reader(&id).unwrap().read_to_end(&mut second).unwrap();
    let mut rest = Vec::new();
    first.read_to_end(&mut rest).unwrap();
    assert_eq!(second, public);
    assert_eq!([prefix.as_slice(), rest.as_slice()].concat(), public);
    drop(first);
    files.denied.store(true, Ordering::SeqCst);
    assert_eq!(
        blobs.insert(public.as_slice(), public.len() as u64, 1024),
        Err(Error::Io)
    );
    assert_eq!(blobs.ids().count(), 1);
}
