//! Public synthetic forms exercising snapshot confirmation, causal continuation and local durability.
use taypeer_core::{DatabaseMetadataPatch, GroupMetadataPatch};
use taypeer_services::{
    AttachmentEdit, BinaryEdit, BinaryRequest, BinaryTarget, DatabaseService, DraftSaveOutcome,
    DraftTarget, EntryPatch, FieldUpdate, GroupForm, ServiceError, new_operation_id,
};

const PASSWORD: &[u8] = b"PUBLIC autosave master password";

#[test]
fn complete_group_forms_retry_the_same_receipt_before_and_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC group retries.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC group retries".into(), PASSWORD)
        .unwrap();
    let create = new_operation_id().unwrap();
    let creation = || GroupForm {
        id: None,
        parent: None,
        name: "PUBLIC complete group".into(),
        description: Some("PUBLIC creation description".into()),
        icon: Default::default(),
    };
    let group = service
        .save_group_form(&session, creation(), &create)
        .unwrap();
    let unchanged = std::fs::read(&path).unwrap();
    assert_eq!(
        service
            .save_group_form(&session, creation(), &create)
            .unwrap(),
        group
    );
    assert_eq!(std::fs::read(&path).unwrap(), unchanged);
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        1
    );
    let update = new_operation_id().unwrap();
    let editing = || GroupForm {
        id: Some(group.clone()),
        parent: None,
        name: "PUBLIC changed group".into(),
        description: None,
        icon: Default::default(),
    };
    assert_eq!(
        service
            .save_group_form(&session, editing(), &update)
            .unwrap(),
        group
    );
    let unchanged = std::fs::read(&path).unwrap();
    assert_eq!(
        service
            .save_group_form(&session, editing(), &update)
            .unwrap(),
        group
    );
    assert_eq!(std::fs::read(&path).unwrap(), unchanged);
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        2
    );
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    let unchanged = std::fs::read(&path).unwrap();
    assert_eq!(
        service
            .save_group_form(&session, creation(), &create)
            .unwrap(),
        group
    );
    assert_eq!(
        service
            .save_group_form(&session, editing(), &update)
            .unwrap(),
        group
    );
    assert_eq!(std::fs::read(&path).unwrap(), unchanged);
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        2
    );
    assert_eq!(
        service.group_info(&session).unwrap()[0].group.name,
        "PUBLIC changed group"
    );
}

fn title(value: &str) -> EntryPatch {
    EntryPatch {
        title: FieldUpdate::Set(value.into()),
        ..Default::default()
    }
}

#[test]
fn every_parked_draft_retains_its_binary_contents_through_collection_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC binary forms.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC binary forms".into(), PASSWORD)
        .unwrap();
    let mut retained = Vec::new();
    for contents in [
        b"PUBLIC first parked bytes".as_slice(),
        b"PUBLIC second parked bytes".as_slice(),
    ] {
        let source = directory.path().join("PUBLIC source");
        std::fs::write(&source, contents).unwrap();
        service.start_create_entry_ungrouped(&session).unwrap();
        service
            .edit_binary(
                &session,
                &BinaryRequest {
                    target: BinaryTarget::Draft,
                    edit: BinaryEdit::Attachment(AttachmentEdit::Add {
                        path: source,
                        name: None,
                    }),
                    review: None,
                },
                &new_operation_id().unwrap(),
            )
            .unwrap();
        let identity = service.editor_view(&session).unwrap().identity;
        let blob = service
            .binary_view(&session, &BinaryTarget::Draft)
            .unwrap()
            .value
            .attachments[0]
            .contents[0]
            .id
            .clone();
        retained.push((identity.draft, blob, contents.to_vec()));
    }
    service.persist_drafts(&session).unwrap();
    service
        .collect_blobs(&session, &new_operation_id().unwrap())
        .unwrap();
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    assert_eq!(service.drafts(&session).unwrap().value.len(), 2);
    for (id, blob, contents) in retained {
        service.resume_draft(&session, &id).unwrap();
        let exported = directory.path().join("PUBLIC export");
        service
            .export_binary(&session, &BinaryTarget::Draft, &blob, &exported, true)
            .unwrap();
        assert_eq!(std::fs::read(exported).unwrap(), contents);
    }
    assert!(
        service
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
}

