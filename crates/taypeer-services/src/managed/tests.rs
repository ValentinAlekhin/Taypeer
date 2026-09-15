//! All keys, passwords and contents in this module are deliberately PUBLIC synthetic fixtures.
use super::*;
use std::{path::PathBuf, sync::Arc};
use taypeer_storage::ArchiveStore;
use taypeer_sync::{Backend, Command, Coordinator, CoordinatorPersistence, Reply};
use taypeer_trust::{JoinProof, TransportKey};

const PASSWORD: &[u8] = b"PUBLIC managed fixture password";
const NEW_PASSWORD: &[u8] = b"PUBLIC independently rotated fixture password";

mod compatibility;

#[test]
fn receipt_revocation_and_kdf_changes_never_manage_backup_paths() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(28);
    let b = Profile::new(29);
    let path = directory.path().join("a.taypeer");
    let (mut service, session) = create(&a, path.clone());
    service
        .create_group(&session, "PUBLIC local group".into(), None)
        .unwrap();
    admit(&mut service, &session, &a, &b);
    let peer_path = directory.path().join("b.taypeer");
    copy_to(&a, &session.database, &peer_path);
    let (mut peer, peer_session) = b.open(peer_path, PASSWORD);
    peer.create_group(&peer_session, "PUBLIC incoming group".into(), None)
        .unwrap();
    deliver(&b, &a, &session.database);
    service.apply_received(&session).unwrap();
    assert_eq!(service.groups(&session).unwrap().value.len(), 2);
    assert!(std::fs::read_dir(directory.path()).unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        !name.contains(".backups") && !name.contains(".before-")
    }));

    // Existing copies, even beyond the former ten-file limit, are user-owned files.
    let backups = directory.path().join("a.taypeer.backups");
    std::fs::create_dir(&backups).unwrap();
    for index in 0..12 {
        std::fs::write(
            backups.join(format!("{index:020}.taypeer")),
            b"PUBLIC old copy",
        )
        .unwrap();
    }
    let revoke = Digest::of(b"PUBLIC revoke without a backup");
    let previous = directory
        .path()
        .join(format!("a.taypeer.before-{revoke}.taypeer"));
    std::fs::write(&previous, b"PUBLIC unrelated old before file").unwrap();
    service
        .rotate_password(&session, revoke, NEW_PASSWORD, Some(b.identity().device))
        .unwrap();
    let rotated = a
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();
    service
        .rotate_password(&session, revoke, NEW_PASSWORD, Some(b.identity().device))
        .unwrap();
    assert_eq!(
        a.coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint(),
        rotated
    );
    assert_eq!(
        std::fs::read(&previous).unwrap(),
        b"PUBLIC unrelated old before file"
    );
    assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 12);
    for entry in std::fs::read_dir(&backups).unwrap() {
        assert_eq!(
            std::fs::read(entry.unwrap().path()).unwrap(),
            b"PUBLIC old copy"
        );
    }

    std::fs::rename(&backups, directory.path().join("PUBLIC untouched copies")).unwrap();
    std::fs::write(&backups, b"PUBLIC unrelated file").unwrap();
    let kdf = Digest::of(b"PUBLIC KDF without a backup");
    let policy = DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 600).unwrap();
    service
        .set_database_policy(&session, kdf, policy, Some(NEW_PASSWORD))
        .unwrap();
    let changed = a
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();
    service
        .set_database_policy(&session, kdf, policy, None)
        .unwrap();
    assert_eq!(
        a.coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint(),
        changed
    );
    assert!(
        !directory
            .path()
            .join(format!("a.taypeer.before-{kdf}.taypeer"))
            .exists()
    );
    assert_eq!(std::fs::read(&backups).unwrap(), b"PUBLIC unrelated file");
    let usage = service.storage_usage(&session).unwrap().value;
    assert_eq!(usage.file_bytes, std::fs::metadata(&path).unwrap().len());
    assert!(
        serde_json::to_value(usage)
            .unwrap()
            .get("backup_bytes")
            .is_none()
    );
    service.lock(&session).unwrap();
    drop(service);
    a.coordinator.unregister(&session.database).unwrap();
    let (service, session) = a.open(path, NEW_PASSWORD);
    assert_eq!(service.groups(&session).unwrap().value.len(), 2);
}

