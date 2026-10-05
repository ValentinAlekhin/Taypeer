//! Public synthetic file scenarios exercising the shared application API.
use std::fs;
use taypeer_core::{Color, EntryId, GroupId, OperationId};
use taypeer_services::*;

const PASSWORD: &[u8] = b"PUBLIC binary master";

fn setup(
    service: &mut DatabaseService,
    directory: &std::path::Path,
) -> (SessionToken, GroupId, EntryId) {
    let session = service
        .create_file(
            &directory.join("PUBLIC.taypeer"),
            "PUBLIC binary database".into(),
            PASSWORD,
        )
        .unwrap();
    let group = service
        .create_group(
            &session,
            "PUBLIC group".into(),
            None,
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group.clone()).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC binary entry".into(),
                password: Some("PUBLIC hidden value".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = service
        .save_draft(&session, &taypeer_services::new_operation_id().unwrap())
        .unwrap()
        .value;
    (session, group, entry)
}
fn edit(
    service: &mut DatabaseService,
    session: &SessionToken,
    target: BinaryTarget,
    edit: BinaryEdit,
    operation: &str,
) {
    service
        .edit_binary(
            session,
            &BinaryRequest {
                target,
                edit,
                review: None,
            },
            &OperationId::new(operation),
        )
        .unwrap();
}

#[test]
fn attachment_replacement_history_clone_and_legacy_editor_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("PUBLIC input.txt");
    fs::write(&input, b"PUBLIC original contents").unwrap();
    let mut service = DatabaseService::new();
    let (session, group, entry) = setup(&mut service, directory.path());
    let target = BinaryTarget::Entry(entry.clone());
    for operation in ["PUBLIC add one", "PUBLIC add two"] {
        edit(
            &mut service,
            &session,
            target.clone(),
            BinaryEdit::Attachment(AttachmentEdit::Add {
                path: input.clone(),
                name: Some("PUBLIC same name".into()),
            }),
            operation,
        );
    }
    let view = service.binary_view(&session, &target).unwrap().value;
    assert_eq!(view.attachments.len(), 2);
    assert_ne!(view.attachments[0].id, view.attachments[1].id);
    assert_eq!(
        service
            .storage_usage(&session)
            .unwrap()
            .value
            .attachment_bytes,
        24
    );
    let original_revision = service
        .history(&session, &entry)
        .unwrap()
        .value
        .last()
        .unwrap()
        .id
        .clone();
    let first = view.attachments[0].id.clone();
    fs::write(&input, b"PUBLIC replacement").unwrap();
    edit(
        &mut service,
        &session,
        target.clone(),
        BinaryEdit::Attachment(AttachmentEdit::Replace {
            attachment: first.clone(),
            path: input,
        }),
        "PUBLIC replace",
    );
    edit(
        &mut service,
        &session,
        target.clone(),
        BinaryEdit::Appearance {
            foreground: FieldUpdate::Set(Color([10, 20, 30, 255])),
            background: FieldUpdate::Keep,
        },
        "PUBLIC color",
    );
    let mut form = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .fields;
    form.title = "PUBLIC renamed by old frontend".into();
    service.update_draft(&session, form).unwrap();
    service
        .save_draft(&session, &taypeer_services::new_operation_id().unwrap())
        .unwrap();
    assert_eq!(
        service
            .view_entry(&session, &entry)
            .unwrap()
            .value
            .attachments
            .len(),
        2
    );
    assert_eq!(
        service
            .view_entry(&session, &entry)
            .unwrap()
            .value
            .appearance
            .foreground,
        Some(Color([10, 20, 30, 255]))
    );
    let clone = service
        .clone_entry(
            &session,
            &entry,
            group,
            None,
            &OperationId::new("PUBLIC clone"),
        )
        .unwrap()
        .value;
    let cloned = service
        .binary_view(&session, &BinaryTarget::Entry(clone))
        .unwrap()
        .value;
    assert!(
        cloned
            .attachments
            .iter()
            .all(|a| !view.attachments.iter().any(|old| a.id == old.id))
    );
    drop(service);
    let mut service = DatabaseService::new();
    let session = service
        .open_file(&directory.path().join("PUBLIC.taypeer"), PASSWORD)
        .unwrap();
    let old_target = BinaryTarget::Revision {
        entry: entry.clone(),
        revision: original_revision,
    };
    let old = service.binary_view(&session, &old_target).unwrap().value;
    let path = directory.path().join("PUBLIC exported.txt");
    service
        .export_binary(
            &session,
            &old_target,
            &old.attachments[0].contents[0].id,
            &path,
            false,
        )
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"PUBLIC original contents");
    assert_eq!(
        service
            .export_binary(
                &session,
                &old_target,
                &old.attachments[0].contents[0].id,
                &path,
                false
            )
            .unwrap_err(),
        ServiceError::Storage(StorageError::AlreadyExists)
    );
    let before = fs::read(directory.path().join("PUBLIC.taypeer")).unwrap();
    assert!(!before.windows(6).any(|bytes| bytes == b"PUBLIC"));
}

