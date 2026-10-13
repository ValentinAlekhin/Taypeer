//! PUBLIC actual encrypted files prove numeric projection and nonmutating calibration.
use super::*;
use std::path::Path;

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

struct DescriptorOnlyPort {
    inner: Box<dyn CipherPersistence>,
}

impl CipherPersistence for DescriptorOnlyPort {
    fn snapshot(&self) -> Result<ArchiveSnapshot, StorageError> {
        self.inner.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, StorageError> {
        self.inner.commit(request)
    }
    fn working_copy(&self) -> Digest {
        self.inner.working_copy()
    }
    fn path(&self) -> &Path {
        Path::new("")
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

fn saved_entry(service: &mut DatabaseService, session: &SessionToken) -> EntryId {
    let group = service
        .create_group(
            session,
            "PUBLIC policy group".into(),
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
                title: "PUBLIC policy entry".into(),
                password: Some("PUBLIC policy secret".into()),
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
fn descriptor_only_policy_read_reports_actual_ciphertext_size_without_file_or_history_writes() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(107);
    let path = directory.path().join("PUBLIC descriptor policy.taypeer");
    let (mut service, original) = create(&profile, path.clone());
    service.lock(&original).unwrap();
    drop(service);
    // Android gives the descriptor-only port to a new isolated worker, rather
    // than replacing a retained worker's filesystem binding with another path.
    let mut service = DatabaseService::new();
    let session = service
        .open_managed(
            Box::new(DescriptorOnlyPort {
                inner: port(&profile, &original, &path),
            }),
            PASSWORD,
            || Ok(Some(profile.author())),
        )
        .unwrap();
    let entry = saved_entry(&mut service, &session);
    let contents = b"PUBLIC descriptor-only policy attachment";
    service.start_edit_entry(&session, &entry).unwrap();
    let draft = service.editor_view(&session).unwrap().identity.draft;
    service
        .import_attachment_stream(
            &session,
            &draft,
            contents.as_slice(),
            crate::AttachmentImport {
                length: contents.len() as u64,
                name: "PUBLIC descriptor attachment".into(),
                replacement: None,
            },
            &taypeer_core::OperationId::new("PUBLIC descriptor policy attachment"),
        )
        .unwrap();
    let editor = service.editor_view(&session).unwrap();
    let operation = taypeer_core::OperationId::new("PUBLIC descriptor attachment save");
    let outcome = service
        .save_draft_snapshot(&session, &draft, editor.identity.revision, &operation)
        .unwrap()
        .value;
    let crate::DraftSaveOutcome::Saved {
        identity,
        operation: saved_operation,
    } = outcome
    else {
        panic!("PUBLIC attachment must be durably saved");
    };
    assert_eq!(identity.target, crate::DraftTarget::Entry(entry.clone()));
    assert_eq!(saved_operation, operation);
    let before = std::fs::read(&path).unwrap();
    let history: Vec<_> = service
        .history(&session, &entry)
        .unwrap()
        .value
        .into_iter()
        .map(|row| row.id)
        .collect();
    let snapshot = profile.coordinator.snapshot(&session.database).unwrap();

    let details = service.database_policy_details(&session).unwrap().value;
    assert_eq!(details.usage.file_bytes, before.len() as u64);
    assert_eq!(details.usage.attachment_bytes, contents.len() as u64);
    assert_eq!(
        details.policy,
        DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap()
    );
    assert_eq!(
        serde_json::to_value(details.usage).unwrap(),
        serde_json::to_value(service.storage_usage(&session).unwrap().value).unwrap()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        profile
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint(),
        snapshot.fingerprint()
    );
    assert_eq!(
        service
            .history(&session, &entry)
            .unwrap()
            .value
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        history
    );
}

#[test]
fn numeric_policy_and_preview_preserve_ciphertext_history_binary_usage_and_local_draft() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(103);
    let path = directory.path().join("PUBLIC policy.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let entry = saved_entry(&mut service, &session);
    let input = directory.path().join("PUBLIC attachment");
    let contents = b"PUBLIC attachment retained by history";
    std::fs::write(&input, contents).unwrap();
    service
        .edit_binary(
            &session,
            &crate::BinaryRequest {
                target: crate::BinaryTarget::Entry(entry.clone()),
                review: None,
                edit: crate::BinaryEdit::Attachment(crate::AttachmentEdit::Add {
                    path: input,
                    name: None,
                }),
            },
            &taypeer_core::OperationId::new("PUBLIC policy attachment"),
        )
        .unwrap();
    service.start_edit_entry(&session, &entry).unwrap();
    let mut fields = service.draft(&session).unwrap().value.unwrap().fields;
    fields.title = "PUBLIC unsaved policy draft".into();
    service.update_draft(&session, fields).unwrap();
    service.persist_drafts(&session).unwrap();
    let before = std::fs::read(&path).unwrap();
    let draft_path = directory.path().join(format!(
        "PUBLIC policy.taypeer.{}.draft",
        Digest::of(&profile.seed)
    ));
    let draft_before = std::fs::read(&draft_path).unwrap();
    let snapshot = profile.coordinator.snapshot(&session.database).unwrap();
    let checkpoint = snapshot
        .object(snapshot.metadata().manifest.body.checkpoint)
        .unwrap();
    let parameters =
        taypeer_storage::password_kdf_parameters(&checkpoint.password_header().unwrap()).unwrap();
    let history: Vec<_> = service
        .history(&session, &entry)
        .unwrap()
        .value
        .into_iter()
        .map(|row| row.id)
        .collect();
    let details = service.database_policy_details(&session).unwrap().value;
    assert_eq!(details.protection.parameters, parameters);
    assert!(
        details
            .protection
            .password_derivation_ms
            .is_some_and(|elapsed| elapsed > 0)
    );
    assert_eq!(details.policy.kdf_target_ms(), 500);
    assert_eq!(details.usage.attachment_bytes, contents.len() as u64);
    assert_eq!(details.usage.file_bytes, before.len() as u64);
    assert_eq!(
        serde_json::to_value(&details.usage).unwrap(),
        serde_json::to_value(service.storage_usage(&session).unwrap().value).unwrap()
    );
    assert!(!details.usage.over_limit);
    assert!(details.usage.missing.is_empty());
    let preview = service.calibrate_protection(&session, 500).unwrap().value;
    assert_eq!(preview.target_ms, 500);
    assert_eq!(preview.measurement.parameters.memory_kib, 65_536);
    assert_eq!(preview.measurement.parameters.parallelism, 1);
    assert!((3..=256).contains(&preview.measurement.parameters.iterations));
    assert!(preview.measurement.elapsed_ms > 0);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(draft_path).unwrap(), draft_before);
    assert_eq!(
        profile
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint(),
        snapshot.fingerprint()
    );
    assert_eq!(
        service
            .history(&session, &entry)
            .unwrap()
            .value
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        history
    );
    assert_eq!(
        service.draft(&session).unwrap().value.unwrap().fields.title,
        "PUBLIC unsaved policy draft"
    );
    let after = service.database_policy_details(&session).unwrap().value;
    assert_eq!(
        after.protection.password_derivation_ms,
        details.protection.password_derivation_ms
    );
    assert_eq!(
        serde_json::to_value(after.usage).unwrap(),
        serde_json::to_value(details.usage).unwrap()
    );
}

#[test]
fn quota_only_apply_keeps_measured_wrapper_but_new_epoch_clears_it_and_reopen_measures_again() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(104);
    let path = directory.path().join("PUBLIC KDF epoch.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let entry = saved_entry(&mut service, &session);
    let original = service.database_policy_details(&session).unwrap().value;
    let before = profile.coordinator.snapshot(&session.database).unwrap();
    let quota = DatabasePolicy::new(512 * 1024, 1024 * 1024, 500).unwrap();
    service
        .set_database_policy(
            &session,
            Digest::of(b"PUBLIC quota-only apply"),
            quota,
            None,
        )
        .unwrap();
    let quota_details = service.database_policy_details(&session).unwrap().value;
    let quota_snapshot = profile.coordinator.snapshot(&session.database).unwrap();
    assert_eq!(
        quota_snapshot.chain().head().epoch,
        before.chain().head().epoch
    );
    assert_eq!(
        quota_details.protection.parameters,
        original.protection.parameters
    );
    assert_eq!(
        quota_details.protection.password_derivation_ms,
        original.protection.password_derivation_ms
    );
    assert_eq!(quota_details.policy, quota);
    let operation = Digest::of(b"PUBLIC changed KDF target apply");
    let changed = DatabasePolicy::new(512 * 1024, 1024 * 1024, 750).unwrap();
    service
        .set_database_policy(&session, operation, changed, Some(PASSWORD))
        .unwrap();
    let changed_snapshot = profile.coordinator.snapshot(&session.database).unwrap();
    assert_eq!(
        changed_snapshot.chain().head().epoch,
        before.chain().head().epoch + 1
    );
    let changed_details = service.database_policy_details(&session).unwrap().value;
    assert_eq!(changed_details.policy, changed);
    assert_eq!(changed_details.protection.password_derivation_ms, None);
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    let durable = std::fs::read(&path).unwrap();
    service
        .set_database_policy(&session, operation, changed, None)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), durable);
    service.lock(&session).unwrap();
    let reopened = service
        .open_managed(port(&profile, &session, &path), PASSWORD, || {
            Ok(Some(profile.author()))
        })
        .unwrap();
    assert!(
        service
            .database_policy_details(&reopened)
            .unwrap()
            .value
            .protection
            .password_derivation_ms
            .is_some_and(|elapsed| elapsed > 0)
    );
    assert_eq!(std::fs::read(path).unwrap(), durable);
}

#[test]
fn keyed_open_has_no_password_measurement_and_readonly_or_invalid_preview_never_publishes() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(105);
    let path = directory.path().join("PUBLIC keyed policy.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let (_, key) = service.password_authenticated_key(&session).unwrap();
    service.lock(&session).unwrap();
    let before = std::fs::read(&path).unwrap();
    let keyed = service
        .open_managed_with_key_and_author(port(&profile, &session, &path), key, |_| {
            Ok(Some(profile.author()))
        })
        .unwrap();
    assert_eq!(
        service
            .database_policy_details(&keyed)
            .unwrap()
            .value
            .protection
            .password_derivation_ms,
        None
    );
    for invalid in [499, 5001] {
        assert!(matches!(
            service.calibrate_protection(&keyed, invalid),
            Err(ServiceError::InvalidInput)
        ));
    }
    service.lock(&keyed).unwrap();
    assert!(matches!(
        service.database_policy_details(&keyed),
        Err(ServiceError::Locked)
    ));
    let readonly = service
        .open_managed(port(&profile, &keyed, &path), PASSWORD, || Ok(None))
        .unwrap();
    assert!(
        service
            .database_policy_details(&readonly)
            .unwrap()
            .value
            .protection
            .password_derivation_ms
            .is_some()
    );
    assert!(matches!(
        service.calibrate_protection(&readonly, 500),
        Err(ServiceError::ReadOnly)
    ));
    assert!(matches!(
        service.database_policy_details(&keyed),
        Err(ServiceError::ExpiredSession)
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn rejected_current_password_leaves_the_operation_available_for_one_corrected_apply() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(106);
    let path = directory.path().join("PUBLIC rejected policy.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let entry = saved_entry(&mut service, &session);
    let before = std::fs::read(&path).unwrap();
    let snapshot = profile.coordinator.snapshot(&session.database).unwrap();
    let policy = DatabasePolicy::new(512 * 1024, 1024 * 1024, 750).unwrap();
    let operation = Digest::of(b"PUBLIC corrected policy password");
    for rejected in [b"PUBLIC incorrect current password".as_slice(), b""] {
        assert!(matches!(
            service.set_database_policy(&session, operation, policy, Some(rejected)),
            Err(ServiceError::InvalidInput)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            profile
                .coordinator
                .snapshot(&session.database)
                .unwrap()
                .fingerprint(),
            snapshot.fingerprint()
        );
        assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
        assert_eq!(
            service
                .database_policy(&session)
                .unwrap()
                .value
                .kdf_target_ms(),
            500
        );
    }
    service
        .set_database_policy(&session, operation, policy, Some(PASSWORD))
        .unwrap();
    let applied = profile.coordinator.snapshot(&session.database).unwrap();
    assert_eq!(
        applied.chain().head().epoch,
        snapshot.chain().head().epoch + 1
    );
    assert_eq!(service.database_policy(&session).unwrap().value, policy);
    let durable = std::fs::read(&path).unwrap();
    service
        .set_database_policy(&session, operation, policy, None)
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), durable);
    assert_eq!(
        profile
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint(),
        applied.fingerprint()
    );
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
}
