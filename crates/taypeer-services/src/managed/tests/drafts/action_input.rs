//! PUBLIC mutations distinguish rejected candidates from failed durable publication.
use super::*;
use taypeer_core::{OperationId, RevisionId};
use taypeer_document::{RecoveryMode, RecoveryRequest};

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    service: DatabaseService,
    session: SessionToken,
    failures: Arc<Failures>,
    source: GroupId,
    destination: GroupId,
    entry: EntryId,
    revision: RevisionId,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC action outcomes.taypeer");
        let profile = Profile::new(54);
        let (mut seed, session) = create(&profile, path.clone());
        let database = session.database.clone();
        seed.lock(&session).unwrap();
        drop(seed);
        let failures = Arc::new(Failures::default());
        let (mut service, session) =
            reopen(&profile, path.clone(), database, Arc::clone(&failures));
        let mut group = |name: &str| {
            service
                .create_group(
                    &session,
                    name.into(),
                    None,
                    &crate::new_operation_id().unwrap(),
                )
                .unwrap()
                .value
                .id
        };
        let source = group("PUBLIC source group");
        let destination = group("PUBLIC destination group");
        let entry = service
            .create_entry(
                &session,
                source.clone(),
                EntryPatch {
                    title: FieldUpdate::Set("PUBLIC saved title".into()),
                    ..Default::default()
                },
                &crate::new_operation_id().unwrap(),
            )
            .unwrap()
            .value;
        let revision = service.history(&session, &entry).unwrap().value[0]
            .id
            .clone();
        service
            .update_entry(
                &session,
                &entry,
                EntryPatch {
                    title: FieldUpdate::Set("PUBLIC current title".into()),
                    ..Default::default()
                },
                &crate::new_operation_id().unwrap(),
            )
            .unwrap();
        Self {
            _directory: directory,
            path,
            service,
            session,
            failures,
            source,
            destination,
            entry,
            revision,
        }
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap()
    }
    fn purge(&mut self, target: ObjectId) {
        for action in [LifecycleAction::Trash, LifecycleAction::Purge] {
            let review = self
                .service
                .prepare_lifecycle(&self.session, action, target.clone(), None)
                .unwrap()
                .value;
            self.service
                .confirm_lifecycle(&self.session, &review, &crate::new_operation_id().unwrap())
                .unwrap();
        }
    }
    fn history(&self) -> Vec<RevisionId> {
        self.service
            .history(&self.session, &self.entry)
            .unwrap()
            .value
            .into_iter()
            .map(|row| row.id)
            .collect()
    }
}

#[derive(Clone, Copy)]
enum Action {
    Clone,
    Restore,
}
impl Action {
    fn confirm(
        self,
        fixture: &mut Fixture,
        destination: GroupId,
        operation: &OperationId,
    ) -> Result<EntryId, ServiceError> {
        let service = &mut fixture.service;
        match self {
            Self::Clone => service.clone_entry(
                &fixture.session,
                &fixture.entry,
                destination,
                None,
                operation,
            ),
            Self::Restore => service.restore_revision(
                &fixture.session,
                &fixture.entry,
                &fixture.revision,
                destination,
                operation,
            ),
        }
        .map(|result| result.value)
    }
    fn assert_one_effect(self, fixture: &Fixture, result: &EntryId) {
        match self {
            Self::Clone => {
                assert_ne!(result, &fixture.entry);
                assert_eq!(fixture.history().len(), 2);
                assert_eq!(
                    fixture
                        .service
                        .history(&fixture.session, result)
                        .unwrap()
                        .value
                        .len(),
                    1
                );
            }
            Self::Restore => {
                assert_eq!(result, &fixture.entry);
                assert_eq!(fixture.history().len(), 3);
                assert_eq!(
                    fixture
                        .service
                        .view_entry(&fixture.session, result)
                        .unwrap()
                        .value
                        .title,
                    "PUBLIC saved title"
                );
            }
        }
    }
}

#[test]
fn deleted_destinations_reject_before_publication_and_corrected_intents_apply_once() {
    for action in [Action::Clone, Action::Restore] {
        let mut fixture = Fixture::new();
        let deleted = fixture.destination.clone();
        fixture.purge(ObjectId::Group(deleted.clone()));
        let before = fixture.bytes();
        let history = fixture.history();
        assert_eq!(
            action.confirm(&mut fixture, deleted, &crate::new_operation_id().unwrap()),
            Err(ServiceError::InvalidInput)
        );
        assert_eq!(fixture.bytes(), before);
        assert_eq!(fixture.history(), history);
        let destination = fixture.source.clone();
        let corrected = crate::new_operation_id().unwrap();
        let result = action
            .confirm(&mut fixture, destination.clone(), &corrected)
            .unwrap();
        action.assert_one_effect(&fixture, &result);
        let saved = fixture.bytes();
        assert_eq!(
            action
                .confirm(&mut fixture, destination, &corrected)
                .unwrap(),
            result
        );
        assert_eq!(fixture.bytes(), saved);
        action.assert_one_effect(&fixture, &result);
    }
}