#[test]
fn binary_draft_is_independent_and_cleanup_releases_only_unreachable_working_contents() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("PUBLIC input.bin");
    let content = vec![42_u8; 128 * 1024];
    fs::write(&input, &content).unwrap();
    let mut service = DatabaseService::new();
    let (session, _, entry) = setup(&mut service, directory.path());
    service.start_edit_entry(&session, &entry).unwrap();
    edit(
        &mut service,
        &session,
        BinaryTarget::Draft,
        BinaryEdit::Attachment(AttachmentEdit::Add {
            path: input,
            name: None,
        }),
        "PUBLIC draft add",
    );
    assert_eq!(
        service
            .storage_usage(&session)
            .unwrap()
            .value
            .attachment_bytes,
        0
    );
    assert_eq!(
        service.storage_usage(&session).unwrap().value.draft_bytes,
        content.len() as u64
    );
    service
        .collect_blobs(&session, &OperationId::new("PUBLIC gc active draft"))
        .unwrap();
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service
        .open_file(&directory.path().join("PUBLIC.taypeer"), PASSWORD)
        .unwrap();
    assert!(service.binary_view(&session, &BinaryTarget::Draft).is_err());
    service.restore_draft(&session).unwrap();
    let draft = service
        .binary_view(&session, &BinaryTarget::Draft)
        .unwrap()
        .value;
    let attachment = draft.attachments[0].id.clone();
    let blob = draft.attachments[0].contents[0].id.clone();
    let output = directory.path().join("PUBLIC draft export.bin");
    service
        .export_binary(&session, &BinaryTarget::Draft, &blob, &output, false)
        .unwrap();
    assert_eq!(fs::read(output).unwrap(), content);
    service
        .save_draft(&session, &taypeer_services::new_operation_id().unwrap())
        .unwrap();
    let target = BinaryTarget::Entry(entry.clone());
    edit(
        &mut service,
        &session,
        target.clone(),
        BinaryEdit::Attachment(AttachmentEdit::Remove { attachment }),
        "PUBLIC remove",
    );
    assert_eq!(
        service
            .storage_usage(&session)
            .unwrap()
            .value
            .attachment_bytes,
        content.len() as u64
    );
    let before = service.storage_usage(&session).unwrap().value.file_bytes;
    let revisions = service
        .history(&session, &entry)
        .unwrap()
        .value
        .into_iter()
        .map(|r| r.id)
        .collect();
    service
        .purge_history(
            &session,
            &entry,
            revisions,
            &OperationId::new("PUBLIC history cleanup"),
        )
        .unwrap();
    let usage = service
        .collect_blobs(&session, &OperationId::new("PUBLIC final gc"))
        .unwrap()
        .value;
    assert_eq!(usage.attachment_bytes, 0);
    assert_eq!(usage.retained_bytes, 0);
    assert!(usage.file_bytes + content.len() as u64 / 2 < before);
    assert!(
        serde_json::to_value(&usage)
            .unwrap()
            .get("backup_bytes")
            .is_none()
    );
    assert!(
        service
            .export_binary(
                &session,
                &target,
                &blob,
                &directory.path().join("PUBLIC forbidden.bin"),
                false
            )
            .is_err()
    );
}

#[test]
fn failed_binary_save_preserves_the_document_and_operation_can_be_retried() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("PUBLIC input.bin");
    fs::write(&input, b"PUBLIC retained bytes").unwrap();
    let mut service = DatabaseService::new();
    let (session, _, entry) = setup(&mut service, directory.path());
    let path = directory.path().join("PUBLIC.taypeer");
    let original = fs::read(&path).unwrap();
    fs::write(&path, b"PUBLIC external change").unwrap();
    let request = BinaryRequest {
        target: BinaryTarget::Entry(entry.clone()),
        edit: BinaryEdit::Attachment(AttachmentEdit::Add {
            path: input,
            name: None,
        }),
        review: None,
    };
    let operation = OperationId::new("PUBLIC retry");
    assert_eq!(
        service
            .edit_binary(&session, &request, &operation)
            .unwrap_err(),
        ServiceError::Storage(StorageError::Changed)
    );
    assert!(
        service
            .binary_view(&session, &request.target)
            .unwrap()
            .value
            .attachments
            .is_empty()
    );
    fs::write(&path, original).unwrap();
    service.edit_binary(&session, &request, &operation).unwrap();
    let committed = fs::read(&path).unwrap();
    service.edit_binary(&session, &request, &operation).unwrap();
    assert_eq!(fs::read(&path).unwrap(), committed);
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 2);
}