#[test]
fn recovery_of_an_incomplete_inventory_preserves_the_collection_hold() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(26);
    let b = Profile::new(27);
    let (mut sa, ta) = create(&a, directory.path().join("a.taypeer"));
    admit(&mut sa, &ta, &a, &b);
    let bp = directory.path().join("b.taypeer");
    copy_to(&a, &ta.database, &bp);
    let (mut sb, tb) = b.open(bp, PASSWORD);
    sb.create_group(&tb, "PUBLIC announced but not delivered".into(), None)
        .unwrap();
    let incoming = b.coordinator.snapshot(&ta.database).unwrap();
    let reply = a
        .coordinator
        .command(
            b.transport.public(),
            Command::Offer(Box::new(incoming.metadata().clone())),
        )
        .unwrap();
    assert!(matches!(reply, Reply::Needed(ids) if !ids.is_empty()));
    assert!(sa.collect_received(&ta).unwrap().value.held);
    let recovered_path = directory.path().join("recovered.taypeer");
    sa.prepare_trust_recovery(
        &ta,
        NEW_PASSWORD,
        a.identity(),
        Digest::of(b"PUBLIC incomplete recovery"),
    )
    .unwrap()
    .create(&recovered_path, &a.transport, None)
    .unwrap();
    sa.lock(&ta).unwrap();
    drop(sa);
    a.coordinator.unregister(&ta.database).unwrap();
    let (mut recovered, session) = a.open(recovered_path, NEW_PASSWORD);
    assert!(recovered.can_write(&session).unwrap());
    assert!(recovered.collect_received(&session).unwrap().value.held);
}

#[test]
fn signed_fork_freezes_receivers_and_survives_restart_without_blocking_reads() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(23);
    let competing = Profile::new(23); // PUBLIC rollback simulation: the same signing identity.
    let receiver = Profile::new(24);
    let witness = Profile::new(25);
    let (mut left, session) = create(&manager, directory.path().join("left.taypeer"));
    left.create_group(&session, "PUBLIC fork history".into(), None)
        .unwrap();
    admit(&mut left, &session, &manager, &receiver);
    admit(&mut left, &session, &manager, &witness);
    let right_path = directory.path().join("right.taypeer");
    let receiver_path = directory.path().join("receiver.taypeer");
    let witness_path = directory.path().join("witness.taypeer");
    copy_to(&manager, &session.database, &right_path);
    copy_to(&manager, &session.database, &receiver_path);
    copy_to(&manager, &session.database, &witness_path);
    let (mut right, rt) = competing.open(right_path, PASSWORD);
    let (mut received, token) = receiver.open(receiver_path.clone(), PASSWORD);
    let (mut seen, wt) = witness.open(witness_path, PASSWORD);
    left.set_database_policy(
        &session,
        Digest::of(b"PUBLIC left policy"),
        DatabasePolicy::new(512 * 1024, 2 * 1024 * 1024, 500).unwrap(),
        None,
    )
    .unwrap();
    right
        .set_database_policy(
            &rt,
            Digest::of(b"PUBLIC right policy"),
            DatabasePolicy::new(256 * 1024, 2 * 1024 * 1024, 500).unwrap(),
            None,
        )
        .unwrap();
    deliver(&manager, &receiver, &session.database);
    received.apply_received(&token).unwrap();
    let other = competing.coordinator.snapshot(&session.database).unwrap();
    assert!(
        receiver
            .coordinator
            .command(
                competing.transport.public(),
                Command::Offer(Box::new(other.metadata().clone()))
            )
            .is_err()
    );
    assert!(
        !receiver
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .metadata()
            .journal
            .forks
            .is_empty()
    );
    assert!(
        received
            .create_group(&token, "PUBLIC forbidden write".into(), None)
            .is_err()
    );
    assert_eq!(received.groups(&token).unwrap().value.len(), 1);
    let evidence = receiver.coordinator.snapshot(&session.database).unwrap();
    assert!(
        witness
            .coordinator
            .command(
                receiver.transport.public(),
                Command::Offer(Box::new(evidence.metadata().clone()))
            )
            .is_err()
    );
    assert!(
        !witness
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .metadata()
            .journal
            .forks
            .is_empty()
    );
    assert!(seen.apply_received(&wt).is_err());
    received.lock(&token).unwrap();
    drop(received);
    receiver.coordinator.unregister(&session.database).unwrap();
    let (mut reopened, token) = receiver.open(receiver_path, PASSWORD);
    assert_eq!(reopened.groups(&token).unwrap().value.len(), 1);
    assert!(reopened.apply_received(&token).is_err());
}

