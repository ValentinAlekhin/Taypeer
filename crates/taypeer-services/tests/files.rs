//! Synthetic end-to-end scenarios against actual encrypted files and restarted services.

use std::fs;
use taypeer_services::{DatabaseService, EditableEntry, ServiceError, StorageError};

const PASSWORD: &[u8] = b"  PUBLIC master password  ";

#[test]
fn encrypted_file_survives_restart_with_identity_values_and_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC hidden database title".into(), PASSWORD)
        .unwrap();
    assert_eq!(
        service
            .unlock(&session.database, std::str::from_utf8(PASSWORD).unwrap())
            .unwrap_err(),
        ServiceError::InvalidContext
    );
    assert!(service.is_current(&session));
    let group = service
        .create_group(&session, "PUBLIC hidden group".into(), None)
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group.clone()).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC hidden entry".into(),
                password: Some("PUBLIC first secret".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = service.save_draft(&session).unwrap().value;
    let mut form = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .fields;
    form.password = Some("PUBLIC second secret".into());
    service.update_draft(&session, form).unwrap();
    service.save_draft(&session).unwrap();
    let bytes = fs::read(&path).unwrap();
    for marker in ["PUBLIC", "hidden", "first secret", "second secret"] {
        assert!(
            !bytes
                .windows(marker.len())
                .any(|window| window == marker.as_bytes())
        );
    }
    drop(service);
    let mut reopened = DatabaseService::new();
    assert_eq!(
        reopened
            .open_file(&path, b"PUBLIC master password")
            .unwrap_err(),
        ServiceError::Storage(StorageError::Authentication)
    );
    assert!(reopened.databases().is_empty());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let restored = reopened.open_file(&path, PASSWORD).unwrap();
    assert_eq!(restored.database, session.database);
    assert_eq!(reopened.groups(&restored).unwrap().value[0].id, group);
    assert_eq!(
        reopened
            .reveal_password(&restored, &entry)
            .unwrap()
            .value
            .expose(),
        "PUBLIC second secret"
    );
    let history = reopened.history(&restored, &entry).unwrap().value;
    assert_eq!(history.len(), 2);
    assert_eq!(
        reopened
            .reveal_revision_password(&restored, &entry, &history[0].id)
            .unwrap()
            .value
            .expose(),
        "PUBLIC first secret"
    );
    reopened.lock(&restored).unwrap();
    assert_eq!(
        reopened.view_entry(&restored, &entry).unwrap_err(),
        ServiceError::Locked
    );
    let fresh = reopened
        .unlock(&restored.database, std::str::from_utf8(PASSWORD).unwrap())
        .unwrap();
    assert!(!reopened.is_current(&restored));
    assert_eq!(
        reopened
            .reveal_password(&fresh, &entry)
            .unwrap()
            .value
            .expose(),
        "PUBLIC second secret"
    );
}

#[test]
fn failed_backup_preserves_file_saved_state_and_retryable_draft() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC".into(), PASSWORD)
        .unwrap();
    fs::write(
        directory.path().join("fixture.taypeer.backups"),
        b"PUBLIC obstacle",
    )
    .unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        service
            .create_group(&session, "PUBLIC rejected".into(), None)
            .unwrap_err(),
        ServiceError::Storage(StorageError::Io)
    );
    assert!(service.groups(&session).unwrap().value.is_empty());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::remove_file(directory.path().join("fixture.taypeer.backups")).unwrap();
    let group = service
        .create_group(&session, "PUBLIC group".into(), None)
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC pending".into(),
                ..Default::default()
            },
        )
        .unwrap();
    fs::rename(
        directory.path().join("fixture.taypeer.backups"),
        directory.path().join("retained-backups"),
    )
    .unwrap();
    fs::write(
        directory.path().join("fixture.taypeer.backups"),
        b"PUBLIC obstacle",
    )
    .unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(service.save_draft(&session).is_err());
    assert!(
        service
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    assert_eq!(
        service.draft(&session).unwrap().value.unwrap().fields.title,
        "PUBLIC pending"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::remove_file(directory.path().join("fixture.taypeer.backups")).unwrap();
    service.save_draft(&session).unwrap();
    assert_eq!(service.entries(&session, None, "").unwrap().value.len(), 1);
}

#[test]
fn encrypted_interrupted_draft_survives_restart_and_explicit_discard() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC".into(), PASSWORD)
        .unwrap();
    let group = service
        .create_group(&session, "PUBLIC group".into(), None)
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC unsaved marker".into(),
                ..Default::default()
            },
        )
        .unwrap();
    service.lock(&session).unwrap();
    let local = fs::read(directory.path().join("fixture.taypeer.draft")).unwrap();
    assert!(!local.windows(6).any(|window| window == b"PUBLIC"));
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    assert!(service.draft(&session).unwrap().value.is_none());
    assert!(service.pending_draft(&session).unwrap().value.is_some());
    assert_eq!(
        service.save_draft(&session).unwrap_err(),
        ServiceError::DraftNeedsRestore
    );
    assert_eq!(
        service.restore_draft(&session).unwrap().value.fields.title,
        "PUBLIC unsaved marker"
    );
    service.cancel_draft(&session).unwrap();
    drop(service);
    let mut service = DatabaseService::new();
    let session = service.open_file(&path, PASSWORD).unwrap();
    assert!(service.pending_draft(&session).unwrap().value.is_none());
    assert!(
        service
            .entries(&session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
}

#[test]
fn draft_storage_failure_still_locks_and_revokes_access() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.taypeer");
    let mut service = DatabaseService::new();
    let session = service
        .create_file(&path, "PUBLIC".into(), PASSWORD)
        .unwrap();
    let group = service
        .create_group(&session, "PUBLIC group".into(), None)
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC unsaved".into(),
                ..Default::default()
            },
        )
        .unwrap();
    fs::create_dir(directory.path().join("fixture.taypeer.draft")).unwrap();
    assert!(service.lock(&session).is_err());
    assert!(!service.is_current(&session));
    assert_eq!(service.draft(&session).unwrap_err(), ServiceError::Locked);
}
