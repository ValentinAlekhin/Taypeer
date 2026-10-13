//! PUBLIC group Save preserves its exact parent receipt through uncertain durable publication.
use super::*;
use taypeer_core::{GroupMetadataPatch, IconRef};

#[test]
fn uncertain_parent_save_confirms_once_and_replay_preserves_later_input_after_restart() {
    for published in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let profile = Profile::new(72);
        let path = directory.path().join("PUBLIC uncertain parent.taypeer");
        let (mut seed, session) = create(&profile, path.clone());
        let group = seed
            .create_group(
                &session,
                "PUBLIC source".into(),
                None,
                &crate::new_operation_id().unwrap(),
            )
            .unwrap()
            .value
            .id;
        let parent = seed
            .create_group(
                &session,
                "PUBLIC destination".into(),
                None,
                &crate::new_operation_id().unwrap(),
            )
            .unwrap()
            .value
            .id;
        let database = session.database.clone();
        seed.lock(&session).unwrap();
        drop(seed);
        let failures = Arc::new(Failures::default());
        let (mut service, session) = reopen(
            &profile,
            path.clone(),
            database.clone(),
            Arc::clone(&failures),
        );
        let form = service.start_edit_group(&session, &group).unwrap().value;
        let icon = IconRef::Lucide("folder".to_owned().try_into().unwrap());
        let captured = service
            .patch_group_draft(
                &session,
                &form.identity.draft,
                GroupMetadataPatch {
                    name: FieldUpdate::Set("PUBLIC confirmed parent name".into()),
                    description: FieldUpdate::Set("PUBLIC confirmed parent description".into()),
                    icon: FieldUpdate::Set(icon.clone()),
                    parent: FieldUpdate::Set(parent.clone()),
                },
            )
            .unwrap()
            .value;
        let operation = crate::new_operation_id().unwrap();
        let before = std::fs::read(&path).unwrap();
        if published {
            failures.uncertain_commit.store(true, Ordering::SeqCst);
        } else {
            failures
                .uncertain_before_commit
                .store(true, Ordering::SeqCst);
        }
        assert_eq!(
            service
                .save_draft_snapshot(&session, &captured.draft, captured.revision, &operation)
                .err(),
            Some(ServiceError::Storage(StorageError::CommitUncertain))
        );
        let uncertain = std::fs::read(&path).unwrap();
        assert_eq!(uncertain != before, published);
        assert!(!service.can_write(&session).unwrap());
        assert_eq!(
            service
                .patch_group_draft(
                    &session,
                    &captured.draft,
                    GroupMetadataPatch {
                        parent: FieldUpdate::Clear,
                        ..Default::default()
                    },
                )
                .err(),
            Some(ServiceError::Storage(StorageError::CommitUncertain))
        );
        service.reconcile_uncertain(&session).unwrap();
        let reconciled = service
            .metadata_draft(&session, &captured.draft)
            .unwrap()
            .value;
        assert_eq!(reconciled.identity, captured);
        assert_eq!(reconciled.parent.as_ref(), Some(&parent));
        assert_eq!(reconciled.dirty, !published);
        let saved = service
            .save_draft_snapshot(&session, &captured.draft, captured.revision, &operation)
            .unwrap()
            .value;
        assert!(matches!(saved, DraftSaveOutcome::Saved { .. }));
        let confirmed = std::fs::read(&path).unwrap();
        if published {
            assert_eq!(confirmed, uncertain);
        }
        let history = service.group_history(&session, &group).unwrap().value;
        assert_eq!(history.len(), 2);
        let revision = history
            .iter()
            .find(|revision| revision.snapshot.group.name == "PUBLIC confirmed parent name")
            .unwrap();
        assert_eq!(revision.snapshot.group.parent.as_ref(), Some(&parent));
        assert_eq!(
            revision.snapshot.description.as_deref(),
            Some("PUBLIC confirmed parent description")
        );
        assert!(revision.snapshot.group.icon == icon);
        let later = service
            .patch_group_draft(
                &session,
                &captured.draft,
                GroupMetadataPatch {
                    name: FieldUpdate::Set("PUBLIC later parent input".into()),
                    parent: FieldUpdate::Clear,
                    ..Default::default()
                },
            )
            .unwrap()
            .value;
        assert!(later.revision > captured.revision);
        service.persist_drafts(&session).unwrap();
        service.lock(&session).unwrap();
        drop(service);
        let (mut service, session) = reopen(&profile, path.clone(), database, failures);
        assert_eq!(
            service.resume_draft(&session, &later.draft).unwrap().value,
            later
        );
        assert_eq!(
            service
                .save_draft_snapshot(&session, &captured.draft, captured.revision, &operation)
                .unwrap()
                .value,
            saved
        );
        assert_eq!(std::fs::read(&path).unwrap(), confirmed);
        let retained = service
            .metadata_draft(&session, &later.draft)
            .unwrap()
            .value;
        assert_eq!(retained.identity, later);
        assert_eq!(retained.parent, None);
        assert_eq!(retained.name, "PUBLIC later parent input");
        assert!(retained.dirty);
        assert_eq!(
            service.group_history(&session, &group).unwrap().value.len(),
            2
        );
        service
            .save_draft_snapshot(
                &session,
                &later.draft,
                later.revision,
                &crate::new_operation_id().unwrap(),
            )
            .unwrap();
        let groups = service.groups(&session).unwrap().value;
        let current = groups.iter().find(|row| row.id == group).unwrap();
        assert_eq!(current.parent, None);
        assert_eq!(current.name, "PUBLIC later parent input");
        assert_eq!(
            service.group_history(&session, &group).unwrap().value.len(),
            3
        );
    }
}
