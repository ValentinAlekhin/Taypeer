//! PUBLIC fixtures: keys stay in this plaintext service test, never a host credential port.
use super::*;
use std::{
    cell::Cell,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn port(profile: &Profile, session: &SessionToken, path: &Path) -> Box<dyn CipherPersistence> {
    Box::new(
        CoordinatorPersistence::new(
            Arc::clone(&profile.coordinator),
            session.database.clone(),
            path.into(),
            Digest::of(&profile.seed),
        )
        .unwrap(),
    )
}

fn entry(service: &mut DatabaseService, session: &SessionToken) -> EntryId {
    let group = service
        .create_group(
            session,
            "PUBLIC keyed group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    service.start_create_entry(session, group).unwrap();
    service
        .update_draft(
            session,
            EditableEntry {
                title: "PUBLIC keyed entry".into(),
                password: Some("PUBLIC keyed value".into()),
                ..Default::default()
            },
        )
        .unwrap();
    service
        .save_draft(session, &crate::new_operation_id().unwrap())
        .unwrap()
        .value
}

#[test]
fn password_credential_handshake_preserves_current_key_provenance_and_measurement() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(86);
    let path = directory.path().join("PUBLIC password credentials.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let snapshot = profile.coordinator.snapshot(&session.database).unwrap();
    service.lock(&session).unwrap();
    let called = Cell::new(0);
    assert!(
        service
            .open_managed_with_credentials(
                port(&profile, &session, &path),
                b"PUBLIC wrong password",
                |_, _| {
                    called.set(called.get() + 1);
                    Ok(Some(profile.author()))
                },
            )
            .is_err()
    );
    assert_eq!(called.get(), 0);
    let reopened = service
        .open_managed_with_credentials(port(&profile, &session, &path), PASSWORD, |current, key| {
            assert_eq!(current.fingerprint(), snapshot.fingerprint());
            DatabaseService::authenticate_archive_read_key(current, key)?;
            called.set(called.get() + 1);
            Ok(Some(profile.author()))
        })
        .unwrap();
    assert_eq!(called.get(), 1);
    let (scope, key) = service.password_authenticated_key(&reopened).unwrap();
    assert_eq!(scope.snapshot, snapshot.fingerprint());
    assert!(
        service
            .database_policy_details(&reopened)
            .unwrap()
            .value
            .protection
            .password_derivation_ms
            .is_some_and(|elapsed| elapsed > 0)
    );
    service.lock(&reopened).unwrap();
    let keyed = service
        .open_managed_with_key_and_author(port(&profile, &reopened, &path), key, |_| {
            Ok(Some(profile.author()))
        })
        .unwrap();
    assert!(matches!(
        service.password_authenticated_key(&keyed),
        Err(ServiceError::Unauthorized)
    ));
    assert_eq!(
        service
            .database_policy_details(&keyed)
            .unwrap()
            .value
            .protection
            .password_derivation_ms,
        None
    );
}

#[test]
fn keyed_open_preserves_current_scope_history_blobs_and_encrypted_draft() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(85);
    let path = directory.path().join("PUBLIC keyed.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let entry = entry(&mut service, &session);
    let attachment = directory.path().join("PUBLIC attachment");
    std::fs::write(&attachment, b"PUBLIC unchanged keyed attachment").unwrap();
    service
        .edit_binary(
            &session,
            &crate::BinaryRequest {
                target: crate::BinaryTarget::Entry(entry.clone()),
                review: None,
                edit: crate::BinaryEdit::Attachment(crate::AttachmentEdit::Add {
                    path: attachment,
                    name: None,
                }),
            },
            &taypeer_core::OperationId::new("PUBLIC keyed attachment"),
        )
        .unwrap();
    let history: Vec<_> = service
        .history(&session, &entry)
        .unwrap()
        .value
        .into_iter()
        .map(|revision| revision.id)
        .collect();
    service.start_edit_entry(&session, &entry).unwrap();
    let mut fields = service.draft(&session).unwrap().value.unwrap().fields;
    fields.title = "PUBLIC interrupted keyed edit".into();
    service.update_draft(&session, fields).unwrap();
    let (scope, key) = service.password_authenticated_key(&session).unwrap();
    let snapshot = profile.coordinator.snapshot(&session.database).unwrap();
    assert_eq!(scope.database, session.database);
    assert_eq!(scope.working_copy, Digest::of(&profile.seed));
    assert_eq!(scope.root, snapshot.chain().root().unwrap());
    assert_eq!(scope.epoch, snapshot.chain().head().epoch);
    assert_eq!(scope.control, snapshot.chain().head_hash().unwrap());
    assert_eq!(scope.snapshot, snapshot.fingerprint());
    assert_eq!(
        scope.checkpoint,
        snapshot.metadata().manifest.body.checkpoint
    );
    service.lock(&session).unwrap();
    let before = std::fs::read(&path).unwrap();
    let draft_path = directory.path().join(format!(
        "PUBLIC keyed.taypeer.{}.draft",
        Digest::of(&profile.seed),
    ));
    let draft_before = std::fs::read(&draft_path).unwrap();
    let called = Cell::new(0);
    let reopened = service
        .open_managed_with_key_and_author(port(&profile, &session, &path), key, |authenticated| {
            assert_eq!(authenticated.fingerprint(), scope.snapshot);
            called.set(called.get() + 1);
            Ok(Some(profile.author()))
        })
        .unwrap();
    assert_eq!(called.get(), 1);
    assert_ne!(reopened.generation, session.generation);
    assert!(matches!(
        service.groups(&session),
        Err(ServiceError::ExpiredSession)
    ));
    assert_eq!(service.entries(&reopened, None, "").unwrap().value.len(), 1);
    assert_eq!(
        service
            .history(&reopened, &entry)
            .unwrap()
            .value
            .into_iter()
            .map(|revision| revision.id)
            .collect::<Vec<_>>(),
        history
    );
    assert!(service.pending_draft(&reopened).unwrap().value.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(&draft_path).unwrap(), draft_before);
    assert!(matches!(
        service.password_authenticated_key(&reopened),
        Err(ServiceError::Unauthorized)
    ));
    let target = crate::BinaryTarget::Entry(entry);
    let binaries = service.binary_view(&reopened, &target).unwrap().value;
    let output = directory.path().join("PUBLIC exported keyed attachment");
    service
        .export_binary(
            &reopened,
            &target,
            &binaries.attachments[0].contents[0].id,
            &output,
            false,
        )
        .unwrap();
    assert_eq!(
        std::fs::read(output).unwrap(),
        b"PUBLIC unchanged keyed attachment"
    );
    service.restore_draft(&reopened).unwrap();
    assert_eq!(
        service
            .draft(&reopened)
            .unwrap()
            .value
            .unwrap()
            .fields
            .title,
        "PUBLIC interrupted keyed edit"
    );
}

#[test]
fn incorrect_and_foreign_keys_fail_before_author_without_unlocking_prior_session() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(86);
    let foreign = Profile::new(87);
    let path = directory.path().join("PUBLIC authenticated.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let (other, other_session) = create(&foreign, directory.path().join("PUBLIC foreign.taypeer"));
    let (_, foreign_key) = other.password_authenticated_key(&other_session).unwrap();
    service.lock(&session).unwrap();
    let before = std::fs::read(&path).unwrap();
    let called = Cell::new(0);
    for key in [ReadKey::from_secret(&[42; 32]), foreign_key] {
        assert!(matches!(
            service.open_managed_with_key_and_author(port(&profile, &session, &path), key, |_| {
                called.set(called.get() + 1);
                Ok(Some(profile.author()))
            },),
            Err(ServiceError::Storage(StorageError::Authentication))
        ));
        assert!(matches!(
            service.groups(&session),
            Err(ServiceError::Locked)
        ));
    }
    assert_eq!(called.get(), 0);
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn rotation_requires_new_password_provenance_and_old_key_never_acquires_author() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(88);
    let path = directory.path().join("PUBLIC epoch.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let (original, old_key) = service.password_authenticated_key(&session).unwrap();
    service
        .create_group(
            &session,
            "PUBLIC same-epoch save".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    let (saved, _) = service.password_authenticated_key(&session).unwrap();
    assert_eq!(saved.epoch, original.epoch);
    assert_eq!(saved.root, original.root);
    assert_ne!(saved.snapshot, original.snapshot);
    service
        .rotate_password(
            &session,
            Digest::of(b"PUBLIC keyed rotation"),
            NEW_PASSWORD,
            None,
        )
        .unwrap();
    assert!(matches!(
        service.password_authenticated_key(&session),
        Err(ServiceError::Unauthorized)
    ));
    service.lock(&session).unwrap();
    assert!(matches!(
        service.password_authenticated_key(&session),
        Err(ServiceError::Locked)
    ));
    let called = Cell::new(0);
    assert!(matches!(
        service.open_managed_with_key_and_author(port(&profile, &session, &path), old_key, |_| {
            called.set(called.get() + 1);
            Ok(Some(profile.author()))
        }),
        Err(ServiceError::Storage(StorageError::Authentication))
    ));
    assert_eq!(called.get(), 0);
    let fresh = service
        .open_managed(port(&profile, &session, &path), NEW_PASSWORD, || {
            Ok(Some(profile.author()))
        })
        .unwrap();
    let (current, _) = service.password_authenticated_key(&fresh).unwrap();
    assert_eq!(current.epoch, original.epoch + 1);
    assert_eq!(current.root, original.root);
}

#[test]
fn learned_rotation_authenticates_latest_baseline_before_author_and_retains_history() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(89);
    let member = Profile::new(90);
    let (mut authority, source) =
        create(&manager, directory.path().join("PUBLIC authority.taypeer"));
    let entry = entry(&mut authority, &source);
    admit(&mut authority, &source, &manager, &member);
    let path = directory.path().join("PUBLIC member.taypeer");
    copy_to(&manager, &source.database, &path);
    let (mut service, session) = member.open(path.clone(), PASSWORD);
    service.apply_received(&session).unwrap();
    let (_, old_key) = service.password_authenticated_key(&session).unwrap();
    let history = service.history(&session, &entry).unwrap().value.len();
    service.start_edit_entry(&session, &entry).unwrap();
    let mut fields = service.draft(&session).unwrap().value.unwrap().fields;
    fields.title = "PUBLIC learned-rotation draft".into();
    service.update_draft(&session, fields).unwrap();
    service.lock(&session).unwrap();
    authority
        .rotate_password(
            &source,
            Digest::of(b"PUBLIC learned keyed rotation"),
            NEW_PASSWORD,
            None,
        )
        .unwrap();
    deliver(&manager, &member, &source.database);
    let current = member.coordinator.snapshot(&source.database).unwrap();
    let active = current
        .object(current.metadata().manifest.body.checkpoint)
        .unwrap();
    assert!(active.envelope().epoch < current.chain().head().epoch);
    let called = Cell::new(0);
    assert!(matches!(
        service.open_managed_with_key_and_author(port(&member, &session, &path), old_key, |_| {
            called.set(called.get() + 1);
            Ok(Some(member.author()))
        }),
        Err(ServiceError::Storage(StorageError::Authentication))
    ));
    assert_eq!(called.get(), 0);
    let key = DatabaseService::authenticate_archive_credentials(&current, NEW_PASSWORD).unwrap();
    let reopened = service
        .open_managed_with_key_and_author(port(&member, &session, &path), key, |authenticated| {
            assert_eq!(authenticated.fingerprint(), current.fingerprint());
            called.set(called.get() + 1);
            Ok(Some(member.author()))
        })
        .unwrap();
    assert_eq!(called.get(), 1);
    service.apply_received(&reopened).unwrap();
    assert_eq!(
        service.history(&reopened, &entry).unwrap().value.len(),
        history
    );
    assert!(service.pending_draft(&reopened).unwrap().value.is_some());
    assert!(matches!(
        service.password_authenticated_key(&reopened),
        Err(ServiceError::Unauthorized)
    ));
    service.restore_draft(&reopened).unwrap();
    assert_eq!(
        service
            .draft(&reopened)
            .unwrap()
            .value
            .unwrap()
            .fields
            .title,
        "PUBLIC learned-rotation draft"
    );
}

struct LostCommitReply {
    inner: Box<dyn CipherPersistence>,
    lose: Arc<AtomicBool>,
}
impl CipherPersistence for LostCommitReply {
    fn snapshot(&self) -> Result<ArchiveSnapshot, StorageError> {
        self.inner.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, StorageError> {
        let saved = self.inner.commit(request)?;
        if self.lose.swap(false, Ordering::SeqCst) {
            return Err(StorageError::CommitUncertain);
        }
        Ok(saved)
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
fn uncertain_document_publication_never_yields_an_enrollment_key() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(91);
    let path = directory.path().join("PUBLIC unknown.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    service.lock(&session).unwrap();
    let lose = Arc::new(AtomicBool::new(false));
    let opened = service
        .open_managed(
            Box::new(LostCommitReply {
                inner: port(&profile, &session, &path),
                lose: Arc::clone(&lose),
            }),
            PASSWORD,
            || Ok(Some(profile.author())),
        )
        .unwrap();
    service.password_authenticated_key(&opened).unwrap();
    lose.store(true, Ordering::SeqCst);
    assert!(matches!(
        service.create_group(
            &opened,
            "PUBLIC unknown publication".into(),
            None,
            &crate::new_operation_id().unwrap()
        ),
        Err(ServiceError::Storage(StorageError::CommitUncertain))
    ));
    let durable = std::fs::read(path).unwrap();
    assert!(matches!(
        service.password_authenticated_key(&opened),
        Err(ServiceError::Storage(StorageError::CommitUncertain))
    ));
    assert_eq!(
        std::fs::read(directory.path().join("PUBLIC unknown.taypeer")).unwrap(),
        durable
    );
}

#[test]
fn incoming_snapshot_and_signed_fork_refuse_enrollment_without_mutating_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(92);
    let rolled_back_manager = Profile::new(92);
    let member = Profile::new(93);
    let (mut left, left_session) = create(&manager, directory.path().join("PUBLIC left.taypeer"));
    admit(&mut left, &left_session, &manager, &member);
    let right_path = directory.path().join("PUBLIC right.taypeer");
    let path = directory.path().join("PUBLIC fork.taypeer");
    copy_to(&manager, &left_session.database, &right_path);
    copy_to(&manager, &left_session.database, &path);
    let (mut right, right_session) = rolled_back_manager.open(right_path, PASSWORD);
    let (mut service, session) = member.open(path.clone(), PASSWORD);
    service.apply_received(&session).unwrap();
    service.password_authenticated_key(&session).unwrap();
    left.set_database_policy(
        &left_session,
        Digest::of(b"PUBLIC left enrollment control"),
        DatabasePolicy::new(512 * 1024, 2 * 1024 * 1024, 500).unwrap(),
        None,
    )
    .unwrap();
    right
        .set_database_policy(
            &right_session,
            Digest::of(b"PUBLIC right enrollment control"),
            DatabasePolicy::new(256 * 1024, 2 * 1024 * 1024, 500).unwrap(),
            None,
        )
        .unwrap();
    deliver(&manager, &member, &session.database);
    let incoming = std::fs::read(&path).unwrap();
    assert!(matches!(
        service.password_authenticated_key(&session),
        Err(ServiceError::Storage(StorageError::Changed))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), incoming);
    service.apply_received(&session).unwrap();
    let conflicting = rolled_back_manager
        .coordinator
        .snapshot(&right_session.database)
        .unwrap();
    assert!(
        member
            .coordinator
            .command(
                rolled_back_manager.transport.public(),
                Command::Offer(Box::new(conflicting.metadata().clone()))
            )
            .is_err()
    );
    let evidence = std::fs::read(&path).unwrap();
    assert!(matches!(
        service.password_authenticated_key(&session),
        Err(ServiceError::Trust(taypeer_trust::Error::Fork))
    ));
    assert_eq!(std::fs::read(path).unwrap(), evidence);
}
