//! PUBLIC images exercise causal form staging; no network request or real document is used.
use std::{
    fs,
    io::{self, Read},
};
use taypeer_core::{
    BlobId, DraftId, GroupMetadataPatch, ICON_LIMIT, IconRef, IconSource, OperationId,
};
use taypeer_services::{
    BinaryEdit, BinaryRequest, BinaryTarget, DatabaseService, DraftIdentity, DraftTarget,
    EntryPatch, FieldUpdate, IconImport, ServiceError, SessionToken, new_operation_id,
};

const PASSWORD: &[u8] = b"PUBLIC icon form master";
const FIRST: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#224466"/></svg>"##;
const SECOND: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><circle cx="10" cy="10" r="8" fill="#6688aa"/></svg>"##;

#[test]
fn stored_favicon_rejects_a_builtin_operation_and_exact_replay_preserves_later_input() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sidecar = directory.path().join("PUBLIC.taypeer.draft");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC icon action replay".into(), PASSWORD)
        .unwrap();
    let form = entry_form(&mut service, &session);
    service
        .import_icon_stream(
            &session,
            &form.draft,
            FIRST,
            IconImport {
                length: FIRST.len() as u64,
                source: IconSource::Favicon("https://public.invalid/original".into()),
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let original = service
        .stored_favicon(&session, &form.draft)
        .unwrap()
        .value
        .unwrap();
    // Use the active-entry target used by Native set_icon, rather than an import receipt.
    let builtin = BinaryRequest {
        target: BinaryTarget::Draft,
        edit: BinaryEdit::Icon(taypeer_services::IconInput::Lucide(
            "key-round".to_owned().try_into().unwrap(),
        )),
        review: None,
    };
    let builtin_operation = new_operation_id().unwrap();
    service
        .edit_binary(&session, &builtin, &builtin_operation)
        .unwrap();
    let current = service.active_draft(&session).unwrap().value.unwrap();
    let encrypted = fs::read(&sidecar).unwrap();
    let working = fs::read(&path).unwrap();
    assert_eq!(
        service
            .select_stored_favicon(
                &session,
                &form.draft,
                &original.identity,
                &builtin_operation
            )
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(
        service.active_draft(&session).unwrap().value.unwrap(),
        current
    );
    assert_eq!(fs::read(&sidecar).unwrap(), encrypted);
    assert_eq!(fs::read(&path).unwrap(), working);
    assert!(matches!(
        image(&service, &session, BinaryTarget::Form(form.draft.clone())),
        IconRef::Lucide(_)
    ));
    // The rejected foreign intent must leave the original builtin receipt intact.
    service
        .edit_binary(&session, &builtin, &builtin_operation)
        .unwrap();
    assert_eq!(
        service.active_draft(&session).unwrap().value.unwrap(),
        current
    );
    assert_eq!(fs::read(&sidecar).unwrap(), encrypted);

    let selection = new_operation_id().unwrap();
    service
        .select_stored_favicon(&session, &form.draft, &original.identity, &selection)
        .unwrap();
    service
        .edit_binary(&session, &builtin, &new_operation_id().unwrap())
        .unwrap();
    let later = service.active_draft(&session).unwrap().value.unwrap();
    let encrypted = fs::read(&sidecar).unwrap();
    assert_eq!(
        service
            .select_stored_favicon(&session, &form.draft, &original.identity, &selection)
            .unwrap()
            .value,
        later
    );
    assert_eq!(
        service.active_draft(&session).unwrap().value.unwrap(),
        later
    );
    assert_eq!(fs::read(&sidecar).unwrap(), encrypted);
    assert_eq!(fs::read(&path).unwrap(), working);
    assert!(matches!(
        image(&service, &session, BinaryTarget::Form(form.draft)),
        IconRef::Lucide(_)
    ));
}

#[test]
fn new_form_reselects_the_exact_local_favicon_original_without_reimport_and_delete_is_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC stored originals".into(), PASSWORD)
        .unwrap();
    let form = entry_form(&mut service, &session);
    let before = fs::read(&path).unwrap();
    let source = IconSource::Favicon("https://public.invalid/original".into());
    let imported = new_operation_id().unwrap();
    service
        .import_icon_stream(
            &session,
            &form.draft,
            FIRST,
            IconImport {
                length: FIRST.len() as u64,
                source: source.clone(),
            },
            &imported,
        )
        .unwrap();
    let original = service
        .stored_favicon(&session, &form.draft)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(original.bytes, Some(FIRST.len() as u64));
    let lucide = |draft: &DraftId| BinaryRequest {
        target: BinaryTarget::Form(draft.clone()),
        edit: BinaryEdit::Icon(taypeer_services::IconInput::Lucide(
            "key-round".to_owned().try_into().unwrap(),
        )),
        review: None,
    };
    service
        .edit_binary(&session, &lucide(&form.draft), &new_operation_id().unwrap())
        .unwrap();
    assert_eq!(
        service.storage_usage(&session).unwrap().value.draft_bytes,
        FIRST.len() as u64
    );
    assert_eq!(
        service
            .stored_favicon(&session, &form.draft)
            .unwrap()
            .value
            .unwrap()
            .identity,
        original.identity
    );
    let scope = BinaryTarget::FormFavicon {
        draft: form.draft.clone(),
        identity: original.identity.clone(),
    };
    let mut preview = Vec::new();
    service
        .export_binary_stream(&session, &scope, &original.identity, &mut preview)
        .unwrap();
    assert_eq!(preview, FIRST);
    let selection = new_operation_id().unwrap();
    service
        .select_stored_favicon(&session, &form.draft, &original.identity, &selection)
        .unwrap();
    assert!(matches!(
        image(&service, &session, BinaryTarget::Form(form.draft.clone())),
        IconRef::Image {
            source: IconSource::Favicon(_),
            ..
        }
    ));
    service
        .edit_binary(&session, &lucide(&form.draft), &new_operation_id().unwrap())
        .unwrap();
    let current = service.active_draft(&session).unwrap().value.unwrap();
    assert_eq!(
        service
            .select_stored_favicon(&session, &form.draft, &original.identity, &selection)
            .unwrap()
            .value,
        current
    );
    assert!(matches!(
        image(&service, &session, BinaryTarget::Form(form.draft.clone())),
        IconRef::Lucide(_)
    ));
    assert_eq!(
        service
            .select_stored_favicon(
                &session,
                &form.draft,
                &BlobId::new("PUBLIC wrong original"),
                &new_operation_id().unwrap()
            )
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(
        service
            .select_stored_favicon(&session, &form.draft, &original.identity, &imported)
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(
        service
            .icon_import_receipt(&session, &form.draft, &source, &selection)
            .err(),
        Some(ServiceError::InvalidInput)
    );
    service
        .import_icon_stream(
            &session,
            &form.draft,
            SECOND,
            IconImport {
                length: SECOND.len() as u64,
                source: source.clone(),
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let changed = service
        .stored_favicon(&session, &form.draft)
        .unwrap()
        .value
        .unwrap();
    assert_ne!(changed.identity, original.identity);
    let after_change = service.active_draft(&session).unwrap().value.unwrap();
    assert_eq!(
        service
            .select_stored_favicon(
                &session,
                &form.draft,
                &original.identity,
                &new_operation_id().unwrap()
            )
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(
        service.active_draft(&session).unwrap().value.unwrap(),
        after_change
    );
    assert_eq!(
        service.binary_view(&session, &scope).err(),
        Some(ServiceError::NotFound)
    );
    service
        .edit_binary(&session, &lucide(&form.draft), &new_operation_id().unwrap())
        .unwrap();
    assert_eq!(
        service
            .stored_favicon(&session, &form.draft)
            .unwrap()
            .value
            .unwrap()
            .identity,
        changed.identity
    );
    assert_eq!(fs::read(&path).unwrap(), before);

    let replacement = service
        .start_create_group(&session, None)
        .unwrap()
        .value
        .identity;
    let replacement_operation = new_operation_id().unwrap();
    import(
        &mut service,
        &session,
        &replacement.draft,
        SECOND,
        &replacement_operation,
    );
    assert_eq!(
        service
            .select_stored_favicon(&session, &replacement.draft, &original.identity, &selection)
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(
        service
            .select_stored_favicon(
                &session,
                &replacement.draft,
                &original.identity,
                &new_operation_id().unwrap()
            )
            .err(),
        Some(ServiceError::InvalidInput)
    );
    service.delete_draft(&session, &form.draft).unwrap();
    assert_eq!(
        service
            .icon_import_receipt(&session, &form.draft, &source, &imported)
            .err(),
        Some(ServiceError::NoDraft)
    );
    assert!(
        service
            .icon_import_receipt(
                &session,
                &replacement.draft,
                &IconSource::File,
                &replacement_operation
            )
            .unwrap()
            .value
            .is_some()
    );
    assert_eq!(
        service.storage_usage(&session).unwrap().value.draft_bytes,
        SECOND.len() as u64
    );
    service
        .collect_blobs(&session, &new_operation_id().unwrap())
        .unwrap();
    assert_eq!(
        service
            .icon_preview(&session, &BinaryTarget::Form(replacement.draft.clone()))
            .unwrap()
            .unwrap()
            .bytes
            .as_slice(),
        SECOND
    );
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    assert_eq!(
        service
            .icon_import_receipt(&session, &form.draft, &source, &imported)
            .err(),
        Some(ServiceError::NoDraft)
    );
    assert!(
        service
            .icon_import_receipt(
                &session,
                &replacement.draft,
                &IconSource::File,
                &replacement_operation
            )
            .unwrap()
            .value
            .is_some()
    );
}

fn entry_form(service: &mut DatabaseService, session: &SessionToken) -> DraftIdentity {
    let group = service
        .create_group(
            session,
            "PUBLIC icons".into(),
            None,
            &new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    service.start_create_entry(session, group).unwrap();
    service.active_draft(session).unwrap().value.unwrap()
}
fn import(
    service: &mut DatabaseService,
    session: &SessionToken,
    draft: &DraftId,
    bytes: &[u8],
    operation: &OperationId,
) -> DraftIdentity {
    service
        .import_icon_stream(
            session,
            draft,
            bytes,
            IconImport {
                length: bytes.len() as u64,
                source: IconSource::File,
            },
            operation,
        )
        .unwrap()
        .value
}
fn image(service: &DatabaseService, session: &SessionToken, target: BinaryTarget) -> IconRef {
    service
        .binary_view(session, &target)
        .unwrap()
        .value
        .icons
        .into_iter()
        .next()
        .unwrap()
}
struct Unreadable;
impl Read for Unreadable {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("an acknowledged PUBLIC icon must not read its replacement provider")
    }
}

struct ThrowingProvider;
impl Read for ThrowingProvider {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
}

#[test]
fn invalid_selected_streams_preserve_exact_existing_encrypted_draft_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC immutable prior draft".into(), PASSWORD)
        .unwrap();
    let initial = entry_form(&mut service, &session);
    let form = import(
        &mut service,
        &session,
        &initial.draft,
        FIRST,
        &new_operation_id().unwrap(),
    );
    let prior = fs::read(directory.path().join("PUBLIC.taypeer.draft")).unwrap();
    let working = fs::read(&path).unwrap();
    let original = image(&service, &session, BinaryTarget::Form(form.draft.clone()));
    let mut growing = FIRST.to_vec();
    growing.push(b'P');
    let invalid = b"PUBLIC invalid icon";
    let cases: Vec<(Box<dyn Read>, u64, ServiceError)> = vec![
        (
            Box::new(io::Cursor::new(FIRST[..FIRST.len() - 1].to_vec())),
            FIRST.len() as u64,
            ServiceError::InvalidInput,
        ),
        (
            Box::new(io::Cursor::new(growing)),
            FIRST.len() as u64,
            ServiceError::InvalidInput,
        ),
        (
            Box::new(io::Cursor::new(invalid.to_vec())),
            invalid.len() as u64,
            ServiceError::Icon(taypeer_services::icons::IconError::InvalidImage),
        ),
        (
            Box::new(ThrowingProvider),
            FIRST.len() as u64,
            ServiceError::Icon(taypeer_services::icons::IconError::File),
        ),
    ];
    let operation = new_operation_id().unwrap();
    for (input, length, expected) in cases {
        assert_eq!(
            service
                .import_icon_stream(
                    &session,
                    &form.draft,
                    input,
                    IconImport {
                        length,
                        source: IconSource::File
                    },
                    &operation
                )
                .err(),
            Some(expected)
        );
        assert_eq!(service.active_draft(&session).unwrap().value.unwrap(), form);
        assert!(image(&service, &session, BinaryTarget::Form(form.draft.clone())) == original);
        assert_eq!(
            fs::read(directory.path().join("PUBLIC.taypeer.draft")).unwrap(),
            prior
        );
        assert_eq!(fs::read(&path).unwrap(), working);
        assert!(
            service
                .icon_import_receipt(&session, &form.draft, &IconSource::File, &operation)
                .unwrap()
                .value
                .is_none()
        );
    }
    import(&mut service, &session, &form.draft, SECOND, &operation);
}

#[test]
fn original_icon_limit_is_exact_and_overlimit_refuses_before_reading() {
    let mut service = DatabaseService::new();
    let database = service.create_database("PUBLIC exact image limit").unwrap();
    let session = service
        .unlock(&database, taypeer_services::DEMO_PASSWORD)
        .unwrap();
    let form = service
        .start_create_group(&session, None)
        .unwrap()
        .value
        .identity;
    let mut bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#224466"/><!--"##.to_vec();
    let end = b"--></svg>";
    bytes.resize(ICON_LIMIT as usize - end.len(), b'P');
    bytes.extend_from_slice(end);
    assert_eq!(bytes.len() as u64, ICON_LIMIT);
    import(
        &mut service,
        &session,
        &form.draft,
        &bytes,
        &new_operation_id().unwrap(),
    );
    let before = service.active_draft(&session).unwrap().value.unwrap();
    assert_eq!(
        service
            .import_icon_stream(
                &session,
                &form.draft,
                Unreadable,
                IconImport {
                    length: ICON_LIMIT + 1,
                    source: IconSource::File
                },
                &new_operation_id().unwrap()
            )
            .err(),
        Some(ServiceError::Icon(
            taypeer_services::icons::IconError::TooLarge
        ))
    );
    assert_eq!(
        service.active_draft(&session).unwrap().value.unwrap(),
        before
    );
    assert_eq!(
        service.storage_usage(&session).unwrap().value.draft_bytes,
        ICON_LIMIT
    );
}

#[test]
fn unfinished_entry_icon_and_text_share_one_saved_revision_and_old_retry_preserves_later_input() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC icons".into(), PASSWORD)
        .unwrap();
    let form = entry_form(&mut service, &session);
    let before = fs::read(&path).unwrap();
    let operation = new_operation_id().unwrap();
    let imported = import(&mut service, &session, &form.draft, FIRST, &operation);
    assert_eq!(imported.revision.0, form.revision.0 + 1);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        service
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    let preview = service
        .icon_preview(&session, &BinaryTarget::Form(form.draft.clone()))
        .unwrap()
        .unwrap();
    assert_eq!(preview.bytes.as_slice(), FIRST);
    service
        .patch_draft(
            &session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC staged icon entry".into()),
                notes: FieldUpdate::Set("PUBLIC same snapshot".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let save = service.active_draft(&session).unwrap().value.unwrap();
    service
        .save_draft_snapshot(
            &session,
            &save.draft,
            save.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let DraftTarget::NewEntry { entry, .. } = form.target else {
        panic!("PUBLIC fixture must be a new entry")
    };
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    let saved = service.view_entry(&session, &entry).unwrap().value;
    assert_eq!(saved.notes.as_deref(), Some("PUBLIC same snapshot"));
    assert!(matches!(
        saved.appearance.icon,
        IconRef::Image {
            source: IconSource::File,
            ..
        }
    ));

    import(
        &mut service,
        &session,
        &form.draft,
        SECOND,
        &new_operation_id().unwrap(),
    );
    service
        .patch_draft(
            &session,
            EntryPatch {
                notes: FieldUpdate::Set("PUBLIC newer unsaved text".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let newer = service.active_draft(&session).unwrap().value.unwrap();
    let newer_icon = image(&service, &session, BinaryTarget::Form(form.draft.clone()));
    let before_retry = fs::read(&path).unwrap();
    let receipt = service
        .import_icon_stream(
            &session,
            &form.draft,
            Unreadable,
            IconImport {
                length: u64::MAX,
                source: IconSource::File,
            },
            &operation,
        )
        .unwrap()
        .value;
    assert_eq!(receipt, newer);
    assert!(image(&service, &session, BinaryTarget::Form(form.draft.clone())) == newer_icon);
    assert_eq!(
        service
            .editor_view(&session)
            .unwrap()
            .fields
            .notes
            .as_deref(),
        Some("PUBLIC newer unsaved text")
    );
    assert_eq!(fs::read(&path).unwrap(), before_retry);
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    assert_eq!(
        service
            .icon_import_receipt(
                &session,
                &form.draft,
                &IconSource::Url("https://public.invalid/icon.svg".into()),
                &operation
            )
            .err(),
        Some(ServiceError::InvalidInput)
    );
}

#[test]
fn unfinished_group_icon_is_locally_durable_and_uses_the_same_metadata_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC group image".into(), PASSWORD)
        .unwrap();
    let form = service
        .start_create_group(&session, None)
        .unwrap()
        .value
        .identity;
    let before = fs::read(&path).unwrap();
    let operation = new_operation_id().unwrap();
    let staged = import(&mut service, &session, &form.draft, FIRST, &operation);
    assert_eq!(staged.revision.0, 1);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(service.groups(&session).unwrap().value.is_empty());
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    service.resume_draft(&session, &form.draft).unwrap();
    assert_eq!(
        service
            .icon_preview(&session, &BinaryTarget::Form(form.draft.clone()))
            .unwrap()
            .unwrap()
            .bytes
            .as_slice(),
        FIRST
    );
    service
        .patch_group_draft(
            &session,
            &form.draft,
            GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC combined group".into()),
                description: FieldUpdate::Set("PUBLIC combined description".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let saved = service.active_draft(&session).unwrap().value.unwrap();
    service
        .save_draft_snapshot(
            &session,
            &saved.draft,
            saved.revision,
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let group = service.groups(&session).unwrap().value.remove(0);
    assert_eq!(group.name, "PUBLIC combined group");
    assert!(matches!(group.icon, IconRef::Image { .. }));
    assert_eq!(
        service
            .group_history(&session, &group.id)
            .unwrap()
            .value
            .len(),
        1
    );
    assert_eq!(
        service
            .metadata_draft(&session, &form.draft)
            .unwrap()
            .value
            .description
            .as_deref(),
        Some("PUBLIC combined description")
    );
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target: BinaryTarget::Form(form.draft.clone()),
                edit: BinaryEdit::Icon(taypeer_services::IconInput::Lucide(
                    "key-round".to_owned().try_into().unwrap(),
                )),
                review: None,
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let current = service.active_draft(&session).unwrap().value.unwrap();
    assert_eq!(
        service
            .import_icon_stream(
                &session,
                &form.draft,
                Unreadable,
                IconImport {
                    length: 0,
                    source: IconSource::File
                },
                &operation
            )
            .unwrap()
            .value,
        current
    );
    assert!(matches!(
        image(&service, &session, BinaryTarget::Form(form.draft)),
        IconRef::Lucide(_)
    ));
    assert_eq!(
        service
            .group_history(&session, &group.id)
            .unwrap()
            .value
            .len(),
        1
    );
}

#[test]
fn rejected_images_and_failed_staging_preserve_form_blob_and_operation_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC image failures".into(), PASSWORD)
        .unwrap();
    let form = entry_form(&mut service, &session);
    let before = fs::read(&path).unwrap();
    let operation = new_operation_id().unwrap();
    let before_usage = service.storage_usage(&session).unwrap().value.draft_bytes;
    for (bytes, length, expected) in [
        (FIRST, FIRST.len() as u64 + 1, ServiceError::InvalidInput),
        (FIRST, FIRST.len() as u64 - 1, ServiceError::InvalidInput),
        (
            b"PUBLIC invalid raster".as_slice(),
            b"PUBLIC invalid raster".len() as u64,
            ServiceError::Icon(taypeer_services::icons::IconError::InvalidImage),
        ),
    ] {
        assert_eq!(
            service
                .import_icon_stream(
                    &session,
                    &form.draft,
                    bytes,
                    IconImport {
                        length,
                        source: IconSource::File
                    },
                    &operation
                )
                .err(),
            Some(expected)
        );
        assert_eq!(service.active_draft(&session).unwrap().value.unwrap(), form);
        assert_eq!(
            service.storage_usage(&session).unwrap().value.draft_bytes,
            before_usage
        );
    }
    assert_eq!(
        service
            .import_icon_stream(
                &session,
                &form.draft,
                Unreadable,
                IconImport {
                    length: ICON_LIMIT + 1,
                    source: IconSource::File
                },
                &operation
            )
            .err(),
        Some(ServiceError::Icon(
            taypeer_services::icons::IconError::TooLarge
        ))
    );
    let draft_path = directory.path().join("PUBLIC.taypeer.draft");
    fs::create_dir(&draft_path).unwrap();
    assert_eq!(
        service
            .import_icon_stream(
                &session,
                &form.draft,
                FIRST,
                IconImport {
                    length: FIRST.len() as u64,
                    source: IconSource::File
                },
                &operation
            )
            .err(),
        Some(ServiceError::Storage(taypeer_storage::Error::Io))
    );
    assert_eq!(service.active_draft(&session).unwrap().value.unwrap(), form);
    assert!(
        service
            .icon_import_receipt(&session, &form.draft, &IconSource::File, &operation)
            .unwrap()
            .value
            .is_none()
    );
    assert_eq!(
        service.storage_usage(&session).unwrap().value.draft_bytes,
        before_usage
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir(draft_path).unwrap();
    import(&mut service, &session, &form.draft, FIRST, &operation);
}

#[test]
fn icon_import_rejects_a_different_active_form_database_metadata_and_revoked_session_before_read() {
    let mut service = DatabaseService::new();
    let database = service.create_database("PUBLIC icon scope").unwrap();
    let session = service
        .unlock(&database, taypeer_services::DEMO_PASSWORD)
        .unwrap();
    let first = entry_form(&mut service, &session);
    service.start_create_group(&session, None).unwrap();
    let operation = new_operation_id().unwrap();
    assert_eq!(
        service
            .import_icon_stream(
                &session,
                &first.draft,
                Unreadable,
                IconImport {
                    length: 0,
                    source: IconSource::File
                },
                &operation
            )
            .err(),
        Some(ServiceError::NoDraft)
    );
    let database_form = service
        .start_edit_database_info(&session)
        .unwrap()
        .value
        .identity;
    assert_eq!(
        service
            .import_icon_stream(
                &session,
                &database_form.draft,
                Unreadable,
                IconImport {
                    length: 0,
                    source: IconSource::File
                },
                &operation
            )
            .err(),
        Some(ServiceError::InvalidContext)
    );
    service.lock(&session).unwrap();
    assert_eq!(
        service
            .icon_import_receipt(
                &session,
                &database_form.draft,
                &IconSource::File,
                &operation
            )
            .err(),
        Some(ServiceError::Locked)
    );
}