#[test]
fn handoff_survives_restart_before_successor_delivery_and_rejects_old_manager() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(21);
    let b = Profile::new(22);
    let ap = directory.path().join("a.taypeer");
    let (mut sa, ta) = create(&a, ap.clone());
    sa.create_group(&ta, "PUBLIC management history".into(), None)
        .unwrap();
    admit(&mut sa, &ta, &a, &b);
    let bp = directory.path().join("b.taypeer");
    copy_to(&a, &ta.database, &bp);
    let (mut sb, tb) = b.open(bp, PASSWORD);
    let operation = Digest::of(b"PUBLIC interrupted handoff");
    let consent = sb.consent_management(&tb, operation).unwrap();
    sa.transfer_management(&ta, consent.clone()).unwrap();
    let committed = a.coordinator.snapshot(&ta.database).unwrap().fingerprint();
    assert_eq!(
        a.coordinator
            .snapshot(&ta.database)
            .unwrap()
            .chain()
            .head()
            .manager,
        b.identity().device
    );
    assert!(
        sa.rotate_password(
            &ta,
            Digest::of(b"PUBLIC former manager"),
            NEW_PASSWORD,
            None
        )
        .is_err()
    );
    sa.lock(&ta).unwrap();
    drop(sa);
    a.coordinator.unregister(&ta.database).unwrap();
    let (mut sa, ta) = a.open(ap, PASSWORD);
    sa.transfer_management(&ta, consent).unwrap();
    assert_eq!(
        a.coordinator.snapshot(&ta.database).unwrap().fingerprint(),
        committed
    );
    deliver(&a, &b, &ta.database);
    sb.apply_received(&tb).unwrap();
    sb.rotate_password(
        &tb,
        Digest::of(b"PUBLIC successor rotation"),
        NEW_PASSWORD,
        None,
    )
    .unwrap();
    assert_eq!(sb.groups(&tb).unwrap().value.len(), 1);
    deliver(&b, &a, &ta.database);
    assert!(
        sa.create_group(&ta, "PUBLIC expired session".into(), None)
            .is_err()
    );
}

#[test]
fn recovery_preserves_original_history_and_pending_sources_without_inheriting_admission() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(18);
    let b = Profile::new(19);
    let c = Profile::new(20);
    let (mut sa, ta) = create(&a, directory.path().join("source.taypeer"));
    let group = sa
        .create_group(&ta, "PUBLIC preserved group".into(), None)
        .unwrap()
        .value
        .id;
    sa.start_create_entry(&ta, group.clone()).unwrap();
    sa.update_draft(
        &ta,
        EditableEntry {
            title: "PUBLIC preserved entry".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let entry = sa.save_draft(&ta).unwrap().value;
    let attachment_path = directory.path().join("PUBLIC recovery attachment");
    std::fs::write(&attachment_path, b"PUBLIC inherited binary").unwrap();
    sa.edit_binary(
        &ta,
        &crate::BinaryRequest {
            target: crate::BinaryTarget::Entry(entry.clone()),
            review: None,
            edit: crate::BinaryEdit::Attachment(crate::AttachmentEdit::Add {
                path: attachment_path,
                name: None,
            }),
        },
        &taypeer_core::OperationId::new("PUBLIC recovery binary"),
    )
    .unwrap();
    admit(&mut sa, &ta, &a, &b);
    let bp = directory.path().join("b.taypeer");
    copy_to(&a, &ta.database, &bp);
    let (mut sb, tb) = b.open(bp, PASSWORD);
    sb.create_group(&tb, "PUBLIC not accepted before recovery".into(), None)
        .unwrap();
    deliver(&b, &a, &ta.database);
    let copied = directory.path().join("copy.taypeer");
    copy_to(&a, &ta.database, &copied);
    let (mut reader, token) = c.open(copied.clone(), PASSWORD);
    assert!(!reader.can_write(&token).unwrap());
    let original = std::fs::read(&copied).unwrap();
    let pending = reader.received_sources(&token).unwrap().value;
    assert_eq!(pending.len(), 1);
    let recovered_path = directory.path().join("recovered.taypeer");
    let seed = reader
        .prepare_trust_recovery(
            &token,
            NEW_PASSWORD,
            c.identity(),
            Digest::of(b"PUBLIC recover trust"),
        )
        .unwrap();
    let new_root = seed.controls[0].hash().unwrap();
    assert_ne!(
        new_root,
        a.coordinator
            .snapshot(&ta.database)
            .unwrap()
            .chain()
            .root()
            .unwrap()
    );
    assert_eq!(seed.controls[0].body.members.len(), 1);
    assert_eq!(seed.controls[0].body.manager, c.identity().device);
    seed.create(&recovered_path, &c.transport, None).unwrap();
    assert_eq!(
        reader
            .trust_recovery_retry(
                &token,
                &recovered_path,
                NEW_PASSWORD,
                &c.identity(),
                Digest::of(b"PUBLIC recover trust")
            )
            .unwrap(),
        new_root
    );
    assert!(
        reader
            .trust_recovery_retry(
                &token,
                &recovered_path,
                NEW_PASSWORD,
                &c.identity(),
                Digest::of(b"PUBLIC different recovery")
            )
            .is_err()
    );
    assert!(
        reader
            .trust_recovery_retry(
                &token,
                &recovered_path,
                PASSWORD,
                &c.identity(),
                Digest::of(b"PUBLIC recover trust")
            )
            .is_err()
    );
    assert_eq!(std::fs::read(&copied).unwrap(), original);
    reader.lock(&token).unwrap();
    drop(reader);
    c.coordinator.unregister(&token.database).unwrap();
    let (mut recovered, session) = c.open(recovered_path, NEW_PASSWORD);
    assert_eq!(session.database, token.database);
    assert!(recovered.can_write(&session).unwrap());
    assert_eq!(recovered.history(&session, &entry).unwrap().value.len(), 2);
    assert_eq!(recovered.apply_received(&session).unwrap().value.applied, 0);
    assert_eq!(recovered.groups(&session).unwrap().value.len(), 1);
    assert_eq!(
        recovered.received_sources(&session).unwrap().value[0].change,
        pending[0].change
    );
    recovered
        .create_group(&session, "PUBLIC new trust author".into(), None)
        .unwrap();
    let collection = recovered.collect_received(&session).unwrap().value;
    assert!(!collection.held);
    let view = recovered
        .binary_view(&session, &crate::BinaryTarget::Entry(entry.clone()))
        .unwrap()
        .value;
    let output = directory.path().join("PUBLIC exported recovered binary");
    recovered
        .export_binary(
            &session,
            &crate::BinaryTarget::Entry(entry.clone()),
            &view.attachments[0].contents[0].id,
            &output,
            false,
        )
        .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"PUBLIC inherited binary");
    assert_eq!(
        recovered.received_sources(&session).unwrap().value[0].change,
        pending[0].change
    );
    let second_path = directory.path().join("recovered-again.taypeer");
    recovered
        .prepare_trust_recovery(
            &session,
            PASSWORD,
            c.identity(),
            Digest::of(b"PUBLIC second recovery"),
        )
        .unwrap()
        .create(&second_path, &c.transport, None)
        .unwrap();
    recovered.lock(&session).unwrap();
    drop(recovered);
    c.coordinator.unregister(&session.database).unwrap();
    let (mut twice, session) = c.open(second_path, PASSWORD);
    assert_eq!(twice.apply_received(&session).unwrap().value.applied, 0);
    assert_eq!(twice.history(&session, &entry).unwrap().value.len(), 2);
    assert!(!twice.collect_received(&session).unwrap().value.held);
    assert_eq!(
        twice.received_sources(&session).unwrap().value[0].change,
        pending[0].change
    );
}

