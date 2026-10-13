//! PUBLIC-only named recovery: real encrypted archives, exact receipts and frozen generations.
use super::*;
use crate::{NamedRecoveryIntent, TrustRecoverySource, validate_named_trust_recovery};

fn intent(profile: &Profile, source: &TrustRecoverySource) -> NamedRecoveryIntent {
    let snapshot = source.snapshot();
    NamedRecoveryIntent {
        operation: Digest::of(b"PUBLIC named recovery operation"),
        database: snapshot.chain().head().database.clone(),
        working_copy: Digest::of(&profile.seed),
        root: snapshot.chain().root().unwrap(),
        source: snapshot.fingerprint(),
        identity: profile.identity(),
        name: "PUBLIC exact recovered name  ".into(),
    }
}

#[test]
fn named_candidate_preserves_entry_history_binary_and_source_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(31);
    let path = directory.path().join("source.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let group = service
        .create_group(
            &session,
            "PUBLIC group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC history entry".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = service
        .save_draft(&session, &crate::new_operation_id().unwrap())
        .unwrap()
        .value;
    let binary = directory.path().join("PUBLIC binary");
    std::fs::write(&binary, b"PUBLIC unchanged original attachment").unwrap();
    service
        .edit_binary(
            &session,
            &crate::BinaryRequest {
                target: crate::BinaryTarget::Entry(entry.clone()),
                review: None,
                edit: crate::BinaryEdit::Attachment(crate::AttachmentEdit::Add {
                    path: binary,
                    name: None,
                }),
            },
            &taypeer_core::OperationId::new("PUBLIC original binary operation"),
        )
        .unwrap();
    let history: Vec<_> = service
        .history(&session, &entry)
        .unwrap()
        .value
        .into_iter()
        .map(|revision| revision.id)
        .collect();
    let before = std::fs::read(&path).unwrap();
    let frozen = service.freeze_trust_recovery_source(&session).unwrap();
    let exact = intent(&profile, &frozen);
    let candidate_path = directory.path().join("candidate.taypeer");
    let prepared = frozen.bind_named(exact.clone(), NEW_PASSWORD).unwrap();
    assert!(prepared.matches(&exact, NEW_PASSWORD));
    let seed = prepared.prepare(&profile.author()).unwrap();
    let root = seed.controls[0].hash().unwrap();
    drop(
        seed.create(&candidate_path, &profile.transport, None)
            .unwrap(),
    );
    let candidate = ArchiveSnapshot::open(&candidate_path, Some(root)).unwrap();
    assert_eq!(
        validate_named_trust_recovery(&frozen.snapshot(), &candidate, &exact, NEW_PASSWORD)
            .unwrap(),
        root
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let bytes = std::fs::read(&candidate_path).unwrap();
    assert!(
        !bytes
            .windows(exact.name.len())
            .any(|part| part == exact.name.as_bytes())
    );
    service.lock(&session).unwrap();
    drop(service);
    profile.coordinator.unregister(&session.database).unwrap();
    let (recovered, opened) = profile.open(candidate_path, NEW_PASSWORD);
    assert_eq!(recovered.database_info(&opened).unwrap().name, exact.name);
    assert_eq!(recovered.entries(&opened, None, "").unwrap().value.len(), 1);
    assert_eq!(
        recovered
            .history(&opened, &entry)
            .unwrap()
            .value
            .into_iter()
            .map(|revision| revision.id)
            .collect::<Vec<_>>(),
        history
    );
    let view = recovered
        .binary_view(&opened, &crate::BinaryTarget::Entry(entry.clone()))
        .unwrap()
        .value;
    let output = directory.path().join("PUBLIC recovered attachment");
    recovered
        .export_binary(
            &opened,
            &crate::BinaryTarget::Entry(entry),
            &view.attachments[0].contents[0].id,
            &output,
            false,
        )
        .unwrap();
    assert_eq!(
        std::fs::read(output).unwrap(),
        b"PUBLIC unchanged original attachment"
    );
}

#[test]
fn frozen_recovery_does_not_substitute_a_later_incoming_document_generation() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(32);
    let receiver = Profile::new(33);
    let (mut original, original_session) =
        create(&manager, directory.path().join("manager.taypeer"));
    admit(&mut original, &original_session, &manager, &receiver);
    let source_path = directory.path().join("receiver.taypeer");
    copy_to(&manager, &original_session.database, &source_path);
    let (mut source, session) = receiver.open(source_path.clone(), PASSWORD);
    let frozen = source.freeze_trust_recovery_source(&session).unwrap();
    let original_fingerprint = frozen.snapshot().fingerprint();
    let exact = intent(&receiver, &frozen);
    original
        .create_group(
            &original_session,
            "PUBLIC later incoming group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    deliver(&manager, &receiver, &session.database);
    assert!(source.apply_received(&session).unwrap().value.applied > 0);
    assert_eq!(source.groups(&session).unwrap().value.len(), 1);
    assert_ne!(
        receiver
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint(),
        original_fingerprint
    );
    let incoming_bytes = std::fs::read(&source_path).unwrap();
    let candidate_path = directory.path().join("candidate.taypeer");
    let seed = frozen
        .bind_named(exact.clone(), NEW_PASSWORD)
        .unwrap()
        .prepare(&receiver.author())
        .unwrap();
    drop(
        seed.create(&candidate_path, &receiver.transport, None)
            .unwrap(),
    );
    let candidate = ArchiveSnapshot::open(&candidate_path, None).unwrap();
    validate_named_trust_recovery(&frozen.snapshot(), &candidate, &exact, NEW_PASSWORD).unwrap();
    assert_eq!(std::fs::read(&source_path).unwrap(), incoming_bytes);
    source.lock(&session).unwrap();
    drop(source);
    receiver.coordinator.unregister(&session.database).unwrap();
    let (recovered, opened) = receiver.open(candidate_path, NEW_PASSWORD);
    assert!(recovered.groups(&opened).unwrap().value.is_empty());
    assert_eq!(recovered.database_info(&opened).unwrap().name, exact.name);
}

#[test]
fn receipt_requires_exact_name_full_identity_source_password_and_named_schema() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(34);
    let (service, session) = create(&profile, directory.path().join("source.taypeer"));
    let frozen = service.freeze_trust_recovery_source(&session).unwrap();
    let exact = intent(&profile, &frozen);
    let path = directory.path().join("candidate.taypeer");
    drop(
        frozen
            .bind_named(exact.clone(), NEW_PASSWORD)
            .unwrap()
            .prepare(&profile.author())
            .unwrap()
            .create(&path, &profile.transport, None)
            .unwrap(),
    );
    let candidate = ArchiveSnapshot::open(&path, None).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut changed_name = exact.clone();
    changed_name.name.push('x');
    let mut changed_transport = exact.clone();
    changed_transport.identity = Identity::new(
        profile.author().public(),
        TransportKey::from_seed(&[99; 32]).public(),
    )
    .unwrap();
    let mut changed_source = exact.clone();
    changed_source.source = Digest::of(b"PUBLIC other generation");
    let mut changed_operation = exact.clone();
    changed_operation.operation = Digest::of(b"PUBLIC other operation");
    for changed in [
        changed_name,
        changed_transport,
        changed_source,
        changed_operation,
    ] {
        assert!(matches!(
            validate_named_trust_recovery(&frozen.snapshot(), &candidate, &changed, NEW_PASSWORD),
            Err(ServiceError::Trust(taypeer_trust::Error::OperationMismatch))
        ));
    }
    assert!(
        validate_named_trust_recovery(&frozen.snapshot(), &candidate, &exact, PASSWORD).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let legacy_path = directory.path().join("legacy.taypeer");
    drop(
        service
            .prepare_trust_recovery(&session, NEW_PASSWORD, profile.identity(), exact.operation)
            .unwrap()
            .create(&legacy_path, &profile.transport, None)
            .unwrap(),
    );
    let legacy = ArchiveSnapshot::open(&legacy_path, None).unwrap();
    assert!(matches!(
        validate_named_trust_recovery(&frozen.snapshot(), &legacy, &exact, NEW_PASSWORD),
        Err(ServiceError::Trust(taypeer_trust::Error::OperationMismatch))
    ));
}

#[test]
fn invalid_input_and_stale_source_refuse_without_changing_ciphertext() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(35);
    let path = directory.path().join("source.taypeer");
    let (service, session) = create(&profile, path.clone());
    let before = std::fs::read(&path).unwrap();
    let source = service.freeze_trust_recovery_source(&session).unwrap();
    let exact = intent(&profile, &source);
    assert!(matches!(
        source.validate_input("", NEW_PASSWORD),
        Err(ServiceError::InvalidInput)
    ));
    assert!(matches!(
        source.validate_input(&exact.name, PASSWORD),
        Err(ServiceError::InvalidInput)
    ));
    assert!(matches!(
        source.validate_input(&exact.name, b""),
        Err(ServiceError::InvalidInput)
    ));
    let mut changed = exact;
    changed.source = Digest::of(b"PUBLIC stale snapshot");
    assert!(matches!(
        source.bind_named(changed, NEW_PASSWORD),
        Err(ServiceError::Storage(StorageError::Changed))
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn readable_signed_fork_recovers_without_removing_old_chain_guards_or_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(36);
    let competing = Profile::new(36); // PUBLIC rolled-back signer fixture.
    let receiver = Profile::new(37);
    let (mut left, left_session) = create(&manager, directory.path().join("left.taypeer"));
    left.create_group(
        &left_session,
        "PUBLIC preserved fork data".into(),
        None,
        &crate::new_operation_id().unwrap(),
    )
    .unwrap();
    admit(&mut left, &left_session, &manager, &receiver);
    let right_path = directory.path().join("right.taypeer");
    let receiver_path = directory.path().join("receiver.taypeer");
    copy_to(&manager, &left_session.database, &right_path);
    copy_to(&manager, &left_session.database, &receiver_path);
    let (mut right, right_session) = competing.open(right_path, PASSWORD);
    let (mut source, source_session) = receiver.open(receiver_path.clone(), PASSWORD);
    left.set_database_policy(
        &left_session,
        Digest::of(b"PUBLIC left recovery fork"),
        DatabasePolicy::new(512 * 1024, 2 * 1024 * 1024, 500).unwrap(),
        None,
    )
    .unwrap();
    right
        .set_database_policy(
            &right_session,
            Digest::of(b"PUBLIC right recovery fork"),
            DatabasePolicy::new(256 * 1024, 2 * 1024 * 1024, 500).unwrap(),
            None,
        )
        .unwrap();
    deliver(&manager, &receiver, &source_session.database);
    source.apply_received(&source_session).unwrap();
    let conflicting = competing
        .coordinator
        .snapshot(&right_session.database)
        .unwrap();
    assert!(
        receiver
            .coordinator
            .command(
                competing.transport.public(),
                Command::Offer(Box::new(conflicting.metadata().clone()))
            )
            .is_err()
    );
    source.lock(&source_session).unwrap();
    drop(source);
    receiver
        .coordinator
        .unregister(&source_session.database)
        .unwrap();
    let (mut source, source_session) = receiver.open(receiver_path.clone(), PASSWORD);
    let original = std::fs::read(&receiver_path).unwrap();
    let evidence = receiver
        .coordinator
        .snapshot(&source_session.database)
        .unwrap();
    assert!(!evidence.metadata().journal.forks.is_empty());
    assert!(!source.can_write(&source_session).unwrap());
    assert!(source.apply_received(&source_session).is_err());
    assert!(
        source
            .create_group(
                &source_session,
                "PUBLIC forbidden old-chain write".into(),
                None,
                &crate::new_operation_id().unwrap()
            )
            .is_err()
    );
    let frozen = source
        .freeze_trust_recovery_source(&source_session)
        .unwrap();
    let exact = intent(&receiver, &frozen);
    let candidate_path = directory.path().join("recovered.taypeer");
    drop(
        frozen
            .bind_named(exact.clone(), NEW_PASSWORD)
            .unwrap()
            .prepare(&receiver.author())
            .unwrap()
            .create(&candidate_path, &receiver.transport, None)
            .unwrap(),
    );
    let candidate = ArchiveSnapshot::open(&candidate_path, None).unwrap();
    validate_named_trust_recovery(&frozen.snapshot(), &candidate, &exact, NEW_PASSWORD).unwrap();
    assert_eq!(std::fs::read(&receiver_path).unwrap(), original);
    assert_eq!(
        receiver
            .coordinator
            .snapshot(&source_session.database)
            .unwrap()
            .metadata()
            .journal
            .forks,
        evidence.metadata().journal.forks
    );
    assert!(!source.can_write(&source_session).unwrap());
    source.lock(&source_session).unwrap();
    drop(source);
    receiver
        .coordinator
        .unregister(&source_session.database)
        .unwrap();
    let (recovered, opened) = receiver.open(candidate_path, NEW_PASSWORD);
    assert_eq!(recovered.groups(&opened).unwrap().value.len(), 1);
    assert!(recovered.can_write(&opened).unwrap());
    assert_eq!(recovered.database_info(&opened).unwrap().name, exact.name);
}