#[test]
fn selected_attachment_retry_survives_autosave_newer_input_removal_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = DatabaseService::new();
    let (session, _, entry) = setup(&mut service, directory.path());
    let draft = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .identity
        .draft;
    let bytes = b"PUBLIC selected attachment contents";
    let operation = OperationId::new("PUBLIC selected attachment operation");
    let import = |service: &mut DatabaseService, session: &SessionToken, bytes: &[u8]| {
        service.import_attachment_stream(
            session,
            &draft,
            bytes,
            AttachmentImport {
                length: bytes.len() as u64,
                name: "PUBLIC selected.txt".into(),
                replacement: None,
            },
            &operation,
        )
    };
    import(&mut service, &session, bytes).unwrap();
    let before = service.editor_view(&session).unwrap();
    let binary = service
        .binary_view(&session, &BinaryTarget::Draft)
        .unwrap()
        .value;
    assert_eq!(binary.attachments.len(), 1);
    let attachment = binary.attachments[0].id.clone();
    let blob = binary.attachments[0].contents[0].id.clone();
    import(&mut service, &session, bytes).unwrap();
    assert_eq!(
        service.editor_view(&session).unwrap().identity,
        before.identity
    );
    assert_eq!(
        import(&mut service, &session, &vec![b'x'; bytes.len()]).unwrap_err(),
        ServiceError::InvalidInput,
    );
    service
        .save_draft_snapshot(
            &session,
            &draft,
            before.identity.revision,
            &OperationId::new("PUBLIC selected first snapshot"),
        )
        .unwrap();
    service
        .patch_draft(
            &session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC newer title".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let newer = service.editor_view(&session).unwrap();
    import(&mut service, &session, bytes).unwrap();
    let unchanged = service.editor_view(&session).unwrap();
    assert_eq!(unchanged.identity, newer.identity);
    assert_eq!(unchanged.fields.title, "PUBLIC newer title");
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 2);
    edit(
        &mut service,
        &session,
        BinaryTarget::Draft,
        BinaryEdit::Attachment(AttachmentEdit::Remove { attachment }),
        "PUBLIC selected remove",
    );
    let removed = service.editor_view(&session).unwrap();
    import(&mut service, &session, bytes).unwrap();
    assert_eq!(
        service.editor_view(&session).unwrap().identity,
        removed.identity
    );
    assert!(
        service
            .binary_view(&session, &BinaryTarget::Draft)
            .unwrap()
            .value
            .attachments
            .is_empty()
    );
    service
        .save_draft_snapshot(
            &session,
            &draft,
            removed.identity.revision,
            &OperationId::new("PUBLIC selected removal snapshot"),
        )
        .unwrap();
    // Retain newer unfinished input so this same editor is recoverable after restart.
    service
        .patch_draft(
            &session,
            EntryPatch {
                notes: FieldUpdate::Set("PUBLIC later unsaved notes".into()),
                ..Default::default()
            },
        )
        .unwrap();
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service
        .open_file(&directory.path().join("PUBLIC.taypeer"), PASSWORD)
        .unwrap();
    service.resume_draft(&session, &draft).unwrap();
    let resumed = service.editor_view(&session).unwrap();
    import(&mut service, &session, bytes).unwrap();
    assert_eq!(
        service.editor_view(&session).unwrap().identity,
        resumed.identity
    );
    assert!(
        service
            .binary_view(&session, &BinaryTarget::Draft)
            .unwrap()
            .value
            .attachments
            .is_empty()
    );
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 3);
    let mut hidden = Vec::new();
    assert_eq!(
        service
            .export_binary_stream(
                &session,
                &BinaryTarget::Entry(entry.clone()),
                &blob,
                &mut hidden
            )
            .unwrap_err(),
        ServiceError::NotFound
    );
    assert!(hidden.is_empty());
    let revision = service.history(&session, &entry).unwrap().value[1]
        .id
        .clone();
    let mut output = Vec::new();
    service
        .export_binary_stream(
            &session,
            &BinaryTarget::Revision { entry, revision },
            &blob,
            &mut output,
        )
        .unwrap();
    assert_eq!(output, bytes);
}