#[test]
fn collection_preserves_history_then_releases_purged_binary_contents() {
    use crate::{AttachmentEdit, BinaryEdit, BinaryRequest, BinaryTarget};
    use taypeer_core::OperationId;
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(17);
    let path = directory.path().join("binary.taypeer");
    let (mut service, session) = create(&a, path.clone());
    let group = service
        .create_group(&session, "PUBLIC group".into(), None)
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC entry".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = service.save_draft(&session).unwrap().value;
    let input = directory.path().join("PUBLIC contents");
    std::fs::write(&input, vec![b'P'; 128 * 1024]).unwrap();
    let target = BinaryTarget::Entry(entry.clone());
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target: target.clone(),
                review: None,
                edit: BinaryEdit::Attachment(AttachmentEdit::Add {
                    path: input,
                    name: Some("PUBLIC attachment".into()),
                }),
            },
            &OperationId::new("PUBLIC add for collection"),
        )
        .unwrap();
    let view = service.binary_view(&session, &target).unwrap().value;
    let attachment = view.attachments[0].id.clone();
    let original = service
        .history(&session, &entry)
        .unwrap()
        .value
        .last()
        .unwrap()
        .id
        .clone();
    let before = a.coordinator.snapshot(&session.database).unwrap();
    let blob = before
        .metadata()
        .manifest
        .body
        .objects
        .values()
        .find(|o| o.kind == ObjectKind::Blob)
        .unwrap()
        .digest;
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target,
                review: None,
                edit: BinaryEdit::Attachment(AttachmentEdit::Remove { attachment }),
            },
            &OperationId::new("PUBLIC remove attachment"),
        )
        .unwrap();
    service.collect_received(&session).unwrap();
    assert!(
        a.coordinator
            .snapshot(&session.database)
            .unwrap()
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&blob)
    );
    service
        .purge_history(
            &session,
            &entry,
            BTreeSet::from([original]),
            &OperationId::new("PUBLIC purge binary revision"),
        )
        .unwrap();
    let result = service.collect_received(&session).unwrap().value;
    assert!(!result.held);
    assert!(result.removed_ciphertext_bytes > 128 * 1024);
    assert!(
        !a.coordinator
            .snapshot(&session.database)
            .unwrap()
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&blob)
    );
    let compacted = a
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();
    assert_eq!(
        service
            .collect_received(&session)
            .unwrap()
            .value
            .removed_objects,
        0
    );
    assert_eq!(
        compacted,
        a.coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint()
    );
    service.lock(&session).unwrap();
    drop(service);
    a.coordinator.unregister(&session.database).unwrap();
    let (service, session) = a.open(path, PASSWORD);
    assert_eq!(service.entries(&session, None, "").unwrap().value.len(), 1);
}

