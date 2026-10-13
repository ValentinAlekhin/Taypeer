//! PUBLIC encrypted forms exercise atomic parent Save, retained input and exact receipts.
use std::fs;
use taypeer_core::{GroupId, GroupMetadataPatch, IconRef};
use taypeer_document::ObjectId;
use taypeer_services::{
    DatabaseService, DraftSaveOutcome, DraftTarget, FieldUpdate, ServiceError, SessionToken,
    new_operation_id,
};

const PASSWORD: &[u8] = b"PUBLIC group parent master";

fn create(
    service: &mut DatabaseService,
    session: &SessionToken,
    name: &str,
    parent: Option<GroupId>,
) -> GroupId {
    service
        .create_group(session, name.into(), parent, &new_operation_id().unwrap())
        .unwrap()
        .value
        .id
}

#[test]
fn new_group_parent_choice_keeps_reserved_identity_and_saves_all_fields_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC parent.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC parent".into(), PASSWORD)
        .unwrap();
    let first = create(&mut service, &session, "PUBLIC initial parent", None);
    let second = create(&mut service, &session, "PUBLIC chosen parent", None);
    let form = service
        .start_create_group(&session, Some(first.clone()))
        .unwrap()
        .value;
    let DraftTarget::NewGroup { group, parent } = &form.identity.target else {
        panic!("PUBLIC new group target");
    };
    let group = group.clone();
    assert_eq!(parent.as_ref(), Some(&first));
    assert_eq!(form.parent.as_ref(), Some(&first));
    let before = fs::read(&path).unwrap();
    let icon = IconRef::Lucide("folder".to_owned().try_into().unwrap());
    let input = service
        .patch_group_draft(
            &session,
            &form.identity.draft,
            GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC atomic group".into()),
                description: FieldUpdate::Set("  PUBLIC description\nТекст  ".into()),
                icon: FieldUpdate::Set(icon.clone()),
                parent: FieldUpdate::Set(second.clone()),
            },
        )
        .unwrap()
        .value;
    assert_eq!(input.draft, form.identity.draft);
    assert_eq!(input.target, form.identity.target);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        !service
            .groups(&session)
            .unwrap()
            .value
            .iter()
            .any(|row| row.id == group)
    );
    let operation = new_operation_id().unwrap();
    let saved = service
        .save_draft_snapshot(&session, &input.draft, input.revision, &operation)
        .unwrap()
        .value;
    assert!(matches!(saved, DraftSaveOutcome::Saved { .. }));
    let history = service.group_history(&session, &group).unwrap().value;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].snapshot.group.parent.as_ref(), Some(&second));
    assert_eq!(history[0].snapshot.group.name, "PUBLIC atomic group");
    assert_eq!(
        history[0].snapshot.description.as_deref(),
        Some("  PUBLIC description\nТекст  ")
    );
    assert!(history[0].snapshot.group.icon == icon);
    let current = service
        .metadata_draft(&session, &input.draft)
        .unwrap()
        .value;
    assert_eq!(current.identity.target, DraftTarget::Group(group.clone()));
    assert_eq!(current.parent.as_ref(), Some(&second));
    assert!(!current.dirty);
    let encrypted = fs::read(&path).unwrap();
    assert_ne!(encrypted, before);
    assert!(
        !encrypted
            .windows(b"PUBLIC atomic group".len())
            .any(|window| window == b"PUBLIC atomic group")
    );
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    assert_eq!(
        service
            .save_draft_snapshot(&session, &input.draft, input.revision, &operation)
            .unwrap()
            .value,
        saved
    );
    assert_eq!(fs::read(&path).unwrap(), encrypted);
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        1
    );
}

