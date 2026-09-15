use super::*;
use crate::{BlobStore, ReadKey, create_epoch};
use std::{
    fs,
    io::Read,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use taypeer_core::DatabaseId;
use taypeer_trust::{AuthorKey, ControlTransition, Identity, ObjectKind};

const PASSWORD: &[u8] = b"PUBLIC archive password";
struct Fixture {
    author: AuthorKey,
    transport: TransportKey,
    chain: ControlChain,
    header: Vec<u8>,
    key: ReadKey,
}
impl Fixture {
    fn new() -> Self {
        let author = AuthorKey::from_seed(&[41; 32]);
        let transport = TransportKey::from_seed(&[42; 32]);
        let identity = Identity::new(author.public(), transport.public()).unwrap();
        let chain = ControlChain::genesis(
            DatabaseId::new("PUBLIC archive"),
            identity,
            &author,
            Digest::of(b"PUBLIC policy"),
            4,
        )
        .unwrap();
        let (header, key) = create_epoch(PASSWORD, 500).unwrap();
        Self {
            author,
            transport,
            chain,
            header,
            key,
        }
    }
    fn object(&self, kind: ObjectKind, bytes: &[u8]) -> EncryptedObject {
        EncryptedObject::seal(
            &self.chain,
            &self.author,
            kind,
            &self.header,
            &self.key,
            bytes,
            bytes.len() as u64,
        )
        .unwrap()
    }
    fn initial(&self) -> (ArchiveCandidate, ArchiveMetadata) {
        let mut candidate = ArchiveCandidate::new();
        let checkpoint = candidate
            .insert(self.object(ObjectKind::Checkpoint, b"PUBLIC checkpoint"))
            .unwrap();
        let baseline = candidate
            .insert(self.object(ObjectKind::Baseline, b"PUBLIC baseline"))
            .unwrap();
        let body = Manifest {
            version: 1,
            database: self.chain.head().database.clone(),
            trust_set: self.chain.head().trust_set,
            control: self.chain.head_hash().unwrap(),
            generation: 0,
            signer: self.author.device_id(),
            objects: BTreeMap::new(),
            checkpoint,
            baseline,
            auxiliary: Digest::of(&[]),
        };
        let metadata = candidate
            .metadata(
                &self.chain,
                &self.transport,
                body,
                ArchiveJournal::default(),
            )
            .unwrap();
        (candidate, metadata)
    }
}
#[derive(Default)]
struct Marker {
    value: Mutex<Option<Anchor>>,
    fail_final: AtomicBool,
    fail_prepared: AtomicBool,
}
impl AnchorStore for Marker {
    fn load(&self) -> Result<Option<Anchor>, Error> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn save(&self, anchor: &Anchor) -> Result<(), Error> {
        if anchor.prepared.is_some() && self.fail_prepared.swap(false, Ordering::SeqCst) {
            return Err(Error::Io);
        }
        if anchor.prepared.is_none() && self.fail_final.swap(false, Ordering::SeqCst) {
            return Err(Error::Io);
        }
        *self.value.lock().unwrap() = Some(anchor.clone());
        Ok(())
    }
}

#[test]
fn locked_writer_receives_ciphertext_and_reopens_with_a_complete_signed_inventory() {
    let f = Fixture::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let (candidate, metadata) = f.initial();
    let mut store = ArchiveStore::create(&path, &candidate, metadata, None).unwrap();
    assert!(matches!(
        ArchiveStore::open(&path, None, None),
        Err(Error::Busy)
    ));
    let before = store.snapshot().clone();
    let bytes = vec![b'P'; 2 * 1024 * 1024 + 7];
    let blob = f.object(ObjectKind::Blob, &bytes);
    let incoming =
        EncryptedObject::receive(blob.reader().unwrap(), blob.descriptor(), &f.chain).unwrap();
    // The receive/commit path is supplied no password or read key.
    let mut candidate = before.candidate();
    let id = candidate.insert(incoming.clone()).unwrap();
    assert_eq!(candidate.insert(incoming).unwrap(), id);
    let mut body = before.metadata().manifest.body.clone();
    body.generation += 1;
    let metadata = candidate
        .metadata(&f.chain, &f.transport, body, ArchiveJournal::default())
        .unwrap();
    store
        .commit(before.fingerprint(), &candidate, metadata)
        .unwrap();
    assert_eq!(before.metadata().manifest.body.objects.len(), 2);
    let mut old = before
        .reader(before.metadata().manifest.body.checkpoint)
        .unwrap();
    let mut old_bytes = Vec::new();
    old.read_to_end(&mut old_bytes).unwrap();
    assert!(
        !old_bytes
            .windows(b"PUBLIC checkpoint".len())
            .any(|w| w == b"PUBLIC checkpoint")
    );
    drop(store);
    let reopened = ArchiveStore::open(&path, Some(f.chain.root().unwrap()), None).unwrap();
    let object = reopened.snapshot().object(id).unwrap();
    assert!(object.unlock_key(b"PUBLIC wrong password").is_err());
    let key = object.unlock_key(PASSWORD).unwrap();
    let mut clear = Vec::new();
    object.decrypt(&key, &mut clear).unwrap();
    assert_eq!(clear, bytes);
    assert_eq!(
        reopened.snapshot().metadata().manifest.body.objects.len(),
        3
    );
    assert!(!crate::file::sibling(&path, ".backups").exists());
}

#[test]
fn interrupted_final_marker_reconciles_but_old_file_rollback_is_rejected() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("PUBLIC.taypeer");
    let marker = Arc::new(Marker::default());
    let (candidate, metadata) = f.initial();
    let mut store =
        ArchiveStore::create(&path, &candidate, metadata, Some(marker.clone())).unwrap();
    let original = fs::read(&path).unwrap();
    let previous = store.snapshot().fingerprint();
    let mut next = store.snapshot().metadata().manifest.body.clone();
    next.generation += 1;
    let candidate = store.snapshot().candidate();
    let metadata = candidate
        .metadata(&f.chain, &f.transport, next, ArchiveJournal::default())
        .unwrap();
    marker.fail_prepared.store(true, Ordering::SeqCst);
    assert_eq!(
        store.commit(previous, &candidate, metadata.clone()),
        Err(Error::Io)
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(store.snapshot().fingerprint(), previous);
    marker.fail_final.store(true, Ordering::SeqCst);
    assert_eq!(
        store.commit(previous, &candidate, metadata),
        Err(Error::CommitUncertain)
    );
    drop(store);
    let reopened = ArchiveStore::open(&path, None, Some(marker.clone())).unwrap();
    assert_eq!(reopened.snapshot().metadata().manifest.body.generation, 1);
    drop(reopened);
    fs::write(&path, original).unwrap();
    assert!(matches!(
        ArchiveStore::open(&path, None, Some(marker)),
        Err(Error::Changed)
    ));
}

#[test]
fn manifest_and_envelope_tampering_never_publish_a_candidate() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("PUBLIC.taypeer");
    let (candidate, metadata) = f.initial();
    let store = ArchiveStore::create(&path, &candidate, metadata, None).unwrap();
    let id = store.snapshot().metadata().manifest.body.checkpoint;
    let object = store.snapshot().object(id).unwrap();
    let mut bytes = Vec::new();
    object.reader().unwrap().read_to_end(&mut bytes).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    assert!(EncryptedObject::receive(bytes.as_slice(), object.descriptor(), &f.chain).is_err());
    let mut forged = store.snapshot().metadata().clone();
    forged.manifest.body.generation += 1;
    assert!(forged.verify(f.chain.root().unwrap()).is_err());
    let original = fs::read(&path).unwrap();
    drop(store);
    let mut truncated = original.clone();
    truncated.pop();
    fs::write(&path, &truncated).unwrap();
    assert!(ArchiveSnapshot::open(&path, None).is_err());
    assert_eq!(fs::read(&path).unwrap(), truncated);
    let mut trailing = original;
    trailing.push(0);
    fs::write(&path, &trailing).unwrap();
    assert!(ArchiveSnapshot::open(&path, None).is_err());
}

