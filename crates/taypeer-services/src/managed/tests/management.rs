//! Real encrypted commits with PUBLIC identities and an intentionally lost persistence reply.
use super::*;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

struct LostTransferReply {
    inner: CoordinatorPersistence,
    failure: Arc<AtomicU8>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}
impl CipherPersistence for LostTransferReply {
    fn snapshot(&self) -> Result<ArchiveSnapshot, StorageError> {
        self.inner.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, StorageError> {
        let objects: Vec<_> = request
            .objects
            .iter()
            .map(EncryptedObject::descriptor)
            .collect();
        self.requests.lock().unwrap().push(
            serde_json::to_vec(&(
                request.expected,
                request.control,
                &request.controls,
                objects,
                &request.remove,
                request.checkpoint,
                request.baseline,
                &request.journal,
            ))
            .unwrap(),
        );
        match self.failure.swap(0, Ordering::SeqCst) {
            1 => Err(StorageError::CommitUncertain),
            2 => {
                self.inner.commit(request)?;
                Err(StorageError::CommitUncertain)
            }
            _ => self.inner.commit(request),
        }
    }
    fn working_copy(&self) -> Digest {
        self.inner.working_copy()
    }
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), StorageError> {
        self.inner.save_draft(object)
    }
    fn load_draft(&self, chain: &ControlChain) -> Result<Option<EncryptedObject>, StorageError> {
        self.inner.load_draft(chain)
    }
    fn discard_draft(&self) -> Result<(), StorageError> {
        self.inner.discard_draft()
    }
}

#[test]
fn transfer_uncertain_before_publication_reuses_original_ciphertext_and_blocks_other_writes() {
    assert_uncertain_transfer(1);
}

#[test]
fn transfer_lost_durable_reply_confirms_exact_generation_without_a_second_control() {
    assert_uncertain_transfer(2);
}

fn assert_uncertain_transfer(failure_mode: u8) {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(79);
    let recipient = Profile::new(80);
    let path = directory.path().join("PUBLIC uncertain handoff.taypeer");
    let (mut authority, session) = create(&manager, path.clone());
    let group = authority
        .create_group(
            &session,
            "PUBLIC retained group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    admit(&mut authority, &session, &manager, &recipient);
    let peer_path = directory.path().join("PUBLIC successor.taypeer");
    copy_to(&manager, &session.database, &peer_path);
    let (mut peer, peer_session) = recipient.open(peer_path, PASSWORD);
    peer.apply_received(&peer_session).unwrap();
    let consent = peer
        .consent_management(
            &peer_session,
            Digest::of(b"PUBLIC original transfer request"),
        )
        .unwrap();
    authority.lock(&session).unwrap();
    drop(authority);
    let failure = Arc::new(AtomicU8::new(failure_mode));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let port = LostTransferReply {
        inner: CoordinatorPersistence::new(
            Arc::clone(&manager.coordinator),
            session.database.clone(),
            path.clone(),
            Digest::of(&manager.seed),
        )
        .unwrap(),
        failure: Arc::clone(&failure),
        requests: Arc::clone(&requests),
    };
    let mut authority = DatabaseService::new();
    let session = authority
        .open_managed(Box::new(port), PASSWORD, || Ok(Some(manager.author())))
        .unwrap();
    let previous = manager.coordinator.snapshot(&session.database).unwrap();
    let previous_bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        authority.transfer_management(&session, consent.clone()),
        Err(ServiceError::Storage(StorageError::CommitUncertain))
    );
    let first_bytes = std::fs::read(&path).unwrap();
    if failure_mode == 1 {
        assert_eq!(first_bytes, previous_bytes);
    } else {
        assert_ne!(first_bytes, previous_bytes);
        assert_eq!(
            manager
                .coordinator
                .snapshot(&session.database)
                .unwrap()
                .chain()
                .head()
                .manager,
            recipient.identity().device
        );
    }
    assert!(!authority.can_manage(&session).unwrap());
    assert_eq!(
        authority.management_basis(&session).unwrap_err(),
        ServiceError::Storage(StorageError::CommitUncertain)
    );
    assert_eq!(
        authority.create_invitation(&session, 1).err().unwrap(),
        ServiceError::Storage(StorageError::CommitUncertain)
    );
    assert_eq!(
        authority
            .create_group(
                &session,
                "PUBLIC forbidden concurrent group".into(),
                None,
                &crate::new_operation_id().unwrap()
            )
            .unwrap_err(),
        ServiceError::Storage(StorageError::CommitUncertain)
    );
    let mut changed = consent.clone();
    changed.operation = Digest::of(b"PUBLIC different transfer operation");
    assert_eq!(
        authority.transfer_management(&session, changed),
        Err(ServiceError::Storage(StorageError::CommitUncertain))
    );
    assert_eq!(std::fs::read(&path).unwrap(), first_bytes);
    assert_eq!(
        authority
            .group_history(&session, &group)
            .unwrap()
            .value
            .len(),
        1
    );
    failure.store(1, Ordering::SeqCst);
    assert_eq!(
        authority.transfer_management(&session, consent.clone()),
        Err(ServiceError::Storage(StorageError::CommitUncertain))
    );
    assert_eq!(std::fs::read(&path).unwrap(), first_bytes);
    authority
        .transfer_management(&session, consent.clone())
        .unwrap();
    let confirmed = manager.coordinator.snapshot(&session.database).unwrap();
    let confirmed_bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        confirmed.chain().head().manager,
        recipient.identity().device
    );
    assert_eq!(
        confirmed.chain().head().sequence,
        previous.chain().head().sequence + 1
    );
    assert_eq!(
        confirmed.metadata().manifest.body.generation,
        previous.metadata().manifest.body.generation + 1
    );
    assert_eq!(
        confirmed.chain().head().epoch,
        previous.chain().head().epoch
    );
    if failure_mode == 2 {
        assert_eq!(confirmed_bytes, first_bytes);
    }
    let attempts = requests.lock().unwrap();
    assert_eq!(attempts.len(), 3);
    assert!(attempts.iter().all(|attempt| attempt == &attempts[0]));
    drop(attempts);
    assert!(!authority.can_manage(&session).unwrap());
    assert_eq!(
        authority.create_invitation(&session, 1).err().unwrap(),
        ServiceError::Unauthorized
    );
    authority
        .transfer_management(&session, consent.clone())
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), confirmed_bytes);
    assert_eq!(
        authority
            .group_history(&session, &group)
            .unwrap()
            .value
            .len(),
        1
    );
    authority.lock(&session).unwrap();
    drop(authority);
    manager.coordinator.unregister(&session.database).unwrap();
    let (mut reopened, reopened_session) = manager.open(path.clone(), PASSWORD);
    assert!(!reopened.can_manage(&reopened_session).unwrap());
    reopened
        .transfer_management(&reopened_session, consent)
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), confirmed_bytes);
    assert_eq!(
        reopened
            .group_history(&reopened_session, &group)
            .unwrap()
            .value
            .len(),
        1
    );
}