#[test]
fn untouched_empty_forms_create_no_object_history_or_recoverable_draft() {
    let mut service = DatabaseService::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC untouched.taypeer");
    let session = service
        .create_file(&path, "PUBLIC untouched".into(), PASSWORD)
        .unwrap();
    service.start_create_entry_ungrouped(&session).unwrap();
    let identity = service.editor_view(&session).unwrap().identity;
    assert!(matches!(
        service
            .save_draft_snapshot(
                &session,
                &identity.draft,
                identity.revision,
                &new_operation_id().unwrap()
            )
            .unwrap()
            .value,
        DraftSaveOutcome::Unchanged { .. }
    ));
    service.start_create_group(&session, None).unwrap();
    service.persist_drafts(&session).unwrap();
    assert!(service.drafts(&session).unwrap().value.is_empty());
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    assert!(
        service
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    assert!(service.groups(&session).unwrap().value.is_empty());
    assert!(service.drafts(&session).unwrap().value.is_empty());
}

#[test]
fn snapshots_keep_the_editor_and_retry_only_the_captured_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC database".into(), PASSWORD)
        .unwrap();
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(&session, title("PUBLIC first"))
        .unwrap();
    let first = service.editor_view(&session).unwrap().identity;
    let entry = match &first.target {
        DraftTarget::NewEntry { entry, group: None } => entry.clone(),
        _ => panic!("expected an ungrouped new form"),
    };
    let operation = new_operation_id().unwrap();
    let saved = service
        .save_draft_snapshot(&session, &first.draft, first.revision, &operation)
        .unwrap()
        .value;
    assert!(matches!(saved, DraftSaveOutcome::Saved { .. }));
    assert!(!service.editor_view(&session).unwrap().dirty);
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    service
        .patch_draft(&session, title("PUBLIC second"))
        .unwrap();
    let second = service.editor_view(&session).unwrap().identity;
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        service
            .save_draft_snapshot(&session, &first.draft, first.revision, &operation)
            .unwrap()
            .value,
        saved
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        service.editor_view(&session).unwrap().fields.title,
        "PUBLIC second"
    );
    assert_eq!(
        service
            .save_draft_snapshot(&session, &second.draft, second.revision, &operation)
            .unwrap_err(),
        ServiceError::OperationConflict
    );
    service
        .save_draft_snapshot(
            &session,
            &second.draft,
            second.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let unchanged = service.editor_view(&session).unwrap().identity;
    let result = service
        .save_draft_snapshot(
            &session,
            &unchanged.draft,
            unchanged.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap()
        .value;
    assert!(matches!(result, DraftSaveOutcome::Unchanged { .. }));
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 2);
    let search = service.search_unlocked("PUBLIC second").unwrap();
    assert_eq!(search.len(), 1);
    assert!(search[0].value.group_name.is_none());
    service.lock(&session).unwrap();
    drop(service);
    let mut reopened = DatabaseService::new();
    let session = reopened.open_file(&path, PASSWORD).unwrap();
    assert_eq!(
        reopened.view_entry(&session, &entry).unwrap().value.title,
        "PUBLIC second"
    );
    assert_eq!(reopened.history(&session, &entry).unwrap().value.len(), 2);
    assert_eq!(
        reopened
            .save_draft_snapshot(&session, &first.draft, first.revision, &operation)
            .unwrap()
            .value,
        saved
    );
}

