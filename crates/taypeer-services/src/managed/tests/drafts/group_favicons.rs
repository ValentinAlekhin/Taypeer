//! Actual encrypted managed persistence distinguishes absent publication from lost ACK.
use super::*;
use crate::{FaviconPlanAction, GroupFaviconRequest};
use std::io::{self, Read};

const IMAGE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#14dc28"/></svg>"##;
struct NeverRead;
impl Read for NeverRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("PUBLIC confirmed row must not touch provider")
    }
}

#[test]
fn managed_group_favicon_unknown_publication_reconciles_exact_receipt_before_new_input() {
    for published in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let profile = Profile::new(110);
        let path = directory.path().join("PUBLIC favicon uncertain.taypeer");
        let (mut seed, session) = create(&profile, path.clone());
        seed.lock(&session).unwrap();
        drop(seed);
        let faults = Arc::new(Failures::default());
        let (mut service, session) = reopen(
            &profile,
            path.clone(),
            session.database,
            Arc::clone(&faults),
        );
        begin_entry(&mut service, &session);
        let group = service.editor_view(&session).unwrap().group;
        service
            .patch_draft(
                &session,
                EntryPatch {
                    title: FieldUpdate::Set("PUBLIC saved batch row".into()),
                    url: FieldUpdate::Set("https://public.invalid/batch".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let entry = service
            .save_draft(&session, &crate::new_operation_id().unwrap())
            .unwrap()
            .value;
        let plan = service
            .prepare_group_favicons(
                &session,
                GroupFaviconRequest {
                    group,
                    recursive: false,
                    replace: false,
                    operation: crate::new_operation_id().unwrap(),
                },
            )
            .unwrap()
            .value;
        let request = match &plan.rows[0].action {
            FaviconPlanAction::Acquire(request) => request.clone(),
            _ => panic!("PUBLIC row must have a URL"),
        };
        let before = std::fs::read(&path).unwrap();
        if published {
            faults.uncertain_commit.store(true, Ordering::SeqCst);
        } else {
            faults.uncertain_before_commit.store(true, Ordering::SeqCst);
        }
        assert_eq!(
            service
                .import_entry_favicon_stream(&session, &request, IMAGE, IMAGE.len() as u64)
                .err(),
            Some(ServiceError::Storage(StorageError::CommitUncertain))
        );
        assert!(!service.can_write(&session).unwrap());
        assert_eq!(std::fs::read(&path).unwrap() == before, !published);
        faults.deny_snapshot.store(true, Ordering::SeqCst);
        assert_eq!(
            service.favicon_entry_receipt(&session, &request).err(),
            Some(ServiceError::Storage(StorageError::Io))
        );
        assert!(!service.can_write(&session).unwrap());
        assert_eq!(
            service
                .favicon_entry_receipt(&session, &request)
                .unwrap()
                .value,
            published
        );
        if published {
            service
                .import_entry_favicon_stream(&session, &request, NeverRead, u64::MAX)
                .unwrap();
        } else {
            service
                .import_entry_favicon_stream(&session, &request, IMAGE, IMAGE.len() as u64)
                .unwrap();
        }
        assert!(
            service
                .favicon_entry_receipt(&session, &request)
                .unwrap()
                .value
        );
        assert!(service.can_write(&session).unwrap());
        assert_eq!(service.history(&session, &entry).unwrap().value.len(), 2);
        let acknowledged = std::fs::read(&path).unwrap();
        service
            .import_entry_favicon_stream(&session, &request, NeverRead, 0)
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), acknowledged);
        service.lock(&session).unwrap();
    }
}