#[derive(Clone, Copy)]
enum PublicationFailure {
    Io,
    BeforeCommit,
    AfterCommit,
}
impl PublicationFailure {
    fn inject(self, failures: &Failures) {
        match self {
            Self::Io => failures.deny_snapshot.store(true, Ordering::SeqCst),
            Self::BeforeCommit => failures
                .uncertain_before_commit
                .store(true, Ordering::SeqCst),
            Self::AfterCommit => failures.uncertain_commit.store(true, Ordering::SeqCst),
        }
    }
    fn error(self) -> ServiceError {
        ServiceError::Storage(match self {
            Self::Io => StorageError::Io,
            Self::BeforeCommit | Self::AfterCommit => StorageError::CommitUncertain,
        })
    }
}

#[test]
fn publication_errors_keep_their_type_and_exact_receipts_survive_source_and_destination_purge() {
    for action in [Action::Clone, Action::Restore] {
        for failure in [
            PublicationFailure::Io,
            PublicationFailure::BeforeCommit,
            PublicationFailure::AfterCommit,
        ] {
            let mut fixture = Fixture::new();
            let destination = fixture.destination.clone();
            let operation = crate::new_operation_id().unwrap();
            let before = fixture.bytes();
            let history = fixture.history();
            failure.inject(&fixture.failures);
            assert_eq!(
                action.confirm(&mut fixture, destination.clone(), &operation),
                Err(failure.error())
            );
            assert_eq!(
                fixture.bytes() != before,
                matches!(failure, PublicationFailure::AfterCommit)
            );
            assert_eq!(fixture.history(), history);
            fixture
                .service
                .reconcile_uncertain(&fixture.session)
                .unwrap();
            let result = action
                .confirm(&mut fixture, destination.clone(), &operation)
                .unwrap();
            action.assert_one_effect(&fixture, &result);
            let saved = fixture.bytes();
            assert_eq!(
                action
                    .confirm(&mut fixture, destination.clone(), &operation)
                    .unwrap(),
                result
            );
            assert_eq!(fixture.bytes(), saved);
            fixture.purge(ObjectId::Group(fixture.source.clone()));
            fixture.purge(ObjectId::Group(destination.clone()));
            let purged = fixture.bytes();
            assert_eq!(
                action
                    .confirm(&mut fixture, destination, &operation)
                    .unwrap(),
                result
            );
            assert_eq!(fixture.bytes(), purged);
        }
    }
}

#[test]
fn closed_entry_source_requires_a_group_before_publication_and_replays_after_purge() {
    let mut fixture = Fixture::new();
    let mut late = fixture
        .service
        .checked(&fixture.session)
        .unwrap()
        .document()
        .fork();
    let mut draft = late.begin_edit_entry(&fixture.entry).unwrap();
    draft.fields_mut().title = "PUBLIC late closed title".into();
    late.save_entry(draft, 99).unwrap();
    fixture.purge(ObjectId::Entry(fixture.entry.clone()));
    fixture
        .service
        .checked_mut(&fixture.session)
        .unwrap()
        .change(|doc| Ok(doc.merge(&late)?))
        .unwrap();
    let sources = fixture
        .service
        .pending_sources(&fixture.session)
        .unwrap()
        .value;
    let [source] = sources.as_slice() else {
        panic!("one PUBLIC late source expected")
    };
    let mut request = RecoveryRequest {
        source: source.id.clone(),
        mode: RecoveryMode::Clone,
        destination: None,
        name: None,
        fields: None,
        icon: None,
    };
    let before = fixture.bytes();
    assert_eq!(
        fixture
            .service
            .recover_source(
                &fixture.session,
                &request,
                &crate::new_operation_id().unwrap()
            )
            .unwrap_err(),
        ServiceError::InvalidInput
    );
    assert_eq!(fixture.bytes(), before);
    assert_eq!(
        fixture
            .service
            .pending_sources(&fixture.session)
            .unwrap()
            .value
            .len(),
        1
    );
    assert!(
        fixture
            .service
            .entries(&fixture.session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    request.destination = Some(fixture.destination.clone());
    let operation = crate::new_operation_id().unwrap();
    let result = fixture
        .service
        .recover_source(&fixture.session, &request, &operation)
        .unwrap()
        .value;
    let [ObjectId::Entry(created)] = result.as_slice() else {
        panic!("one PUBLIC extracted entry expected")
    };
    assert_ne!(created, &fixture.entry);
    assert_eq!(
        fixture
            .service
            .history(&fixture.session, created)
            .unwrap()
            .value
            .len(),
        1
    );
    assert!(
        fixture
            .service
            .pending_sources(&fixture.session)
            .unwrap()
            .value
            .is_empty()
    );
    fixture.purge(ObjectId::Group(fixture.destination.clone()));
    let purged = fixture.bytes();
    assert_eq!(
        fixture
            .service
            .recover_source(&fixture.session, &request, &operation)
            .unwrap()
            .value,
        result
    );
    assert_eq!(fixture.bytes(), purged);
}