struct FailingSelectedInput;
impl std::io::Read for FailingSelectedInput {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn selected_input_failures_and_wrong_editor_preserve_prior_form_and_blobs() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = DatabaseService::new();
    let (session, _, entry) = setup(&mut service, directory.path());
    let draft = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .identity
        .draft;
    service
        .patch_draft(
            &session,
            EntryPatch {
                notes: FieldUpdate::Set("PUBLIC prior unsaved form".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let before = service.editor_view(&session).unwrap().identity;
    let size = fs::metadata(directory.path().join("PUBLIC.taypeer"))
        .unwrap()
        .len();
    let failed = |service: &mut DatabaseService, input: Box<dyn std::io::Read>, length| {
        service.import_attachment_stream(
            &session,
            &draft,
            input,
            AttachmentImport {
                length,
                name: "PUBLIC failing source".into(),
                replacement: None,
            },
            &OperationId::new("PUBLIC failed stream"),
        )
    };
    assert_eq!(
        failed(&mut service, Box::new(FailingSelectedInput), 1024).unwrap_err(),
        ServiceError::Storage(StorageError::Io)
    );
    assert_eq!(
        failed(&mut service, Box::new(b"short".as_slice()), 12).unwrap_err(),
        ServiceError::Storage(StorageError::Io)
    );
    assert_eq!(
        failed(&mut service, Box::new(b"too long".as_slice()), 2).unwrap_err(),
        ServiceError::Storage(StorageError::InvalidFile)
    );
    // This reader would fail if invoked; a per-file limit must be checked first.
    assert_eq!(
        failed(
            &mut service,
            Box::new(FailingSelectedInput),
            11 * 1024 * 1024
        )
        .unwrap_err(),
        ServiceError::AttachmentLimit
    );
    assert_eq!(service.editor_view(&session).unwrap().identity, before);
    assert!(
        service
            .binary_view(&session, &BinaryTarget::Draft)
            .unwrap()
            .value
            .attachments
            .is_empty()
    );
    assert_eq!(
        service.storage_usage(&session).unwrap().value.draft_bytes,
        0
    );
    assert_eq!(
        fs::metadata(directory.path().join("PUBLIC.taypeer"))
            .unwrap()
            .len(),
        size
    );
    let second = service
        .start_create_entry_in(&session, None)
        .unwrap()
        .value
        .identity
        .draft;
    assert_ne!(draft, second);
    assert_eq!(
        failed(&mut service, Box::new(FailingSelectedInput), 1024).unwrap_err(),
        ServiceError::InvalidContext
    );
    service.resume_draft(&session, &draft).unwrap();
    failed(&mut service, Box::new(b"PUBLIC retry".as_slice()), 12).unwrap();
    assert_eq!(
        service
            .binary_view(&session, &BinaryTarget::Draft)
            .unwrap()
            .value
            .attachments
            .len(),
        1
    );
}

#[test]
fn unfinished_new_form_retains_streamed_contents_without_a_document_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let (session, _, _) = setup(&mut service, directory.path());
    let draft = service
        .start_create_entry_ungrouped(&session)
        .unwrap()
        .value
        .identity
        .draft;
    let bytes = b"PUBLIC unfinished selected contents";
    let operation = OperationId::new("PUBLIC unfinished attachment import");
    service
        .import_attachment_stream(
            &session,
            &draft,
            bytes.as_slice(),
            AttachmentImport {
                length: bytes.len() as u64,
                name: "PUBLIC unfinished attachment.txt".into(),
                replacement: None,
            },
            &operation,
        )
        .unwrap();
    let editor = service.editor_view(&session).unwrap();
    assert!(editor.fields.title.is_empty());
    let outcome = service
        .save_draft_snapshot(
            &session,
            &draft,
            editor.identity.revision,
            &OperationId::new("PUBLIC unfinished local snapshot"),
        )
        .unwrap()
        .value;
    assert!(matches!(outcome, DraftSaveOutcome::LocalDraftSaved { .. }));
    let blob = service
        .binary_view(&session, &BinaryTarget::Draft)
        .unwrap()
        .value
        .attachments[0]
        .contents[0]
        .id
        .clone();
    assert_eq!(
        service
            .entries(&session, None, "PUBLIC")
            .unwrap()
            .value
            .len(),
        1
    );
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    service.resume_draft(&session, &draft).unwrap();
    let resumed = service.editor_view(&session).unwrap();
    assert_eq!(resumed.identity, editor.identity);
    assert!(resumed.fields.title.is_empty());
    let mut exported = Vec::new();
    service
        .export_binary_stream(&session, &BinaryTarget::Draft, &blob, &mut exported)
        .unwrap();
    assert_eq!(exported, bytes);
}