#[test]
fn received_source_inspection_extraction_and_discard_survive_repackaging_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(14);
    let b = Profile::new(15);
    let c = Profile::new(16);
    let ap = directory.path().join("a.taypeer");
    let (mut sa, ta) = create(&a, ap.clone());
    let group = sa
        .create_group(&ta, "PUBLIC destination".into(), None)
        .unwrap()
        .value
        .id;
    admit(&mut sa, &ta, &a, &b);
    admit(&mut sa, &ta, &a, &c);
    let bp = directory.path().join("b.taypeer");
    let cp = directory.path().join("c.taypeer");
    copy_to(&a, &ta.database, &bp);
    copy_to(&a, &ta.database, &cp);
    let (mut sb, tb) = b.open(bp, PASSWORD);
    let (mut sc, tc) = c.open(cp, PASSWORD);
    sb.start_create_entry(&tb, group.clone()).unwrap();
    sb.update_draft(
        &tb,
        EditableEntry {
            title: "PUBLIC unaccepted entry".into(),
            password: Some("PUBLIC_UNACCEPTED_PASSWORD".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let original = sb.save_draft(&tb).unwrap().value;
    deliver(&b, &c, &ta.database);
    sc.apply_received(&tc).unwrap();
    sa.rotate_password(
        &ta,
        Digest::of(b"PUBLIC extraction revoke"),
        NEW_PASSWORD,
        Some(b.identity().device),
    )
    .unwrap();
    deliver(&c, &a, &ta.database);
    assert_eq!(sa.apply_received(&ta).unwrap().value.applied, 0);
    let sources = sa.received_sources(&ta).unwrap().value;
    assert_eq!(sources.len(), 1);
    let change = &sources[0].change;
    let before = a.coordinator.snapshot(&ta.database).unwrap().fingerprint();
    let view = sa.inspect_received(&ta, change).unwrap().value;
    assert_eq!(view[0].id, original);
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("PUBLIC_UNACCEPTED_PASSWORD")
    );
    assert_eq!(
        sa.reveal_received(&ta, change, &original)
            .unwrap()
            .value
            .expose(),
        "PUBLIC_UNACCEPTED_PASSWORD"
    );
    assert!(sa.entries(&ta, None, "").unwrap().value.is_empty());
    assert_eq!(
        before,
        a.coordinator.snapshot(&ta.database).unwrap().fingerprint()
    );
    let operation = taypeer_core::OperationId::new("PUBLIC extract exact source");
    let extracted = sa
        .extract_received(&ta, change, &original, group.clone(), &operation)
        .unwrap()
        .value;
    assert_ne!(extracted, original);
    assert_eq!(sa.history(&ta, &extracted).unwrap().value.len(), 1);
    assert_eq!(
        sa.extract_received(&ta, change, &original, group.clone(), &operation)
            .unwrap()
            .value,
        extracted
    );
    sa.discard_received(&ta, change).unwrap();
    let discarded = a.coordinator.snapshot(&ta.database).unwrap().fingerprint();
    sa.discard_received(&ta, change).unwrap();
    assert_eq!(
        discarded,
        a.coordinator.snapshot(&ta.database).unwrap().fingerprint()
    );
    assert!(sa.inspect_received(&ta, change).is_err());
    let collected = sa.collect_received(&ta).unwrap().value;
    assert!(!collected.held);
    assert!(collected.removed_objects > 0);
    sa.lock(&ta).unwrap();
    drop(sa);
    a.coordinator.unregister(&ta.database).unwrap();
    let (mut sa, ta) = a.open(ap, NEW_PASSWORD);
    deliver(&c, &a, &ta.database);
    assert_eq!(sa.apply_received(&ta).unwrap().value.applied, 0);
    assert_eq!(sa.entries(&ta, None, "").unwrap().value.len(), 1);
    assert!(
        sa.received_sources(&ta)
            .unwrap()
            .value
            .iter()
            .all(|source| source.discarded)
    );
    assert_eq!(
        sa.extract_received(&ta, change, &original, group, &operation)
            .unwrap()
            .value,
        extracted
    );
}

#[test]
fn member_checkpoint_cannot_launder_revoked_sources_or_their_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(11);
    let b = Profile::new(12);
    let c = Profile::new(13);
    let (mut sa, ta) = create(&a, directory.path().join("a.taypeer"));
    admit(&mut sa, &ta, &a, &b);
    admit(&mut sa, &ta, &a, &c);
    let bp = directory.path().join("b.taypeer");
    let cp = directory.path().join("c.taypeer");
    copy_to(&a, &ta.database, &bp);
    copy_to(&a, &ta.database, &cp);
    let (mut sb, tb) = b.open(bp, PASSWORD);
    let (mut sc, tc) = c.open(cp, PASSWORD);
    sb.create_group(&tb, "PUBLIC revoked source".into(), None)
        .unwrap();
    sc.create_group(&tc, "PUBLIC independent source".into(), None)
        .unwrap();
    deliver(&b, &c, &ta.database);
    assert_eq!(sc.apply_received(&tc).unwrap().value.applied, 1);
    sc.create_group(&tc, "PUBLIC dependent source".into(), None)
        .unwrap();
    sa.rotate_password(
        &ta,
        Digest::of(b"PUBLIC revoke B"),
        NEW_PASSWORD,
        Some(b.identity().device),
    )
    .unwrap();
    deliver(&c, &a, &ta.database);
    let report = sa.apply_received(&ta).unwrap().value;
    assert_eq!(report.applied, 1);
    assert!(
        report
            .pending
            .iter()
            .any(|packet| packet.reason == PendingReason::RevokedAuthor)
    );
    assert!(
        report
            .pending
            .iter()
            .any(|packet| packet.reason == PendingReason::Dependency)
    );
    let groups = sa.groups(&ta).unwrap().value;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "PUBLIC independent source");
    let before = a.coordinator.snapshot(&ta.database).unwrap().fingerprint();
    assert_eq!(sa.apply_received(&ta).unwrap().value.applied, 0);
    assert_eq!(
        before,
        a.coordinator.snapshot(&ta.database).unwrap().fingerprint()
    );
}