#[test]
fn signed_epoch_baselines_and_binary_bundles_remain_separate_from_member_checkpoints() {
    let f = Fixture::new();
    let mut blobs = BlobStore::new().unwrap();
    let bytes = b"PUBLIC attachment";
    let blob_id = blobs
        .insert(bytes.as_slice(), bytes.len() as u64, 100)
        .unwrap();
    let bundle = blobs.bundle(b"PUBLIC document").unwrap();
    let length = bundle.length();
    let object = EncryptedObject::seal(
        &f.chain,
        &f.author,
        ObjectKind::Checkpoint,
        &f.header,
        &f.key,
        bundle,
        length,
    )
    .unwrap();
    let (document, reopened) = object.unlock_bundle(&f.key).unwrap();
    assert_eq!(document.as_slice(), b"PUBLIC document");
    assert_eq!(reopened.length(&blob_id), Some(bytes.len() as u64));
    let rotated = f
        .chain
        .transition(
            &f.author,
            Digest::of(b"PUBLIC operation"),
            ControlTransition::Rotate {
                revoke: None,
                policy: Digest::of(b"PUBLIC policy 2"),
            },
        )
        .unwrap();
    object.envelope().verify(&rotated).unwrap();
    assert_eq!(object.envelope().epoch, 0);
}