#[test]
fn pending_parent_snapshot_retry_preserves_later_root_input_through_encrypted_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC pending.taypeer");
    let sidecar = directory.path().join("PUBLIC pending.taypeer.draft");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC pending".into(), PASSWORD)
        .unwrap();
    let group = create(&mut service, &session, "PUBLIC source", None);
    let parent = create(&mut service, &session, "PUBLIC destination", None);
    let form = service.start_edit_group(&session, &group).unwrap().value;
    let captured = service
        .patch_group_draft(
            &session,
            &form.identity.draft,
            GroupMetadataPatch {
                parent: FieldUpdate::Set(parent.clone()),
                ..Default::default()
            },
        )
        .unwrap()
        .value;
    let operation = new_operation_id().unwrap();
    let original = fs::read(&path).unwrap();
    // A real sidecar publication failure leaves the exact immutable attempt unconfirmed.
    fs::create_dir(&sidecar).unwrap();
    assert_eq!(
        service
            .save_draft_snapshot(&session, &captured.draft, captured.revision, &operation)
            .err(),
        Some(ServiceError::Storage(taypeer_storage::Error::Io))
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    let later = service
        .patch_group_draft(
            &session,
            &captured.draft,
            GroupMetadataPatch {
                parent: FieldUpdate::Clear,
                ..Default::default()
            },
        )
        .unwrap()
        .value;
    assert!(later.revision > captured.revision);
    fs::remove_dir(&sidecar).unwrap();
    let saved = service
        .save_draft_snapshot(&session, &captured.draft, captured.revision, &operation)
        .unwrap()
        .value;
    let current = service
        .metadata_draft(&session, &captured.draft)
        .unwrap()
        .value;
    assert_eq!(current.identity, later);
    assert_eq!(current.parent, None);
    assert!(current.dirty);
    assert_eq!(
        service
            .groups(&session)
            .unwrap()
            .value
            .iter()
            .find(|row| row.id == group)
            .unwrap()
            .parent
            .as_ref(),
        Some(&parent)
    );
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        2
    );
    let confirmed = fs::read(&path).unwrap();
    service.persist_drafts(&session).unwrap();
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    let resumed = service
        .resume_draft(&session, &captured.draft)
        .unwrap()
        .value;
    assert_eq!(resumed, later);
    let current = service
        .metadata_draft(&session, &captured.draft)
        .unwrap()
        .value;
    assert_eq!(current.parent, None);
    assert!(current.dirty);
    assert_eq!(
        service
            .save_draft_snapshot(&session, &captured.draft, captured.revision, &operation)
            .unwrap()
            .value,
        saved
    );
    assert_eq!(fs::read(&path).unwrap(), confirmed);
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        2
    );
    service
        .save_draft_snapshot(
            &session,
            &later.draft,
            later.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    assert_eq!(
        service
            .groups(&session)
            .unwrap()
            .value
            .iter()
            .find(|row| row.id == group)
            .unwrap()
            .parent,
        None
    );
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        3
    );
}

#[test]
fn a_destination_trashed_after_selection_refuses_the_whole_save_without_binding_operation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC refused.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC refused".into(), PASSWORD)
        .unwrap();
    let group = create(&mut service, &session, "PUBLIC source", None);
    let parent = create(&mut service, &session, "PUBLIC destination", None);
    let form = service.start_edit_group(&session, &group).unwrap().value;
    let icon = IconRef::Lucide("folder".to_owned().try_into().unwrap());
    let input = service
        .patch_group_draft(
            &session,
            &form.identity.draft,
            GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC retained name".into()),
                description: FieldUpdate::Set("PUBLIC retained description".into()),
                icon: FieldUpdate::Set(icon.clone()),
                parent: FieldUpdate::Set(parent.clone()),
            },
        )
        .unwrap()
        .value;
    service
        .trash_object(
            &session,
            ObjectId::Group(parent.clone()),
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let before = fs::read(&path).unwrap();
    let operation = new_operation_id().unwrap();
    assert_eq!(
        service
            .save_draft_snapshot(&session, &input.draft, input.revision, &operation)
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    let retained = service
        .metadata_draft(&session, &input.draft)
        .unwrap()
        .value;
    assert_eq!(retained.identity, input);
    assert_eq!(retained.parent.as_ref(), Some(&parent));
    assert_eq!(retained.name, "PUBLIC retained name");
    assert_eq!(
        retained.description.as_deref(),
        Some("PUBLIC retained description")
    );
    assert!(retained.icon.as_ref() == Some(&icon));
    assert!(retained.dirty);
    assert!(
        !retained
            .parent_candidates
            .iter()
            .any(|row| row.id == parent)
    );
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        1
    );
    let corrected = service
        .patch_group_draft(
            &session,
            &input.draft,
            GroupMetadataPatch {
                parent: FieldUpdate::Clear,
                ..Default::default()
            },
        )
        .unwrap()
        .value;
    // The precommit rejection did not reserve the operation for the rejected revision.
    service
        .save_draft_snapshot(&session, &corrected.draft, corrected.revision, &operation)
        .unwrap();
    let history = service.group_history(&session, &group).unwrap().value;
    assert_eq!(history.len(), 2);
    let saved = history
        .iter()
        .find(|revision| revision.snapshot.group.name == "PUBLIC retained name")
        .unwrap();
    assert_eq!(saved.snapshot.group.parent, None);
    assert!(saved.snapshot.group.icon == icon);
}
