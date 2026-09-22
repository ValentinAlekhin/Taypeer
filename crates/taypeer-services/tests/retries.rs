//! Public synthetic requests, authenticated reopen, and byte-for-byte retry checks.
use taypeer_services::{
    DatabaseService, EditableEntry, EntryPatch, FieldUpdate, ServiceError, new_operation_id,
};
const PASSWORD: &[u8] = b"PUBLIC retry master password";
fn patch(title: &str) -> EntryPatch {
    EntryPatch {
        title: FieldUpdate::Set(title.into()),
        ..Default::default()
    }
}
#[test]
fn exact_commands_survive_restart_without_reverting_later_changes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut db = DatabaseService::new();
    let session = db
        .create_file(&path, "PUBLIC database".into(), PASSWORD)
        .unwrap();
    let create_group = new_operation_id().unwrap();
    let rename_group = new_operation_id().unwrap();
    let create_entry = new_operation_id().unwrap();
    let update_entry = new_operation_id().unwrap();
    let metadata = new_operation_id().unwrap();
    let save = new_operation_id().unwrap();
    let group = db
        .create_group(&session, "PUBLIC group".into(), None, &create_group)
        .unwrap()
        .value;
    let renamed = db
        .update_group(&session, &group.id, "PUBLIC renamed".into(), &rename_group)
        .unwrap()
        .value;
    let entry = db
        .create_entry(
            &session,
            group.id.clone(),
            patch("PUBLIC original"),
            &create_entry,
        )
        .unwrap()
        .value;
    db.update_entry(&session, &entry, patch("PUBLIC updated"), &update_entry)
        .unwrap();
    db.update_database_info(
        &session,
        "PUBLIC updated database".into(),
        Some("PUBLIC notes".into()),
        &metadata,
    )
    .unwrap();
    db.start_edit_entry(&session, &entry).unwrap();
    db.patch_draft(&session, patch("PUBLIC saved draft"))
        .unwrap();
    db.save_draft(&session, &save).unwrap();
    for reopen in [false, true] {
        let session = if reopen {
            drop(db);
            db = DatabaseService::new();
            db.open_file(&path, PASSWORD).unwrap()
        } else {
            session.clone()
        };
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            db.create_group(&session, "PUBLIC group".into(), None, &create_group)
                .unwrap()
                .value
                .id,
            group.id
        );
        assert_eq!(
            db.update_group(&session, &group.id, "PUBLIC renamed".into(), &rename_group)
                .unwrap()
                .value
                .name,
            renamed.name
        );
        assert_eq!(
            db.create_entry(
                &session,
                group.id.clone(),
                patch("PUBLIC original"),
                &create_entry
            )
            .unwrap()
            .value,
            entry
        );
        assert_eq!(
            db.update_entry(&session, &entry, patch("PUBLIC updated"), &update_entry)
                .unwrap()
                .value,
            entry
        );
        db.update_database_info(
            &session,
            "PUBLIC updated database".into(),
            Some("PUBLIC notes".into()),
            &metadata,
        )
        .unwrap();
        assert_eq!(db.save_draft(&session, &save).unwrap().value, entry);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(db.groups(&session).unwrap().value.len(), 1);
        assert_eq!(db.entries(&session, None, "").unwrap().value.len(), 1);
        assert_eq!(
            db.view_entry(&session, &entry).unwrap().value.title,
            "PUBLIC saved draft"
        );
        assert_eq!(db.history(&session, &entry).unwrap().value.len(), 3);
        assert_eq!(
            db.create_group(&session, "PUBLIC different".into(), None, &create_group)
                .unwrap_err(),
            ServiceError::OperationConflict
        );
        assert_eq!(
            db.update_group(&session, &group.id, "PUBLIC other".into(), &create_group)
                .unwrap_err(),
            ServiceError::OperationConflict
        );
        assert_eq!(
            db.update_entry(&session, &entry, patch("PUBLIC different"), &update_entry)
                .unwrap_err(),
            ServiceError::OperationConflict
        );
        assert_eq!(
            db.save_draft(&session, &create_entry).unwrap_err(),
            ServiceError::OperationConflict
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        db.start_create_entry(&session, group.id.clone()).unwrap();
        db.update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC another draft".into(),
                ..Default::default()
            },
        )
        .unwrap();
        db.save_draft(&session, &save).unwrap();
        assert_eq!(
            db.draft(&session).unwrap().value.unwrap().fields.title,
            "PUBLIC another draft"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        db.cancel_draft(&session).unwrap();
    }
}

#[test]
fn failed_write_keeps_operation_available_and_permissions_precede_receipts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let displaced = directory.path().join("PUBLIC displaced");
    let mut db = DatabaseService::new();
    let session = db
        .create_file(&path, "PUBLIC database".into(), PASSWORD)
        .unwrap();
    let operation = new_operation_id().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::rename(&path, &displaced).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        db.create_group(&session, "PUBLIC group".into(), None, &operation)
            .is_err()
    );
    assert_eq!(std::fs::read(&displaced).unwrap(), bytes);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&displaced, &path).unwrap();
    let group = db
        .create_group(&session, "PUBLIC group".into(), None, &operation)
        .unwrap()
        .value;
    assert_eq!(
        db.create_group(&session, "PUBLIC group".into(), None, &operation)
            .unwrap()
            .value
            .id,
        group.id
    );
    db.lock(&session).unwrap();
    assert_eq!(
        db.create_group(&session, "PUBLIC group".into(), None, &operation)
            .unwrap_err(),
        ServiceError::Locked
    );
    db.unlock(&session.database, std::str::from_utf8(PASSWORD).unwrap())
        .unwrap();
    assert_eq!(
        db.create_group(&session, "PUBLIC group".into(), None, &operation)
            .unwrap_err(),
        ServiceError::ExpiredSession
    );
}
