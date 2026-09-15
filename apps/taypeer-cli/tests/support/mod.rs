//! Independent dev4 fixture encoding for CLI lifecycle tests. Every payload is PUBLIC.
//! This encoder is test-only; product code never signs arbitrary imported documents.
use std::path::{Path, PathBuf};
use taypeer_core::{DatabasePolicy, EntryId, GroupId};
use taypeer_document::Document;
use taypeer_runtime::profile::NativeProfile;
use taypeer_storage::{ArchiveSeed, BlobStore, EncryptedObject, create_epoch};
use taypeer_trust::{ControlChain, Digest, Identity, ObjectKind, SourceProof};
use zeroize::Zeroizing;

pub fn profile(path: &Path) -> PathBuf {
    path.parent().unwrap().join("profile")
}

pub fn fixture(path: &Path) -> (Document, GroupId, GroupId, EntryId) {
    let lease = NativeProfile::acquire(&profile(path)).unwrap();
    let author = lease.profile().author().unwrap();
    let mut doc =
        Document::new_with_writer("PUBLIC process fixture", 1, *author.device_id().as_bytes())
            .unwrap();
    let a = doc.create_group("PUBLIC A".into(), None, 2).unwrap().id;
    let b = doc.create_group("PUBLIC B".into(), None, 2).unwrap().id;
    let mut draft = doc.begin_create_entry(a.clone()).unwrap();
    draft.fields_mut().title = "PUBLIC entry".into();
    draft.fields_mut().password = Some("PUBLIC_HIDDEN_PROCESS".into());
    let entry = doc.save_entry(draft, 3).unwrap();
    (doc, a, b, entry)
}

pub fn persist(path: &Path, document: &Document, password: &[u8]) {
    let lease = NativeProfile::acquire(&profile(path)).unwrap();
    let author = lease.profile().author().unwrap();
    let transport = lease.profile().transport().unwrap();
    let identity = Identity::new(author.public(), transport.public()).unwrap();
    let policy = DatabasePolicy::new(1024 * 1024 * 100, 1024 * 1024 * 1024, 500).unwrap();
    let salt = [71_u8; 32];
    let commitment = Digest::object(b"taypeer/private-policy/1", &(policy, &salt[..])).unwrap();
    let chain = ControlChain::genesis(
        document.database_id().clone(),
        identity,
        &author,
        commitment,
        4,
    )
    .unwrap();
    let proofs: std::collections::BTreeMap<_, _> = document
        .changes_since(&[])
        .unwrap()
        .into_iter()
        .map(|source| {
            (
                source.metadata().hash.clone(),
                SourceProof::sign(&chain, &author, source.bytes()).unwrap(),
            )
        })
        .collect();
    let (header, key) = create_epoch(password, 500).unwrap();
    let metadata = serde_json::json!({"version": 1, "policy": policy, "policy_salt": salt,
        "keys": {"0": key.secret_bytes().as_ref()}, "proofs": proofs, "blobs": {},
        "processed": [], "discarded": [], "administration": {}});
    let json = Zeroizing::new(serde_json::to_vec(&metadata).unwrap());
    let mut clear = Zeroizing::new(b"TAYCLR4\0".to_vec());
    clear.extend_from_slice(&(json.len() as u64).to_le_bytes());
    clear.extend_from_slice(&json);
    clear.extend_from_slice(&document.export());
    let blobs = BlobStore::new().unwrap();
    let seal = |kind| {
        let reader = blobs.bundle(&clear).unwrap();
        let length = reader.length();
        EncryptedObject::seal(&chain, &author, kind, &header, &key, reader, length).unwrap()
    };
    let checkpoint = seal(ObjectKind::Checkpoint);
    let baseline = seal(ObjectKind::Baseline);
    let seed = ArchiveSeed {
        controls: chain.records().to_vec(),
        checkpoint: checkpoint.descriptor().digest,
        baseline: baseline.descriptor().digest,
        objects: vec![checkpoint, baseline],
    };
    seed.create(path, &transport, None).unwrap();
}
