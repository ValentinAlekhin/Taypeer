//! Synthetic local branches model quota growth; no network admission is involved.
use super::*;
use std::io::Read;
use taypeer_core::ATTACHMENT_LIMIT;

#[test]
fn merged_contents_over_quota_remain_readable_and_new_additions_fail() {
    let mut service = DatabaseService::new();
    let database = service.create_database("PUBLIC quota").unwrap();
    let session = service.unlock(&database, DEMO_PASSWORD).unwrap();
    let state = service.checked_mut(&session).unwrap();
    let group = state
        .document
        .as_mut()
        .unwrap()
        .create_group("PUBLIC group".into(), None, 1)
        .unwrap()
        .id;
    let mut left = state.document().fork();
    let mut right = state.document().fork();
    let mut blobs = state.blobs().unwrap().clone();
    let mut last = None;
    for index in 0..11_u8 {
        // Each branch stays under 100 MiB; all contents are distinct public patterns.
        let branch = if index < 6 { &mut left } else { &mut right };
        let mut draft = branch.begin_create_entry(group.clone()).unwrap();
        draft.fields_mut().title = format!("PUBLIC {index}");
        let blob = blobs
            .insert(
                std::io::repeat(index).take(ATTACHMENT_LIMIT),
                ATTACHMENT_LIMIT,
                ATTACHMENT_LIMIT,
            )
            .unwrap();
        let attachment = draft.add_attachment("PUBLIC pattern".into(), blob.clone());
        let entry = branch.save_entry(draft, 2).unwrap();
        last = Some((entry, attachment, blob));
    }
    left.merge(&right).unwrap();
    state.commit_blobs(left, blobs).unwrap();
    let (entry, attachment, blob) = last.unwrap();
    let target = BinaryTarget::Entry(entry.clone());
    let usage = service.storage_usage(&session).unwrap().value;
    assert!(usage.over_limit);
    assert_eq!(usage.attachment_bytes, 11 * ATTACHMENT_LIMIT);
    assert_eq!(
        service
            .binary_view(&session, &target)
            .unwrap()
            .value
            .attachments
            .len(),
        1
    );
    let directory = tempfile::tempdir().unwrap();
    service
        .export_binary(
            &session,
            &target,
            &blob,
            &directory.path().join("PUBLIC-export"),
            false,
        )
        .unwrap();
    let source = directory.path().join("PUBLIC-small");
    std::fs::write(&source, b"PUBLIC new").unwrap();
    let request = BinaryRequest {
        target: target.clone(),
        edit: BinaryEdit::Attachment(AttachmentEdit::Add {
            path: source,
            name: None,
        }),
        review: None,
    };
    assert_eq!(
        service
            .edit_binary(&session, &request, &OperationId::new("PUBLIC blocked"))
            .err(),
        Some(ServiceError::AttachmentLimit)
    );
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target,
                edit: BinaryEdit::Attachment(AttachmentEdit::Remove { attachment }),
                review: None,
            },
            &OperationId::new("PUBLIC remove"),
        )
        .unwrap();
    // Removal keeps the old version available, so quota remains above the limit until history is purged.
    assert!(service.storage_usage(&session).unwrap().value.over_limit);
}

#[test]
fn per_file_boundary_is_exact_and_rejection_preserves_staging() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC-sized");
    let file = File::create(&path).unwrap();
    let mut blobs = BlobStore::new().unwrap();
    file.set_len(ATTACHMENT_LIMIT).unwrap();
    let id = stage_file(&mut blobs, &path, DatabasePolicy::default()).unwrap();
    assert_eq!(blobs.length(&id), Some(ATTACHMENT_LIMIT));
    file.set_len(ATTACHMENT_LIMIT + 1).unwrap();
    assert_eq!(
        stage_file(&mut blobs, &path, DatabasePolicy::default()).err(),
        Some(ServiceError::AttachmentLimit)
    );
    assert_eq!(blobs.ids().count(), 1);
}
