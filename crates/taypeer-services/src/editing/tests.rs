//! Fixed PUBLIC legacy metadata shapes protect pending snapshot fingerprints.
use super::*;

struct Unreadable;
impl std::io::Read for Unreadable {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("aggregate local operation limit must reject before provider access")
    }
}

#[test]
fn aggregate_local_operation_limit_preserves_receipt_replay_and_blocks_new_acquisition() {
    let mut service = DatabaseService::new();
    let database = service
        .create_database("PUBLIC icon receipt budget")
        .unwrap();
    let session = service.unlock(&database, DEMO_PASSWORD).unwrap();
    let form = service
        .start_create_group(&session, None)
        .unwrap()
        .value
        .identity;
    let bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#224466"/></svg>"##;
    let operation = OperationId::new("PUBLIC acknowledged icon at limit");
    let imported = service
        .import_icon_stream(
            &session,
            &form.draft,
            bytes.as_slice(),
            IconImport {
                length: bytes.len() as u64,
                source: IconSource::File,
            },
            &operation,
        )
        .unwrap()
        .value;
    {
        let state = service.checked_mut(&session).unwrap();
        state
            .drafts
            .cancelled
            .extend((0..MAX_LOCAL_OPERATIONS - 1).map(|index| {
                OperationId::new(format!("PUBLIC previously discarded snapshot {index}"))
            }));
        state.drafts.validate(&database).unwrap();
        assert!(serde_json::to_vec(&state.drafts).unwrap().len() < taypeer_storage::MAX_FILE_SIZE);
    }
    assert_eq!(
        service
            .import_icon_stream(
                &session,
                &form.draft,
                Unreadable,
                IconImport {
                    length: u64::MAX,
                    source: IconSource::File
                },
                &operation
            )
            .unwrap()
            .value,
        imported
    );
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
                &OperationId::new("PUBLIC new operation must not evict receipts")
            )
            .err(),
        Some(ServiceError::InvalidContext)
    );
    assert_eq!(
        service.active_draft(&session).unwrap().value.unwrap(),
        imported
    );
    assert_eq!(
        service
            .icon_import_receipt(&session, &form.draft, &IconSource::File, &operation)
            .unwrap()
            .value,
        Some(imported)
    );
}

#[test]
fn legacy_pending_group_snapshot_keeps_its_fingerprint_without_empty_icon_receipts() {
    // Preserve the previous typed field order: serde_json::Value sorts object
    // keys and therefore cannot represent the historical fingerprint input.
    #[derive(Serialize)]
    struct LegacyMetadataDraft {
        identity: DraftIdentity,
        base: Vec<String>,
        baseline: MetadataFields,
        fields: MetadataFields,
        changed: BTreeSet<MetadataField>,
    }
    #[derive(Serialize)]
    enum LegacyCapturedDraft {
        Metadata(LegacyMetadataDraft),
    }

    let database = DatabaseId::new("PUBLIC legacy icon collection");
    let legacy = serde_json::json!({
        "Metadata": {
            "identity": {
                "draft": "PUBLIC legacy group draft",
                "target": {"Group": "PUBLIC legacy group"},
                "revision": 1
            },
            "base": [],
            "baseline": {"name": "PUBLIC old group", "description": null, "icon": {"kind": "default"}},
            "fields": {"name": "PUBLIC changed group", "description": null, "icon": {"kind": "default"}},
            "changed": ["name"]
        }
    });
    let captured: CapturedDraft = serde_json::from_value(legacy.clone()).unwrap();
    let identity = captured.identity();
    assert_eq!(serde_json::to_value(&captured).unwrap(), legacy);
    let CapturedDraft::Metadata(draft) = &captured else {
        panic!("PUBLIC fixture is metadata")
    };
    let legacy_captured = LegacyCapturedDraft::Metadata(LegacyMetadataDraft {
        identity: draft.identity.clone(),
        base: draft.base.clone(),
        baseline: draft.baseline.clone(),
        fields: draft.fields.clone(),
        changed: draft.changed.clone(),
    });
    assert_eq!(
        serde_json::to_vec(&captured).unwrap(),
        serde_json::to_vec(&legacy_captured).unwrap()
    );
    let old_fingerprint = commands::fingerprint(&(&identity, &legacy_captured)).unwrap();
    let mut collection = DraftCollection::new(database.clone());
    collection
        .metadata
        .insert(identity.draft.clone(), (**draft).clone());
    collection.attempts.insert(
        OperationId::new("PUBLIC legacy pending Save"),
        SnapshotAttempt {
            identity,
            fingerprint: old_fingerprint,
            captured: Some(captured),
            outcome: None,
        },
    );
    let encoded = serde_json::to_vec(&collection).unwrap();
    let loaded: DraftCollection = serde_json::from_slice(&encoded).unwrap();
    loaded.validate(&database).unwrap();
    let pending = loaded.attempts.values().next().unwrap();
    assert!(pending.outcome.is_none());
    assert_eq!(
        commands::fingerprint(&(&pending.identity, pending.captured.as_ref().unwrap())).unwrap(),
        pending.fingerprint
    );
}