#[test]
fn managed_reauthentication_keeps_failed_session_locked_and_expires_old_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(10);
    let path = directory.path().join("reauthentication.taypeer");
    let (mut service, original) = create(&profile, path.clone());
    service
        .create_group(&original, "PUBLIC retained group".into(), None)
        .unwrap();
    service.lock(&original).unwrap();
    let port = || {
        Box::new(
            CoordinatorPersistence::new(
                Arc::clone(&profile.coordinator),
                original.database.clone(),
                path.clone(),
                Digest::of(&profile.seed),
            )
            .unwrap(),
        )
    };
    let failed = service.open_managed(port(), b"PUBLIC wrong password", || {
        panic!("Credentials accessed before authentication")
    });
    assert!(matches!(
        failed,
        Err(ServiceError::Storage(StorageError::Authentication))
    ));
    assert!(service.entries(&original, None, "").is_err());
    let reopened = service
        .open_managed(port(), PASSWORD, || Ok(Some(profile.author())))
        .unwrap();
    assert_ne!(reopened, original);
    assert!(service.entries(&original, None, "").is_err());
    assert_eq!(service.groups(&reopened).unwrap().value.len(), 1);
}

#[test]
fn administrative_retries_preserve_exact_intent_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(9);
    let path = directory.path().join("administration.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let operation = Digest::of(b"PUBLIC retry rotation");
    service
        .rotate_password(&session, operation, NEW_PASSWORD, None)
        .unwrap();
    let rotated = profile
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();
    service
        .rotate_password(&session, operation, NEW_PASSWORD, None)
        .unwrap();
    assert_eq!(
        rotated,
        profile
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint()
    );
    assert!(matches!(
        service.rotate_password(&session, operation, b"PUBLIC different intent", None),
        Err(ServiceError::Trust(taypeer_trust::Error::OperationMismatch))
    ));
    let policy_operation = Digest::of(b"PUBLIC retry KDF policy");
    let policy = DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 600).unwrap();
    service
        .set_database_policy(&session, policy_operation, policy, Some(NEW_PASSWORD))
        .unwrap();
    service.lock(&session).unwrap();
    drop(service);
    profile.coordinator.unregister(&session.database).unwrap();
    let (mut service, session) = profile.open(path, NEW_PASSWORD);
    let before = profile
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();
    service
        .set_database_policy(&session, policy_operation, policy, None)
        .unwrap();
    service
        .rotate_password(&session, operation, NEW_PASSWORD, None)
        .unwrap();
    assert_eq!(
        before,
        profile
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint()
    );
    let other = DatabasePolicy::new(512 * 1024, 2 * 1024 * 1024, 600).unwrap();
    assert!(matches!(
        service.set_database_policy(&session, policy_operation, other, None),
        Err(ServiceError::Trust(taypeer_trust::Error::OperationMismatch))
    ));
}
struct Profile {
    seed: [u8; 32],
    transport: Arc<TransportKey>,
    coordinator: Arc<Coordinator>,
}
impl Profile {
    fn new(n: u8) -> Self {
        let transport = Arc::new(TransportKey::from_seed(&[n + 40; 32]));
        let coordinator = Arc::new(Coordinator::new(Arc::clone(&transport)));
        Self {
            seed: [n; 32],
            transport,
            coordinator,
        }
    }
    fn author(&self) -> AuthorKey {
        AuthorKey::from_seed(&self.seed)
    }
    fn identity(&self) -> Identity {
        Identity::new(self.author().public(), self.transport.public()).unwrap()
    }
    fn open(&self, path: PathBuf, password: &[u8]) -> (DatabaseService, SessionToken) {
        let snapshot = taypeer_storage::ArchiveSnapshot::open(&path, None).unwrap();
        let database = snapshot.chain().head().database.clone();
        let store =
            ArchiveStore::open(&path, Some(snapshot.chain().root().unwrap()), None).unwrap();
        self.coordinator.register(store).unwrap();
        let port = CoordinatorPersistence::new(
            Arc::clone(&self.coordinator),
            database,
            path,
            Digest::of(&self.seed),
        )
        .unwrap();
        let mut service = DatabaseService::new();
        let session = service
            .open_managed(Box::new(port), password, || Ok(Some(self.author())))
            .unwrap();
        (service, session)
    }
}
fn create(profile: &Profile, path: PathBuf) -> (DatabaseService, SessionToken) {
    let seed = DatabaseService::prepare_managed(
        "PUBLIC shared file".into(),
        PASSWORD,
        &profile.author(),
        profile.identity(),
        1,
        DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap(),
    )
    .unwrap();
    let store = seed.create(&path, &profile.transport, None).unwrap();
    drop(store);
    profile.open(path, PASSWORD)
}
fn copy_to(source: &Profile, database: &DatabaseId, destination: &std::path::Path) {
    let snapshot = source.coordinator.snapshot(database).unwrap();
    snapshot
        .candidate()
        .export(snapshot.metadata().clone(), destination)
        .unwrap();
}
fn admit(
    service: &mut DatabaseService,
    session: &SessionToken,
    manager: &Profile,
    member: &Profile,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (invitation, secret) = service.create_invitation(session, now).unwrap();
    let proof = JoinProof::sign(&invitation, member.identity(), &member.author()).unwrap();
    let address =
        taypeer_sync::EndpointAddr::new(member.transport.public().to_string().parse().unwrap());
    let request = invitation.id().unwrap();
    manager
        .coordinator
        .command(
            member.transport.public(),
            Command::Join {
                invitation: Box::new(invitation),
                secret: Zeroizing::new(*secret.expose()),
                proof: Box::new(proof),
                address,
            },
        )
        .unwrap();
    assert_eq!(
        service
            .approve_invitation(session, request, now + 1)
            .unwrap(),
        member.identity().device
    );
    assert_eq!(
        service
            .approve_invitation(session, request, now + 2)
            .unwrap(),
        member.identity().device
    );
}
fn deliver(from: &Profile, to: &Profile, database: &DatabaseId) {
    let snapshot = from.coordinator.snapshot(database).unwrap();
    let reply = to
        .coordinator
        .command(
            from.transport.public(),
            Command::Offer(Box::new(snapshot.metadata().clone())),
        )
        .unwrap();
    let Reply::Needed(needed) = reply else {
        panic!("expected synthetic inventory response");
    };
    for id in needed {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        std::io::copy(&mut snapshot.reader(id).unwrap(), &mut temp).unwrap();
        to.coordinator
            .receive(
                from.transport.public(),
                database,
                snapshot.metadata().manifest.body.objects[&id].clone(),
                temp,
            )
            .unwrap();
    }
}

