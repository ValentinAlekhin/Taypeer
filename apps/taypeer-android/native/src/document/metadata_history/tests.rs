//! PUBLIC shared projections exercise the real Native DTO boundary without fabricated entries.
use super::*;
use taypeer_services::{
    BinaryEdit, BinaryRequest, BinaryTarget, DatabaseService, GroupForm, IconInput,
};

fn fixture() -> (DatabaseService, taypeer_services::SessionToken, GroupId) {
    let mut service = DatabaseService::new();
    let database = service.create_database("PUBLIC metadata history").unwrap();
    let session = service
        .unlock(&database, taypeer_services::DEMO_PASSWORD)
        .unwrap();
    let group = service
        .create_group(
            &session,
            "PUBLIC original group".into(),
            None,
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    (service, session, group)
}

#[test]
fn metadata_dto_keeps_original_and_current_fields_separate_and_never_exports_fake_entry_fields() {
    let (mut service, session, group) = fixture();
    let saved = service
        .group_history(&session, &group)
        .unwrap()
        .value
        .pop()
        .unwrap();
    service
        .save_group_form(
            &session,
            GroupForm {
                id: Some(group.clone()),
                parent: None,
                name: "PUBLIC current group".into(),
                description: Some("  PUBLIC description\nТекст  ".into()),
                icon: taypeer_core::IconRef::Lucide("folder".to_owned().try_into().unwrap()),
            },
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    let current_history = service.group_history(&session, &group).unwrap().value;
    let view = group_view(
        service
            .group_revision_view(&session, &group, &saved.id)
            .unwrap()
            .value,
    )
    .unwrap();
    assert!(matches!(view.kind, FormKind::Group));
    assert_eq!(view.target.as_deref(), Some(group.as_str()));
    assert_eq!(view.saved.name, saved.snapshot.group.name);
    assert_eq!(view.current.name, "PUBLIC current group");
    assert_eq!(
        view.current.description.as_deref(),
        Some("  PUBLIC description\nТекст  ")
    );
    assert!(matches!(view.saved.icon, Some(IconView::Default)));
    assert!(matches!(view.current.icon, Some(IconView::Lucide { ref key }) if key == "folder"));
    assert_eq!(view.revision, saved.id.to_string());
    assert_eq!(view.saved_at, saved.saved_at);
    assert_eq!(format!("{view:?}"), "MetadataRevisionView([REDACTED])");
    reviewed_heads(view.basis).unwrap();
    assert_eq!(
        service.group_history(&session, &group).unwrap().value,
        current_history
    );
    assert!(service.drafts(&session).unwrap().value.is_empty());

    let revision = service
        .database_history(&session)
        .unwrap()
        .value
        .pop()
        .unwrap()
        .id;
    let view = database_view(
        service
            .database_revision_view(&session, &revision)
            .unwrap()
            .value,
    )
    .unwrap();
    assert!(matches!(view.kind, FormKind::Database));
    assert!(view.target.is_none());
    assert!(view.saved.icon.is_none() && view.current.icon.is_none());
    assert!(view.saved.parent.is_none() && view.current.parent.is_none());
    assert!(view.parent_candidates.is_empty());
}

#[test]
fn historical_group_icon_scope_uses_the_exact_selected_image_after_current_lucide_and_purge_revokes_it()
 {
    let (mut service, session, group) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC original.svg");
    let image = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#22aa44"/></svg>"##;
    std::fs::write(&path, image).unwrap();
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target: BinaryTarget::Group(group.clone()),
                edit: BinaryEdit::Icon(IconInput::File(path.clone())),
                review: None,
            },
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    let original = service
        .group_history(&session, &group)
        .unwrap()
        .value
        .into_iter()
        .find(|row| matches!(row.snapshot.group.icon, taypeer_core::IconRef::Image { .. }))
        .unwrap();
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target: BinaryTarget::Group(group.clone()),
                edit: BinaryEdit::Icon(IconInput::Lucide("folder".to_owned().try_into().unwrap())),
                review: None,
            },
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    let target = BinaryTarget::GroupRevision {
        group: group.clone(),
        revision: original.id.clone(),
    };
    let binary = service.binary_view(&session, &target).unwrap().value;
    assert_eq!(binary.icons, vec![original.snapshot.group.icon.clone()]);
    assert!(matches!(
        service
            .binary_view(&session, &BinaryTarget::Group(group.clone()))
            .unwrap()
            .value
            .icons
            .as_slice(),
        [taypeer_core::IconRef::Lucide(_)]
    ));
    let blob = original.snapshot.group.icon.blob().unwrap();
    let mut output = Vec::new();
    service
        .export_binary_stream(&session, &target, blob, &mut output)
        .unwrap();
    assert_eq!(output, image);
    assert_eq!(std::fs::read(&path).unwrap(), image);
    service
        .purge_group_history(
            &session,
            &group,
            [original.id].into_iter().collect(),
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    assert!(matches!(
        service.binary_view(&session, &target),
        Err(ServiceError::NotFound)
    ));
    service.lock(&session).unwrap();
    assert!(matches!(
        service.binary_view(&session, &target),
        Err(ServiceError::Locked | ServiceError::ExpiredSession)
    ));
}

#[test]
fn exact_restore_rejections_are_definite_but_storage_and_transport_failures_remain_uncertain() {
    for rejection in [
        ServiceError::OperationConflict,
        ServiceError::InvalidContext,
        ServiceError::NotFound,
        ServiceError::InvalidInput,
    ] {
        assert!(matches!(
            restore_error(RuntimeError::Service(rejection)),
            AndroidError::InvalidInput
        ));
    }
    assert!(matches!(
        restore_error(RuntimeError::Service(ServiceError::Storage(
            taypeer_storage::Error::CommitUncertain
        ))),
        AndroidError::CommitUncertain
    ));
    assert!(matches!(
        restore_error(RuntimeError::Service(ServiceError::Storage(
            taypeer_storage::Error::Io
        ))),
        AndroidError::StorageIo
    ));
    assert!(matches!(
        restore_error(RuntimeError::Protocol),
        AndroidError::Runtime
    ));
    assert!(reviewed_heads(Vec::new()).is_err());
    assert!(reviewed_heads(vec!["PUBLIC invalid context".into()]).is_err());
}
