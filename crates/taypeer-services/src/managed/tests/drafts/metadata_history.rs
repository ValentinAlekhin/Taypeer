//! Actual encrypted metadata restores and before/after-publication uncertainty; PUBLIC only.
use super::*;
use crate::{DatabaseRevisionRestore, GroupRevisionRestore};
use taypeer_core::{GroupId, IconRef};

fn group(service: &mut DatabaseService, session: &SessionToken, name: &str) -> GroupId {
    service
        .create_group(
            session,
            name.into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id
}

#[test]
fn metadata_history_group_restore_replays_after_encrypted_restart_late_edit_and_source_purge() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(86);
    let path = directory.path().join("PUBLIC group history.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let source = group(&mut service, &session, "PUBLIC original group");
    let parent = group(&mut service, &session, "PUBLIC explicit destination");
    let saved = service
        .group_history(&session, &source)
        .unwrap()
        .value
        .pop()
        .unwrap();
    service
        .save_group_form(
            &session,
            crate::GroupForm {
                id: Some(source.clone()),
                parent: None,
                name: "PUBLIC current group".into(),
                description: Some("  PUBLIC current description\nТекст  ".into()),
                icon: IconRef::Default,
            },
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let comparison = service
        .group_revision_view(&session, &source, &saved.id)
        .unwrap()
        .value;
    assert_eq!(comparison.saved, saved);
    assert_eq!(comparison.current.group.name, "PUBLIC current group");
    assert!(
        comparison
            .parent_candidates
            .iter()
            .any(|row| row.id == parent)
    );
    assert!(service.drafts(&session).unwrap().value.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let request = GroupRevisionRestore {
        group: source.clone(),
        revision: saved.id.clone(),
        parent: Some(parent.clone()),
        heads: comparison.basis,
    };
    let operation = crate::new_operation_id().unwrap();
    assert_eq!(
        service
            .restore_group_revision(&session, &request, &operation)
            .unwrap()
            .value,
        source
    );
    assert_eq!(
        service
            .group_history(&session, &source)
            .unwrap()
            .value
            .len(),
        3
    );
    assert_eq!(
        service
            .group_history(&session, &source)
            .unwrap()
            .value
            .last()
            .unwrap()
            .kind,
        taypeer_core::RevisionKind::Restore
    );
    service
        .save_group_form(
            &session,
            crate::GroupForm {
                id: Some(source.clone()),
                parent: None,
                name: "PUBLIC later independent group".into(),
                description: Some("PUBLIC late description".into()),
                icon: IconRef::Default,
            },
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    service
        .purge_group_history(
            &session,
            &source,
            [saved.id.clone()].into_iter().collect(),
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    let confirmed = std::fs::read(&path).unwrap();
    let history = service.group_history(&session, &source).unwrap().value;
    service.lock(&session).unwrap();
    drop(service);
    profile.coordinator.unregister(&session.database).unwrap();
    let (mut service, session) = profile.open(path.clone(), PASSWORD);
    assert_eq!(
        service
            .restore_group_revision(&session, &request, &operation)
            .unwrap()
            .value,
        source
    );
    assert_eq!(std::fs::read(&path).unwrap(), confirmed);
    assert_eq!(
        service.group_history(&session, &source).unwrap().value,
        history
    );
    assert_eq!(
        service
            .groups(&session)
            .unwrap()
            .value
            .iter()
            .find(|row| row.id == source)
            .unwrap()
            .name,
        "PUBLIC later independent group"
    );
    let mut conflict = request;
    conflict.parent = None;
    assert_eq!(
        service
            .restore_group_revision(&session, &conflict, &operation)
            .err(),
        Some(ServiceError::OperationConflict)
    );
    assert_eq!(std::fs::read(&path).unwrap(), confirmed);
    assert!(matches!(
        service.group_revision_view(&session, &source, &saved.id),
        Err(ServiceError::NotFound)
    ));
}

#[test]
fn metadata_history_database_restore_is_read_only_to_compare_and_exact_across_cold_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(87);
    let path = directory.path().join("PUBLIC database history.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let original = service
        .database_history(&session)
        .unwrap()
        .value
        .pop()
        .unwrap();
    service
        .update_database_info(
            &session,
            "PUBLIC changed database".into(),
            Some("  PUBLIC changed\nТекст  ".into()),
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let comparison = service
        .database_revision_view(&session, &original.id)
        .unwrap()
        .value;
    assert_eq!(comparison.saved, original);
    assert_eq!(comparison.current.name, "PUBLIC changed database");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(service.drafts(&session).unwrap().value.is_empty());
    let policy = service.database_policy(&session).unwrap().value;
    let request = DatabaseRevisionRestore {
        revision: original.id.clone(),
        heads: comparison.basis,
    };
    let operation = crate::new_operation_id().unwrap();
    assert_eq!(
        service
            .restore_database_revision(&session, &request, &operation)
            .unwrap()
            .value,
        session.database
    );
    assert_eq!(
        service.database_info(&session).unwrap().name,
        original.snapshot.name
    );
    assert_eq!(service.database_policy(&session).unwrap().value, policy);
    assert_eq!(service.database_history(&session).unwrap().value.len(), 3);
    service
        .update_database_info(
            &session,
            "PUBLIC latest database".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    service
        .purge_database_history(
            &session,
            [original.id].into_iter().collect(),
            &crate::new_operation_id().unwrap(),
        )
        .unwrap();
    let history = service.database_history(&session).unwrap().value;
    let confirmed = std::fs::read(&path).unwrap();
    service.lock(&session).unwrap();
    drop(service);
    profile.coordinator.unregister(&session.database).unwrap();
    let (mut service, session) = profile.open(path.clone(), PASSWORD);
    service
        .restore_database_revision(&session, &request, &operation)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), confirmed);
    assert_eq!(
        service.database_info(&session).unwrap().name,
        "PUBLIC latest database"
    );
    assert_eq!(service.database_history(&session).unwrap().value, history);
    let mut conflict = request;
    conflict.heads = Vec::new();
    assert_eq!(
        service
            .restore_database_revision(&session, &conflict, &operation)
            .err(),
        Some(ServiceError::OperationConflict)
    );
    assert_eq!(std::fs::read(&path).unwrap(), confirmed);
}

#[test]
fn metadata_history_restore_uncertainty_before_or_after_publication_never_reports_false_ack() {
    for published in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let profile = Profile::new(88);
        let path = directory.path().join("PUBLIC uncertain Restore.taypeer");
        let (mut seed, session) = create(&profile, path.clone());
        let source = group(&mut seed, &session, "PUBLIC source");
        let original = seed
            .group_history(&session, &source)
            .unwrap()
            .value
            .pop()
            .unwrap();
        let database = session.database.clone();
        seed.lock(&session).unwrap();
        drop(seed);
        let failures = Arc::new(Failures::default());
        let (mut service, session) =
            reopen(&profile, path.clone(), database, Arc::clone(&failures));
        let view = service
            .group_revision_view(&session, &source, &original.id)
            .unwrap()
            .value;
        let request = GroupRevisionRestore {
            group: source.clone(),
            revision: original.id,
            parent: None,
            heads: view.basis,
        };
        let operation = crate::new_operation_id().unwrap();
        let before = std::fs::read(&path).unwrap();
        if published {
            failures.uncertain_commit.store(true, Ordering::SeqCst);
        } else {
            failures
                .uncertain_before_commit
                .store(true, Ordering::SeqCst);
        }
        assert!(matches!(
            service.restore_group_revision(&session, &request, &operation),
            Err(ServiceError::Storage(StorageError::CommitUncertain))
        ));
        assert_eq!(
            service
                .group_history(&session, &source)
                .unwrap()
                .value
                .len(),
            1
        );
        let candidate = std::fs::read(&path).unwrap();
        assert_eq!(candidate != before, published);
        assert!(!service.can_write(&session).unwrap());
        service.reconcile_uncertain(&session).unwrap();
        assert_eq!(
            service
                .group_history(&session, &source)
                .unwrap()
                .value
                .len(),
            1 + usize::from(published)
        );
        assert_eq!(
            service
                .restore_group_revision(&session, &request, &operation)
                .unwrap()
                .value,
            source
        );
        assert_eq!(
            service
                .group_history(&session, &source)
                .unwrap()
                .value
                .len(),
            2
        );
        let confirmed = std::fs::read(&path).unwrap();
        service
            .restore_group_revision(&session, &request, &operation)
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), confirmed);
    }
}
