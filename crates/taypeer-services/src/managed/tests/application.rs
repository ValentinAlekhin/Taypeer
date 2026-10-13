//! Actual same-epoch baseline delivery and denied durable publication, using PUBLIC contents.
use super::*;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

struct DeniedCommitPort {
    inner: CoordinatorPersistence,
    deny_next: Arc<AtomicBool>,
}
impl CipherPersistence for DeniedCommitPort {
    fn snapshot(&self) -> Result<ArchiveSnapshot, StorageError> {
        self.inner.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, StorageError> {
        if self.deny_next.swap(false, Ordering::SeqCst) {
            return Err(StorageError::Io);
        }
        self.inner.commit(request)
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

fn open_with_commit_denial(
    profile: &Profile,
    database: DatabaseId,
    path: PathBuf,
    deny_next: Arc<AtomicBool>,
) -> (DatabaseService, SessionToken) {
    let inner = CoordinatorPersistence::new(
        Arc::clone(&profile.coordinator),
        database,
        path,
        Digest::of(&profile.seed),
    )
    .unwrap();
    let mut service = DatabaseService::new();
    let session = service
        .open_managed(
            Box::new(DeniedCommitPort { inner, deny_next }),
            PASSWORD,
            || Ok(Some(profile.author())),
        )
        .unwrap();
    (service, session)
}

#[test]
fn same_epoch_baseline_reports_change_only_after_durable_application() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(60);
    let member = Profile::new(61);
    let (mut authority, session) =
        create(&manager, directory.path().join("PUBLIC manager.taypeer"));
    let group = authority
        .create_group(
            &session,
            "PUBLIC baseline group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    authority.start_create_entry(&session, group).unwrap();
    authority
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC original baseline title".into(),
                password: Some("PUBLIC original baseline password".into()),
                tags: vec!["PUBLIC original baseline tag".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let entry = authority
        .save_draft(&session, &crate::new_operation_id().unwrap())
        .unwrap()
        .value;
    admit(&mut authority, &session, &manager, &member);
    let path = directory.path().join("PUBLIC member.taypeer");
    copy_to(&manager, &session.database, &path);
    let (mut registered, registered_session) = member.open(path.clone(), PASSWORD);
    registered.lock(&registered_session).unwrap();
    drop(registered);
    let deny_next = Arc::new(AtomicBool::new(false));
    let (mut peer, peer_session) = open_with_commit_denial(
        &member,
        session.database.clone(),
        path.clone(),
        Arc::clone(&deny_next),
    );
    let initial = peer.apply_received(&peer_session).unwrap().value;
    assert!(!initial.changed);
    assert_eq!(initial.applied, 0);

    let old = manager.coordinator.snapshot(&session.database).unwrap();
    authority.start_edit_entry(&session, &entry).unwrap();
    authority
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC changed baseline title".into(),
                password: Some("PUBLIC changed baseline password".into()),
                tags: vec!["PUBLIC changed baseline tag".into()],
                ..Default::default()
            },
        )
        .unwrap();
    authority
        .save_draft(&session, &crate::new_operation_id().unwrap())
        .unwrap();
    let policy = DatabasePolicy::new(512 * 1024, 2 * 1024 * 1024, 500).unwrap();
    authority
        .set_database_policy(
            &session,
            Digest::of(b"PUBLIC same-epoch baseline policy"),
            policy,
            None,
        )
        .unwrap();
    let updated = manager.coordinator.snapshot(&session.database).unwrap();
    assert_eq!(old.chain().head().epoch, updated.chain().head().epoch);
    assert_ne!(
        old.chain().head_hash().unwrap(),
        updated.chain().head_hash().unwrap()
    );
    assert_ne!(
        old.metadata().manifest.body.baseline,
        updated.metadata().manifest.body.baseline
    );
    deliver(&manager, &member, &session.database);
    let received = std::fs::read(&path).unwrap();
    let received_fingerprint = member
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();

    deny_next.store(true, Ordering::SeqCst);
    assert!(matches!(
        peer.apply_received(&peer_session),
        Err(ServiceError::Storage(StorageError::Io))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), received);
    assert_eq!(
        member
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint(),
        received_fingerprint
    );
    assert_eq!(
        peer.view_entry(&peer_session, &entry).unwrap().value.title,
        "PUBLIC original baseline title"
    );
    assert_eq!(peer.history(&peer_session, &entry).unwrap().value.len(), 1);

    let applied = peer.apply_received(&peer_session).unwrap().value;
    assert!(applied.changed);
    // Sources already accepted through the authenticated baseline are not counted as packets.
    assert_eq!(applied.applied, 0);
    assert!(applied.pending.is_empty());
    let view = peer.view_entry(&peer_session, &entry).unwrap().value;
    assert_eq!(view.title, "PUBLIC changed baseline title");
    assert_eq!(view.tags, vec!["PUBLIC changed baseline tag"]);
    assert_eq!(
        peer.reveal_password(&peer_session, &entry)
            .unwrap()
            .value
            .expose(),
        "PUBLIC changed baseline password"
    );
    assert_eq!(peer.history(&peer_session, &entry).unwrap().value.len(), 2);
    assert_eq!(peer.database_policy(&peer_session).unwrap().value, policy);
    let confirmed = std::fs::read(&path).unwrap();
    assert_ne!(confirmed, received);
    let empty = peer.apply_received(&peer_session).unwrap().value;
    assert!(!empty.changed);
    assert_eq!(empty.applied, 0);
    assert!(empty.pending.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), confirmed);

    peer.lock(&peer_session).unwrap();
    drop(peer);
    let (mut reopened, reopened_session) =
        open_with_commit_denial(&member, session.database.clone(), path.clone(), deny_next);
    assert_eq!(
        reopened
            .view_entry(&reopened_session, &entry)
            .unwrap()
            .value
            .title,
        "PUBLIC changed baseline title"
    );
    assert_eq!(
        reopened
            .history(&reopened_session, &entry)
            .unwrap()
            .value
            .len(),
        2
    );
    assert!(
        !reopened
            .apply_received(&reopened_session)
            .unwrap()
            .value
            .changed
    );
    assert_eq!(std::fs::read(path).unwrap(), confirmed);
}

#[test]
fn received_handoff_does_not_grant_management_until_baseline_application_is_durable() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(76);
    let recipient = Profile::new(77);
    let other = Profile::new(78);
    let source_path = directory.path().join("PUBLIC handoff source.taypeer");
    let (mut source, source_session) = create(&manager, source_path.clone());
    let group = source
        .create_group(
            &source_session,
            "PUBLIC handoff group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    admit(&mut source, &source_session, &manager, &recipient);
    admit(&mut source, &source_session, &manager, &other);
    let path = directory.path().join("PUBLIC handoff recipient.taypeer");
    copy_to(&manager, &source_session.database, &path);
    let (mut registered, registered_session) = recipient.open(path.clone(), PASSWORD);
    registered.lock(&registered_session).unwrap();
    drop(registered);
    let denied = Arc::new(AtomicBool::new(false));
    let (mut peer, session) = open_with_commit_denial(
        &recipient,
        source_session.database.clone(),
        path.clone(),
        Arc::clone(&denied),
    );
    peer.apply_received(&session).unwrap();
    let previous = peer.management_basis(&session).unwrap().value.unwrap();
    assert_eq!(previous.manager, manager.identity().device);
    assert!(!peer.can_manage(&session).unwrap());
    let operation = Digest::of(b"PUBLIC durably applied handoff");
    let consent = peer.consent_management(&session, operation).unwrap();
    source
        .transfer_management(&source_session, consent.clone())
        .unwrap();
    let source_bytes = std::fs::read(&source_path).unwrap();
    assert!(!source.can_manage(&source_session).unwrap());
    source
        .transfer_management(&source_session, consent)
        .unwrap();
    assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
    deliver(&manager, &recipient, &session.database);
    let received = std::fs::read(&path).unwrap();
    assert_eq!(
        recipient
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .chain()
            .head()
            .manager,
        recipient.identity().device
    );
    assert!(peer.management_basis(&session).unwrap().value.is_none());
    assert!(!peer.can_manage(&session).unwrap());

    peer.lock(&session).unwrap();
    drop(peer);
    let (mut peer, session) = open_with_commit_denial(
        &recipient,
        source_session.database.clone(),
        path.clone(),
        Arc::clone(&denied),
    );
    assert!(peer.management_basis(&session).unwrap().value.is_none());
    assert!(!peer.can_manage(&session).unwrap());

    denied.store(true, Ordering::SeqCst);
    assert_eq!(
        peer.apply_received(&session).unwrap_err(),
        ServiceError::Storage(StorageError::Io)
    );
    assert!(peer.management_basis(&session).unwrap().value.is_none());
    assert!(!peer.can_manage(&session).unwrap());
    assert_eq!(
        peer.create_invitation(&session, 1).err().unwrap(),
        ServiceError::InvalidContext
    );
    assert_eq!(
        peer.set_database_policy(
            &session,
            Digest::of(b"PUBLIC denied policy"),
            DatabasePolicy::default(),
            None
        )
        .unwrap_err(),
        ServiceError::InvalidContext
    );
    assert_eq!(
        peer.rotate_password(
            &session,
            Digest::of(b"PUBLIC denied rotation"),
            NEW_PASSWORD,
            None
        )
        .unwrap_err(),
        ServiceError::InvalidContext
    );
    let snapshot = recipient.coordinator.snapshot(&session.database).unwrap();
    let onward = taypeer_trust::HandoffConsent::sign(
        snapshot.chain(),
        &other.author(),
        Digest::of(b"PUBLIC denied onward handoff"),
    )
    .unwrap();
    assert_eq!(
        peer.transfer_management(&session, onward).unwrap_err(),
        ServiceError::InvalidContext
    );
    assert_eq!(std::fs::read(&path).unwrap(), received);
    assert_eq!(peer.group_history(&session, &group).unwrap().value.len(), 1);

    let applied = peer.apply_received(&session).unwrap().value;
    assert!(applied.changed);
    assert_eq!(applied.applied, 0);
    assert!(peer.can_manage(&session).unwrap());
    let basis = peer.management_basis(&session).unwrap().value.unwrap();
    let applied_snapshot = recipient.coordinator.snapshot(&session.database).unwrap();
    assert_eq!(basis.identity, recipient.identity());
    assert_eq!(basis.manager, recipient.identity().device);
    assert_eq!(basis.control, snapshot.chain().head_hash().unwrap());
    assert_eq!(
        basis.baseline,
        applied_snapshot.metadata().manifest.body.baseline
    );
    assert_ne!(basis.baseline, previous.baseline);
    assert_eq!(
        applied_snapshot
            .object(basis.baseline)
            .unwrap()
            .envelope()
            .control,
        basis.control
    );
    assert_eq!(basis.root, previous.root);
    assert_eq!(peer.group_history(&session, &group).unwrap().value.len(), 1);
    let confirmed = std::fs::read(&path).unwrap();
    peer.lock(&session).unwrap();
    drop(peer);
    let (reopened, reopened_session) =
        open_with_commit_denial(&recipient, session.database, path.clone(), denied);
    assert!(reopened.can_manage(&reopened_session).unwrap());
    assert_eq!(
        reopened
            .management_basis(&reopened_session)
            .unwrap()
            .value
            .unwrap()
            .control,
        basis.control
    );
    assert_eq!(std::fs::read(path).unwrap(), confirmed);
}