#[test]
fn durable_edits_copied_read_only_and_credentials_only_after_authentication() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(1);
    let path = directory.path().join("a.taypeer");
    let (mut service, session) = create(&a, path.clone());
    let group = service
        .create_group(&session, "PUBLIC group".into(), None)
        .unwrap()
        .value;
    service
        .start_create_entry(&session, group.id.clone())
        .unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC entry".into(),
                password: Some("PUBLIC value".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = service.save_draft(&session).unwrap().value;
    let before = a.coordinator.snapshot(&session.database).unwrap();
    let report = service.apply_received(&session).unwrap().value;
    assert_eq!(report.applied, 0);
    assert!(report.pending.is_empty());
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    assert_eq!(
        service
            .database_policy(&session)
            .unwrap()
            .value
            .attachment_bytes(),
        1024 * 1024
    );

    let copied = directory.path().join("copy.taypeer");
    copy_to(&a, &session.database, &copied);
    let b = Profile::new(2);
    let (mut reader, read_session) = b.open(copied.clone(), PASSWORD);
    assert!(!reader.can_write(&read_session).unwrap());
    assert!(
        reader
            .compatibility(&read_session)
            .unwrap()
            .value
            .write
            .is_supported()
    );
    assert_eq!(
        reader.entries(&read_session, None, "").unwrap().value.len(),
        1
    );
    assert!(matches!(
        reader.create_group(&read_session, "PUBLIC forbidden".into(), None),
        Err(ServiceError::ReadOnly)
    ));
    reader.lock(&read_session).unwrap();
    drop(reader);
    b.coordinator.unregister(&session.database).unwrap();

    let store = ArchiveStore::open(&copied, None, None).unwrap();
    b.coordinator.register(store).unwrap();
    let port = CoordinatorPersistence::new(
        Arc::clone(&b.coordinator),
        session.database.clone(),
        copied,
        Digest::of(b"PUBLIC copy marker"),
    )
    .unwrap();
    let called = std::cell::Cell::new(false);
    let result = DatabaseService::new().open_managed(Box::new(port), b"PUBLIC incorrect", || {
        called.set(true);
        Ok(Some(b.author()))
    });
    assert!(matches!(
        result,
        Err(ServiceError::Storage(StorageError::Authentication))
    ));
    assert!(!called.get());
    assert!(
        before
            .metadata()
            .manifest
            .body
            .objects
            .values()
            .any(|o| o.kind == ObjectKind::Change)
    );
}