#[test]
fn legacy_parentless_pending_snapshot_reopens_without_hydration_and_keeps_its_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC legacy parent.taypeer");
    let password = b"PUBLIC legacy parent master";
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC legacy parent".into(), password)
        .unwrap();
    let parent = service
        .create_group(
            &session,
            "PUBLIC parent".into(),
            None,
            &new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    let group = service
        .create_group(
            &session,
            "PUBLIC child".into(),
            Some(parent.clone()),
            &new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    let form = service.start_edit_group(&session, &group).unwrap().value;
    // Generate the previous on-disk shape from an actual encrypted document basis.
    service
        .checked_mut(&session)
        .unwrap()
        .drafts
        .metadata
        .get_mut(&form.identity.draft)
        .unwrap()
        .parent = None;
    let identity = service
        .patch_group_draft(
            &session,
            &form.identity.draft,
            GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC legacy retained text".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .value;
    let operation = new_operation_id().unwrap();
    let state = service.checked_mut(&session).unwrap();
    let captured = state.drafts.captured(&identity.draft).unwrap();
    let typed = serde_json::to_vec(&captured).unwrap();
    assert!(
        !serde_json::to_value(&captured).unwrap()["Metadata"]
            .as_object()
            .unwrap()
            .contains_key("parent")
    );
    let fingerprint = commands::fingerprint(&(&identity, &captured)).unwrap();
    state.drafts.attempts.insert(
        operation.clone(),
        SnapshotAttempt {
            identity: identity.clone(),
            fingerprint: fingerprint.clone(),
            captured: Some(captured),
            outcome: None,
        },
    );
    service.persist_drafts(&session).unwrap();
    service.lock(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, password).unwrap();
    assert_eq!(
        service
            .resume_draft(&session, &identity.draft)
            .unwrap()
            .value,
        identity
    );
    let view = service
        .metadata_draft(&session, &identity.draft)
        .unwrap()
        .value;
    assert_eq!(view.parent.as_ref(), Some(&parent));
    assert_eq!(view.name, "PUBLIC legacy retained text");
    let pending = &service.checked(&session).unwrap().drafts.attempts[&operation];
    assert_eq!(
        serde_json::to_vec(pending.captured.as_ref().unwrap()).unwrap(),
        typed
    );
    assert_eq!(pending.fingerprint, fingerprint);
    let saved = service
        .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
        .unwrap()
        .value;
    let confirmed = std::fs::read(&path).unwrap();
    assert_eq!(
        service.group_history(&session, &group).unwrap().value.len(),
        2
    );
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
    service
        .move_group(
            &session,
            &taypeer_document::GroupMove {
                group: group.clone(),
                parent: None,
                position: taypeer_document::SiblingPosition::Last,
                review: None,
                name: None,
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let after_move = std::fs::read(&path).unwrap();
    assert_ne!(after_move, confirmed);
    assert_eq!(
        service
            .save_draft_snapshot(&session, &identity.draft, identity.revision, &operation)
            .unwrap()
            .value,
        saved
    );
    assert_eq!(std::fs::read(&path).unwrap(), after_move);
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
