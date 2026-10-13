//! Actual encrypted local publication and authenticated incoming changes, with PUBLIC image bytes.
use super::*;
use crate::{BinaryTarget, IconImport};
use std::io::{self, Read};
use taypeer_core::{IconRef, IconSource, OperationId};

const FIRST: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#224466"/></svg>"##;
const SECOND: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><circle cx="10" cy="10" r="8" fill="#6688aa"/></svg>"##;
struct Unreadable;
impl Read for Unreadable {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("exact PUBLIC icon retry must precede provider access")
    }
}
fn import(
    service: &mut DatabaseService,
    session: &SessionToken,
    draft: &taypeer_core::DraftId,
    bytes: &[u8],
    operation: &OperationId,
) -> Result<SessionValue<crate::DraftIdentity>, ServiceError> {
    service.import_icon_stream(
        session,
        draft,
        bytes,
        IconImport {
            length: bytes.len() as u64,
            source: IconSource::File,
        },
        operation,
    )
}
fn current(service: &DatabaseService, session: &SessionToken) -> crate::DraftIdentity {
    service.active_draft(session).unwrap().value.unwrap()
}
fn sidecar(path: &Path, profile: &Profile) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.draft", Digest::of(&profile.seed)));
    PathBuf::from(name)
}

#[test]
fn uncertain_icon_publication_keeps_old_projection_and_retry_confirms_exact_candidate_before_provider()
 {
    for published in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let profile = Profile::new(70);
        let path = directory.path().join("PUBLIC uncertain icon.taypeer");
        let (mut seed, session) = create(&profile, path.clone());
        seed.lock(&session).unwrap();
        drop(seed);
        let failures = Arc::new(Failures::default());
        let (mut service, session) = reopen(
            &profile,
            path.clone(),
            session.database,
            Arc::clone(&failures),
        );
        begin_entry(&mut service, &session);
        service
            .patch_draft(
                &session,
                EntryPatch {
                    title: FieldUpdate::Set("PUBLIC uncertain original title".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        service.persist_drafts(&session).unwrap();
        let before = current(&service, &session);
        let original_sidecar = std::fs::read(sidecar(&path, &profile)).unwrap();
        let working = std::fs::read(&path).unwrap();
        if published {
            failures.uncertain_local_after.store(true, Ordering::SeqCst)
        } else {
            failures
                .uncertain_local_before
                .store(true, Ordering::SeqCst)
        }
        let operation = crate::new_operation_id().unwrap();
        assert!(matches!(
            import(&mut service, &session, &before.draft, FIRST, &operation),
            Err(ServiceError::Storage(StorageError::CommitUncertain))
        ));
        assert_eq!(current(&service, &session), before);
        assert!(matches!(
            service.editor_view(&session).unwrap().appearance.icon,
            IconRef::Default
        ));
        assert_eq!(std::fs::read(&path).unwrap(), working);
        assert!(!service.can_write(&session).unwrap());
        assert_eq!(
            service
                .patch_draft(
                    &session,
                    EntryPatch {
                        notes: FieldUpdate::Set("PUBLIC unrelated write must refuse".into()),
                        ..Default::default()
                    }
                )
                .err(),
            Some(ServiceError::Storage(StorageError::CommitUncertain))
        );
        assert_eq!(
            service
                .icon_import_receipt(
                    &session,
                    &before.draft,
                    &IconSource::File,
                    &crate::new_operation_id().unwrap()
                )
                .err(),
            Some(ServiceError::Storage(StorageError::CommitUncertain))
        );
        let uncertain_bytes = std::fs::read(sidecar(&path, &profile)).unwrap();
        assert_eq!(uncertain_bytes == original_sidecar, !published);
        failures.deny_local.store(true, Ordering::SeqCst);
        assert_eq!(
            service
                .icon_import_receipt(&session, &before.draft, &IconSource::File, &operation)
                .err(),
            Some(ServiceError::Storage(StorageError::Io))
        );
        assert_eq!(current(&service, &session), before);
        assert_eq!(
            std::fs::read(sidecar(&path, &profile)).unwrap(),
            uncertain_bytes
        );
        failures.deny_local.store(false, Ordering::SeqCst);
        let confirmed = service
            .icon_import_receipt(&session, &before.draft, &IconSource::File, &operation)
            .unwrap()
            .value
            .unwrap();
        assert_eq!(confirmed.revision.0, before.revision.0 + 1);
        assert_eq!(current(&service, &session), confirmed);
        assert!(service.can_write(&session).unwrap());
        assert_eq!(
            service
                .import_icon_stream(
                    &session,
                    &before.draft,
                    Unreadable,
                    IconImport {
                        length: u64::MAX,
                        source: IconSource::File
                    },
                    &operation
                )
                .unwrap()
                .value,
            confirmed
        );
        assert_eq!(std::fs::read(&path).unwrap(), working);
        assert_eq!(
            service
                .icon_preview(&session, &BinaryTarget::Form(before.draft.clone()))
                .unwrap()
                .unwrap()
                .bytes
                .as_slice(),
            FIRST
        );
        service
            .save_draft_snapshot(
                &session,
                &confirmed.draft,
                confirmed.revision,
                &crate::new_operation_id().unwrap(),
            )
            .unwrap();
        let crate::DraftTarget::NewEntry { entry, .. } = confirmed.target else {
            panic!("PUBLIC fixture must be creation")
        };
        assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    }
}

#[test]
fn lock_retains_uncertain_icon_candidate_and_revokes_access_even_when_cleanup_fails() {
    for deny_cleanup in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let profile = Profile::new(71);
        let path = directory
            .path()
            .join("PUBLIC locked uncertain icon.taypeer");
        let (mut seed, session) = create(&profile, path.clone());
        seed.lock(&session).unwrap();
        drop(seed);
        let failures = Arc::new(Failures::default());
        let (mut service, session) = reopen(
            &profile,
            path.clone(),
            session.database.clone(),
            Arc::clone(&failures),
        );
        begin_entry(&mut service, &session);
        let before = current(&service, &session);
        let operation = crate::new_operation_id().unwrap();
        failures.uncertain_local_after.store(true, Ordering::SeqCst);
        assert!(matches!(
            import(&mut service, &session, &before.draft, FIRST, &operation),
            Err(ServiceError::Storage(StorageError::CommitUncertain))
        ));
        let published = std::fs::read(sidecar(&path, &profile)).unwrap();
        failures.deny_local.store(deny_cleanup, Ordering::SeqCst);
        let locked = service.lock(&session);
        if deny_cleanup {
            assert_eq!(locked.err(), Some(ServiceError::Storage(StorageError::Io)));
            assert_eq!(std::fs::read(sidecar(&path, &profile)).unwrap(), published);
        } else {
            locked.unwrap();
        }
        assert_eq!(
            service
                .icon_import_receipt(&session, &before.draft, &IconSource::File, &operation)
                .err(),
            Some(ServiceError::Locked)
        );
        drop(service);
        failures.deny_local.store(false, Ordering::SeqCst);
        let (mut reopened, next) = reopen(&profile, path, session.database, failures);
        let receipt = reopened
            .icon_import_receipt(&next, &before.draft, &IconSource::File, &operation)
            .unwrap()
            .value
            .unwrap();
        assert_eq!(receipt.revision.0, before.revision.0 + 1);
        reopened.resume_draft(&next, &before.draft).unwrap();
        assert_eq!(
            reopened
                .icon_preview(&next, &BinaryTarget::Form(before.draft))
                .unwrap()
                .unwrap()
                .bytes
                .as_slice(),
            FIRST
        );
    }
}

#[test]
fn saved_icon_receipt_does_not_retain_stale_form_fields_after_incoming_changes() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(72);
    let member = Profile::new(73);
    let (mut authority, session) =
        create(&manager, directory.path().join("PUBLIC manager.taypeer"));
    begin_entry(&mut authority, &session);
    authority
        .patch_draft(
            &session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC original title".into()),
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
    let (mut peer, peer_session) = member.open(path.clone(), PASSWORD);
    peer.start_edit_entry(&peer_session, &entry).unwrap();
    let old_form = current(&peer, &peer_session);
    let operation = crate::new_operation_id().unwrap();
    import(&mut peer, &peer_session, &old_form.draft, FIRST, &operation).unwrap();
    peer.patch_draft(
        &peer_session,
        EntryPatch {
            notes: FieldUpdate::Set("PUBLIC member icon snapshot".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let saved = current(&peer, &peer_session);
    peer.save_draft_snapshot(
        &peer_session,
        &saved.draft,
        saved.revision,
        &crate::new_operation_id().unwrap(),
    )
    .unwrap();
    deliver(&member, &manager, &session.database);
    authority.apply_received(&session).unwrap();
    peer.lock(&peer_session).unwrap();
    drop(peer);

    authority.start_edit_entry(&session, &entry).unwrap();
    authority
        .patch_draft(
            &session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC incoming newer title".into()),
                notes: FieldUpdate::Set("PUBLIC incoming newer notes".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let incoming = current(&authority, &session);
    import(
        &mut authority,
        &session,
        &incoming.draft,
        SECOND,
        &crate::new_operation_id().unwrap(),
    )
    .unwrap();
    let incoming = current(&authority, &session);
    authority
        .save_draft_snapshot(
            &session,
            &incoming.draft,
            incoming.revision,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    deliver(&manager, &member, &session.database);
    // The locked host still owns the registered store and receives ciphertext.
    // Reopen its worker port without acquiring a second exclusive file lease.
    let (mut peer, next) = reopen(
        &member,
        path.clone(),
        session.database.clone(),
        Arc::new(Failures::default()),
    );
    peer.apply_received(&next).unwrap();
    peer.start_edit_entry(&next, &entry).unwrap();
    let fresh = current(&peer, &next);
    assert_ne!(fresh.draft, old_form.draft);
    let projected = peer.editor_view(&next).unwrap();
    assert_eq!(projected.fields.title, "PUBLIC incoming newer title");
    assert_eq!(
        projected.fields.notes.as_deref(),
        Some("PUBLIC incoming newer notes")
    );
    assert_eq!(
        peer.icon_preview(&next, &BinaryTarget::Form(fresh.draft.clone()))
            .unwrap()
            .unwrap()
            .bytes
            .as_slice(),
        SECOND
    );
    let before = std::fs::read(&path).unwrap();
    let history = peer.history(&next, &entry).unwrap().value.len();
    let receipt = peer
        .import_icon_stream(
            &next,
            &old_form.draft,
            Unreadable,
            IconImport {
                length: u64::MAX,
                source: IconSource::File,
            },
            &operation,
        )
        .unwrap()
        .value;
    assert_eq!(receipt.draft, old_form.draft);
    assert_eq!(current(&peer, &next), fresh);
    assert_eq!(
        peer.editor_view(&next).unwrap().fields.title,
        "PUBLIC incoming newer title"
    );
    assert_eq!(
        peer.icon_preview(&next, &BinaryTarget::Form(fresh.draft))
            .unwrap()
            .unwrap()
            .bytes
            .as_slice(),
        SECOND
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(peer.history(&next, &entry).unwrap().value.len(), history);
}
