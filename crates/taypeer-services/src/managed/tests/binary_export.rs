//! PUBLIC ciphertext fixtures: an export preflight never preserves revoked read authority.
use super::*;
use crate::{AttachmentEdit, BinaryEdit, BinaryRequest, BinaryTarget};

#[test]
fn selected_export_rechecks_learned_revocation_after_successful_preflight_before_any_output() {
    let directory = tempfile::tempdir().unwrap();
    let manager = Profile::new(117);
    let member = Profile::new(118);
    let (mut authority, source) = create(
        &manager,
        directory.path().join("PUBLIC export authority.taypeer"),
    );
    let group = authority
        .create_group(
            &source,
            "PUBLIC export group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    authority.start_create_entry(&source, group).unwrap();
    authority
        .update_draft(
            &source,
            EditableEntry {
                title: "PUBLIC export entry".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = authority
        .save_draft(&source, &crate::new_operation_id().unwrap())
        .unwrap()
        .value;
    let input = directory.path().join("PUBLIC selected binary");
    let contents = b"PUBLIC scoped export original";
    std::fs::write(&input, contents).unwrap();
    let target = BinaryTarget::Entry(entry.clone());
    authority
        .edit_binary(
            &source,
            &BinaryRequest {
                target: target.clone(),
                review: None,
                edit: BinaryEdit::Attachment(AttachmentEdit::Add {
                    path: input,
                    name: None,
                }),
            },
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    admit(&mut authority, &source, &manager, &member);
    let path = directory.path().join("PUBLIC export member.taypeer");
    copy_to(&manager, &source.database, &path);
    let (mut reader, session) = member.open(path.clone(), PASSWORD);
    reader.apply_received(&session).unwrap();
    let view = reader.binary_view(&session, &target).unwrap().value;
    let blob = view.attachments[0].contents[0].id.clone();
    let before = std::fs::read(&path).unwrap();
    let history: Vec<_> = authority
        .history(&source, &entry)
        .unwrap()
        .value
        .into_iter()
        .map(|revision| revision.id)
        .collect();

    assert_eq!(
        reader
            .binary_export_length(&session, &target, &blob)
            .unwrap()
            .value,
        contents.len() as u64,
    );
    let mut accepted = Vec::new();
    reader
        .export_binary_stream(&session, &target, &blob, &mut accepted)
        .unwrap();
    assert_eq!(accepted, contents);
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // Model acquiring a selected output after successful validation. The
    // authenticated new control can arrive before the subsequent stream call.
    reader
        .binary_export_length(&session, &target, &blob)
        .unwrap();
    authority
        .rotate_password(
            &source,
            Digest::of(b"PUBLIC revoke between selected output preflight and export"),
            NEW_PASSWORD,
            Some(member.identity().device),
        )
        .unwrap();
    deliver(&manager, &member, &source.database);
    let received = std::fs::read(&path).unwrap();
    let mut denied = Vec::new();
    assert!(matches!(
        reader.export_binary_stream(&session, &target, &blob, &mut denied),
        Err(ServiceError::ExpiredSession)
    ));
    assert!(denied.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), received);
    assert_eq!(
        authority
            .history(&source, &entry)
            .unwrap()
            .value
            .into_iter()
            .map(|revision| revision.id)
            .collect::<Vec<_>>(),
        history,
    );
    reader.lock(&session).unwrap();
    authority.lock(&source).unwrap();
}