#[test]
fn three_members_merge_offline_history_forward_locked_and_keep_drafts_through_rotation() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(4);
    let b = Profile::new(5);
    let c = Profile::new(6);
    let a_path = directory.path().join("a.taypeer");
    let b_path = directory.path().join("b.taypeer");
    let c_path = directory.path().join("c.taypeer");
    let (mut sa, ta) = create(&a, a_path);
    let group = sa
        .create_group(&ta, "PUBLIC group".into(), None)
        .unwrap()
        .value;
    sa.start_create_entry(&ta, group.id.clone()).unwrap();
    sa.update_draft(
        &ta,
        EditableEntry {
            title: "PUBLIC entry".into(),
            password: Some("PUBLIC base".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let entry = sa.save_draft(&ta).unwrap().value;
    admit(&mut sa, &ta, &a, &b);
    admit(&mut sa, &ta, &a, &c);
    copy_to(&a, &ta.database, &b_path);
    copy_to(&a, &ta.database, &c_path);
    let (mut sb, tb) = b.open(b_path.clone(), PASSWORD);
    let (mut sc, tc) = c.open(c_path.clone(), PASSWORD);
    sa.start_edit_entry(&ta, &entry).unwrap();
    sb.start_edit_entry(&tb, &entry).unwrap();
    let mut fa = sa.draft(&ta).unwrap().value.unwrap().fields;
    let mut fb = sb.draft(&tb).unwrap().value.unwrap().fields;
    fa.password = Some("PUBLIC offline A".into());
    fb.password = Some("PUBLIC offline B".into());
    sa.update_draft(&ta, fa).unwrap();
    sb.update_draft(&tb, fb).unwrap();
    sa.save_draft(&ta).unwrap();
    sb.save_draft(&tb).unwrap();
    sc.lock(&tc).unwrap();
    deliver(&a, &c, &ta.database);
    deliver(&c, &b, &ta.database);
    let report = sb.apply_received(&tb).unwrap().value;
    assert_eq!(report.applied, 1);
    assert!(report.pending.is_empty());
    assert_eq!(sb.history(&tb, &entry).unwrap().value.len(), 3);
    assert!(sb.view_entry(&tb, &entry).unwrap().value.has_conflicts);
    assert_eq!(sb.apply_received(&tb).unwrap().value.applied, 0);

    sb.start_create_entry(&tb, group.id).unwrap();
    sb.update_draft(
        &tb,
        EditableEntry {
            title: "PUBLIC interrupted draft".into(),
            ..Default::default()
        },
    )
    .unwrap();
    sb.lock(&tb).unwrap();
    sa.rotate_password(&ta, Digest::of(b"PUBLIC rotation"), NEW_PASSWORD, None)
        .unwrap();
    deliver(&a, &b, &ta.database);
    drop(sb);
    b.coordinator.unregister(&ta.database).unwrap();
    let (mut sb, tb) = b.open(b_path, NEW_PASSWORD);
    assert!(sb.pending_draft(&tb).unwrap().value.is_some());
    assert_eq!(sb.history(&tb, &entry).unwrap().value.len(), 3);
    sb.restore_draft(&tb).unwrap();
    assert_eq!(
        sb.draft(&tb).unwrap().value.unwrap().fields.title,
        "PUBLIC interrupted draft"
    );
    sb.cancel_draft(&tb).unwrap();
}