#[test]
fn one_collection_retains_multiple_incomplete_forms_and_masks_resumption() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC database".into(), PASSWORD)
        .unwrap();
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(
            &session,
            EntryPatch {
                password: FieldUpdate::Set("PUBLIC protected draft".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let first = service.editor_view(&session).unwrap().identity;
    let result = service
        .save_draft_snapshot(
            &session,
            &first.draft,
            first.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap()
        .value;
    assert!(matches!(result, DraftSaveOutcome::LocalDraftSaved { .. }));
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(&session, title("PUBLIC date form"))
        .unwrap();
    service
        .set_draft_expiry_input(&session, Some("PUBLIC unfinished date".into()))
        .unwrap();
    let second = service.editor_view(&session).unwrap().identity;
    let group = service
        .start_create_group(&session, None)
        .unwrap()
        .value
        .identity;
    service
        .patch_group_draft(
            &session,
            &group.draft,
            GroupMetadataPatch {
                description: FieldUpdate::Set("PUBLIC unfinished group".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(service.drafts(&session).unwrap().value.len(), 3);
    service.lock(&session).unwrap();
    drop(service);
    let bytes = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            path.is_file().then(|| std::fs::read(path).unwrap())
        })
        .flatten()
        .collect::<Vec<_>>();
    assert!(
        !bytes
            .windows(b"PUBLIC protected draft".len())
            .any(|part| part == b"PUBLIC protected draft")
    );
    let mut reopened = DatabaseService::new();
    let session = reopened.open_file(&path, PASSWORD).unwrap();
    assert_eq!(reopened.drafts(&session).unwrap().value.len(), 3);
    assert!(
        reopened
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    assert!(reopened.groups(&session).unwrap().value.is_empty());
    reopened.resume_draft(&session, &first.draft).unwrap();
    let view = reopened.editor_view(&session).unwrap();
    assert!(view.has_password);
    assert!(view.fields.password.is_none());
    reopened.resume_draft(&session, &second.draft).unwrap();
    assert_eq!(
        reopened
            .editor_view(&session)
            .unwrap()
            .expiry_input
            .as_deref(),
        Some("PUBLIC unfinished date")
    );
    assert_eq!(
        reopened
            .metadata_draft(&session, &group.draft)
            .unwrap()
            .value
            .description
            .as_deref(),
        Some("PUBLIC unfinished group")
    );
    reopened.delete_draft(&session, &first.draft).unwrap();
    reopened.lock(&session).unwrap();
    drop(reopened);
    let mut reopened = DatabaseService::new();
    let session = reopened.open_file(&path, PASSWORD).unwrap();
    assert_eq!(reopened.drafts(&session).unwrap().value.len(), 2);
}

#[test]
fn metadata_forms_save_one_revision_and_preserve_unchanged_concurrent_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC database".into(), PASSWORD)
        .unwrap();
    let form = service.start_create_group(&session, None).unwrap().value;
    service
        .patch_group_draft(
            &session,
            &form.identity.draft,
            GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC group".into()),
                description: FieldUpdate::Set("PUBLIC description".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let group_form = service
        .metadata_draft(&session, &form.identity.draft)
        .unwrap()
        .value;
    service
        .save_draft_snapshot(
            &session,
            &group_form.identity.draft,
            group_form.identity.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let group = service.groups(&session).unwrap().value.remove(0).id;
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        1
    );
    service
        .patch_group_draft(
            &session,
            &form.identity.draft,
            GroupMetadataPatch {
                description: FieldUpdate::Set("PUBLIC local description".into()),
                ..Default::default()
            },
        )
        .unwrap();
    service
        .update_group(
            &session,
            &group,
            "PUBLIC concurrent name".into(),
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let identity = service
        .metadata_draft(&session, &form.identity.draft)
        .unwrap()
        .value
        .identity;
    service
        .save_draft_snapshot(
            &session,
            &identity.draft,
            identity.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let info = service.group_info(&session).unwrap().remove(0);
    assert_eq!(info.group.name, "PUBLIC concurrent name");
    assert_eq!(
        info.description.as_deref(),
        Some("PUBLIC local description")
    );
    let form = service
        .start_edit_database_info(&session)
        .unwrap()
        .value
        .identity;
    let before = service.database_history(&session).unwrap().value.len();
    service
        .patch_database_draft(
            &session,
            &form.draft,
            DatabaseMetadataPatch {
                description: FieldUpdate::Set("PUBLIC local metadata".into()),
                ..Default::default()
            },
        )
        .unwrap();
    service
        .update_database_info(
            &session,
            "PUBLIC concurrent database".into(),
            None,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let identity = service
        .metadata_draft(&session, &form.draft)
        .unwrap()
        .value
        .identity;
    service
        .save_draft_snapshot(
            &session,
            &identity.draft,
            identity.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let info = service.database_info(&session).unwrap();
    assert_eq!(info.name, "PUBLIC concurrent database");
    assert_eq!(info.description.as_deref(), Some("PUBLIC local metadata"));
    assert_eq!(
        service.database_history(&session).unwrap().value.len(),
        before + 2
    );
}

#[test]
fn failed_snapshot_retries_its_old_input_then_preserves_the_newer_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let displaced = directory.path().join("PUBLIC displaced");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC database".into(), PASSWORD)
        .unwrap();
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(&session, title("PUBLIC captured"))
        .unwrap();
    let snapshot = service.editor_view(&session).unwrap().identity;
    let operation = new_operation_id().unwrap();
    std::fs::rename(&path, &displaced).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        service
            .save_draft_snapshot(&session, &snapshot.draft, snapshot.revision, &operation)
            .is_err()
    );
    service
        .patch_draft(&session, title("PUBLIC newer"))
        .unwrap();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&displaced, &path).unwrap();
    service
        .save_draft_snapshot(&session, &snapshot.draft, snapshot.revision, &operation)
        .unwrap();
    let view = service.editor_view(&session).unwrap();
    assert_eq!(view.fields.title, "PUBLIC newer");
    assert!(view.dirty);
    service
        .save_draft_snapshot(
            &session,
            &view.identity.draft,
            view.identity.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let entry = service
        .entries(&session, None, "")
        .unwrap()
        .value
        .remove(0)
        .id;
    service.lock(&session).unwrap();
    drop(service);
    let mut reopened = DatabaseService::new();
    let session = reopened.open_file(&path, PASSWORD).unwrap();
    assert_eq!(
        reopened.view_entry(&session, &entry).unwrap().value.title,
        "PUBLIC newer"
    );
    assert_eq!(reopened.history(&session, &entry).unwrap().value.len(), 2);
}

#[test]
fn local_fallback_reports_its_own_failure_and_deleted_attempts_cannot_resurrect() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC fallback.taypeer");
    let displaced = directory.path().join("PUBLIC displaced database");
    let sidecar = directory.path().join("PUBLIC fallback.taypeer.draft");
    let displaced_sidecar = directory.path().join("PUBLIC displaced forms");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC fallback".into(), PASSWORD)
        .unwrap();
    service.start_create_entry_ungrouped(&session).unwrap();
    service
        .patch_draft(&session, title("PUBLIC unconfirmed fallback"))
        .unwrap();
    let identity = service.editor_view(&session).unwrap().identity;
    let operation = new_operation_id().unwrap();
    std::fs::rename(&path, &displaced).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        service
            .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
            .is_err()
    );
    assert!(
        service
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    service.persist_drafts(&session).unwrap();
    std::fs::rename(&sidecar, &displaced_sidecar).unwrap();
    std::fs::create_dir(&sidecar).unwrap();
    assert!(service.persist_drafts(&session).is_err());
    assert!(service.editor_view(&session).unwrap().dirty);
    std::fs::remove_dir(&sidecar).unwrap();
    std::fs::rename(&displaced_sidecar, &sidecar).unwrap();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&displaced, &path).unwrap();
    service.delete_draft(&session, &identity.draft).unwrap();
    assert!(service.drafts(&session).unwrap().value.is_empty());
    assert_eq!(
        service
            .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
            .unwrap_err(),
        ServiceError::OperationConflict
    );
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    assert!(
        service
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    assert!(service.drafts(&session).unwrap().value.is_empty());
    assert_eq!(
        service
            .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
            .unwrap_err(),
        ServiceError::OperationConflict
    );
}
