//! Public synthetic failure injection around durable document and local form publication.
use super::*;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Failures {
    uncertain_commit: AtomicBool,
    local_after_commit: AtomicBool,
    deny_local: AtomicBool,
}

struct Port {
    inner: CoordinatorPersistence,
    failures: Arc<Failures>,
}

impl CipherPersistence for Port {
    fn snapshot(&self) -> Result<ArchiveSnapshot, StorageError> {
        self.inner.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, StorageError> {
        let saved = self.inner.commit(request)?;
        if self
            .failures
            .local_after_commit
            .swap(false, Ordering::SeqCst)
        {
            self.failures.deny_local.store(true, Ordering::SeqCst);
        }
        if self.failures.uncertain_commit.swap(false, Ordering::SeqCst) {
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
        if self.failures.deny_local.load(Ordering::SeqCst) {
            return Err(StorageError::Io);
        }
        self.inner.save_draft(object)
    }
    fn load_draft(&self, chain: &ControlChain) -> Result<Option<EncryptedObject>, StorageError> {
        self.inner.load_draft(chain)
    }
    fn discard_draft(&self) -> Result<(), StorageError> {
        self.inner.discard_draft()
    }
}

fn reopen(
    profile: &Profile,
    path: PathBuf,
    database: DatabaseId,
    failures: Arc<Failures>,
) -> (DatabaseService, SessionToken) {
    let port = Port {
        inner: CoordinatorPersistence::new(
            Arc::clone(&profile.coordinator),
            database,
            path,
            Digest::of(&profile.seed),
        )
        .unwrap(),
        failures,
    };
    let mut service = DatabaseService::new();
    let session = service
        .open_managed(Box::new(port), PASSWORD, || Ok(Some(profile.author())))
        .unwrap();
    (service, session)
}

#[test]
fn uncertain_snapshot_reconciles_the_exact_durable_receipt_without_reauthenticating() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(34);
    let path = directory.path().join("PUBLIC uncertainty.taypeer");
    let (mut seed, session) = create(&profile, path.clone());
    let database = session.database.clone();
    seed.lock(&session).unwrap();
    drop(seed);
    let failures = Arc::new(Failures::default());
    let (mut service, session) = reopen(
        &profile,
        path.clone(),
        database.clone(),
        Arc::clone(&failures),
    );
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(
            &session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC confirmed once".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let identity = service.editor_view(&session).unwrap().identity;
    let operation = crate::new_operation_id().unwrap();
    failures.uncertain_commit.store(true, Ordering::SeqCst);
    assert_eq!(
        service
            .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
            .unwrap_err(),
        ServiceError::Storage(StorageError::CommitUncertain)
    );
    // Local fallback is independently verifiable even while document confirmation is uncertain.
    service.persist_drafts(&session).unwrap();
    service.reconcile_uncertain(&session).unwrap();
    assert!(!service.editor_view(&session).unwrap().dirty);
    let entry = service
        .entries(&session, None, "")
        .unwrap()
        .value
        .remove(0)
        .id;
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    assert!(matches!(
        service
            .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
            .unwrap()
            .value,
        DraftSaveOutcome::Saved { .. }
    ));
    service
        .patch_draft(
            &session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC continuation".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let newer = service.editor_view(&session).unwrap().identity;
    service
        .save_draft_snapshot(
            &session,
            &newer.draft,
            newer.revision,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    service.lock(&session).unwrap();
    drop(service);
    let (service, session) = reopen(&profile, path, database, failures);
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 2);
    assert_eq!(
        service.view_entry(&session, &entry).unwrap().value.title,
        "PUBLIC continuation"
    );
}

#[test]
fn confirmed_snapshot_survives_failed_local_cleanup_and_lost_reply() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(35);
    let path = directory.path().join("PUBLIC cleanup.taypeer");
    let (mut seed, session) = create(&profile, path.clone());
    let database = session.database.clone();
    seed.lock(&session).unwrap();
    drop(seed);
    let failures = Arc::new(Failures::default());
    let (mut service, session) = reopen(
        &profile,
        path.clone(),
        database.clone(),
        Arc::clone(&failures),
    );
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(
            &session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC durable before cleanup".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let identity = service.editor_view(&session).unwrap().identity;
    let operation = crate::new_operation_id().unwrap();
    failures.local_after_commit.store(true, Ordering::SeqCst);
    assert!(matches!(
        service
            .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
            .unwrap()
            .value,
        DraftSaveOutcome::Saved { .. }
    ));
    assert!(!service.editor_view(&session).unwrap().dirty);
    drop(service); // Abrupt restart, before a successful local rewrite.
    failures.deny_local.store(false, Ordering::SeqCst);
    let (mut service, session) = reopen(&profile, path, database, failures);
    assert!(service.drafts(&session).unwrap().value.is_empty());
    let entry = service
        .entries(&session, None, "")
        .unwrap()
        .value
        .remove(0)
        .id;
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    service
        .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
        .unwrap();
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
}

#[test]
fn received_apply_reconciles_uncertainty_before_retry_and_preserves_local_forms() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(36);
    let peer = Profile::new(37);
    let path = directory.path().join("PUBLIC automatic receive.taypeer");
    let (mut seed, session) = create(&profile, path.clone());
    let database = session.database.clone();
    admit(&mut seed, &session, &profile, &peer);
    let peer_path = directory.path().join("PUBLIC sender.taypeer");
    copy_to(&profile, &database, &peer_path);
    let (mut sender, sender_session) = peer.open(peer_path, PASSWORD);
    sender
        .create_group(
            &sender_session,
            "PUBLIC received once".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    seed.lock(&session).unwrap();
    drop(seed);
    let failures = Arc::new(Failures::default());
    let (mut service, session) = reopen(&profile, path, database.clone(), Arc::clone(&failures));
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(
            &session,
            EntryPatch {
                password: FieldUpdate::Set("PUBLIC retained local form".into()),
                ..Default::default()
            },
        )
        .unwrap();
    service.persist_drafts(&session).unwrap();
    deliver(&peer, &profile, &database);
    failures.uncertain_commit.store(true, Ordering::SeqCst);
    assert_eq!(
        service.apply_received(&session).unwrap_err(),
        ServiceError::Storage(StorageError::CommitUncertain)
    );
    service.apply_received(&session).unwrap();
    let groups = service.groups(&session).unwrap().value;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "PUBLIC received once");
    assert_eq!(
        service
            .group_history(&session, &groups[0].id)
            .unwrap()
            .value
            .len(),
        1
    );
    assert!(service.editor_view(&session).unwrap().dirty);
    assert_eq!(
        service.reveal_editor(&session, None).unwrap().expose(),
        "PUBLIC retained local form"
    );
    assert!(service.can_write(&session).unwrap());
}
