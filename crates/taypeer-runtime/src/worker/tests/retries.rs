//! Failures are injected only in the synthetic child's persistence adapter.
use super::*;
use taypeer_core::{EntryId, OperationId};
use taypeer_services::{EntryPatch, FieldUpdate, ServiceError, StorageError};
fn save(id: &OperationId) -> Command {
    Command::SaveDraft {
        operation: id.clone(),
    }
}
fn stop(client: &Client) {
    client.control.invalidate(LockReason::Manual);
    client.control.wait_closed().unwrap();
}
fn request<T: serde::de::DeserializeOwned>(client: &Client, command: &Command) -> T {
    serde_json::from_value(client.request(command, false).unwrap()).unwrap()
}
fn mark(path: &Path, suffix: &str) {
    std::fs::write(path.with_extension(suffix), b"PUBLIC fault").unwrap();
}
fn unmark(path: &Path, suffix: &str) {
    std::fs::remove_file(path.with_extension(suffix)).unwrap();
}
#[test]
fn draft_failures_retry_once_and_reconcile_after_restart() {
    for failure in ["fail-before", "fail-after", "fail-cleanup"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC.taypeer");
        let sessions = SessionController::new(SessionPolicy::default());
        let client = open(&sessions, &path, true);
        edit(&client);
        let id = taypeer_services::new_operation_id().unwrap();
        let before = std::fs::read(&path).unwrap();
        mark(&path, failure);
        assert!(client.request(&save(&id), false).is_err());
        let failed = std::fs::read(&path).unwrap();
        if failure == "fail-before" {
            assert_eq!(failed, before);
        } else {
            assert_ne!(failed, before);
        }
        if failure == "fail-after" {
            assert_eq!(
                client.request(
                    &Command::PatchDraft(EntryPatch {
                        title: FieldUpdate::Set("PUBLIC forbidden".into()),
                        ..Default::default()
                    }),
                    false
                ),
                Err(RuntimeError::Service(ServiceError::Storage(
                    StorageError::CommitUncertain
                )))
            );
        }
        unmark(&path, failure);
        stop(&client);
        let reopened = open(&sessions, &path, false);
        let status: Value = request(&reopened, &Command::DraftStatus);
        if failure == "fail-before" {
            assert!(!status["pending"].is_null());
            reopened.request(&Command::RestoreDraft, false).unwrap();
        } else {
            assert!(status["pending"].is_null());
        }
        let entry: EntryId = request(&reopened, &save(&id));
        let committed = std::fs::read(&path).unwrap();
        let again: EntryId = request(&reopened, &save(&id));
        assert_eq!(again, entry);
        assert_eq!(std::fs::read(&path).unwrap(), committed);
        let history: Vec<taypeer_services::RevisionSummary> =
            request(&reopened, &Command::History(entry));
        assert_eq!(history.len(), 1);
        stop(&reopened);
    }
}
#[test]
fn cleanup_retry_keeps_a_different_editor_and_never_writes_history_twice() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    edit(&client);
    let operation = taypeer_services::new_operation_id().unwrap();
    mark(&path, "fail-cleanup");
    assert!(client.request(&save(&operation), false).is_err());
    let committed = std::fs::read(&path).unwrap();
    unmark(&path, "fail-cleanup");
    let entry: EntryId = request(&client, &save(&operation));
    client
        .request(&Command::BeginEdit(entry.clone()), false)
        .unwrap();
    client
        .request(
            &Command::PatchDraft(EntryPatch {
                title: FieldUpdate::Set("PUBLIC different editor".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    let again: EntryId = request(&client, &save(&operation));
    assert_eq!(again, entry);
    let status: Value = request(&client, &Command::DraftStatus);
    assert_eq!(status["active"], true);
    assert_eq!(status["dirty"], true);
    assert_eq!(std::fs::read(&path).unwrap(), committed);
    stop(&client);
}
#[test]
fn lock_during_confirmation_loses_response_but_not_committed_result() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    edit(&client);
    let operation = taypeer_services::new_operation_id().unwrap();
    mark(&path, "hold-after");
    let running = Arc::clone(&client);
    let command = save(&operation);
    let pending = std::thread::spawn(move || running.request(&command, false));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !path.with_extension("committed").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let committed = std::fs::read(&path).unwrap();
    stop(&client);
    assert!(pending.join().unwrap().is_err());
    unmark(&path, "hold-after");
    let reopened = open(&sessions, &path, false);
    let status: Value = request(&reopened, &Command::DraftStatus);
    assert!(status["pending"].is_null());
    let _: EntryId = request(&reopened, &save(&operation));
    assert_eq!(std::fs::read(&path).unwrap(), committed);
    stop(&reopened);
}

#[test]
fn all_six_commands_return_original_results_in_a_new_process() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    let operation = || taypeer_services::new_operation_id().unwrap();
    let create_group = Command::CreateGroup {
        name: "PUBLIC original group".into(),
        parent: None,
        operation: operation(),
    };
    let group: taypeer_services::GroupSummary = request(&client, &create_group);
    let rename = Command::RenameGroup {
        id: group.id.clone(),
        name: "PUBLIC renamed group".into(),
        operation: operation(),
    };
    let create = Command::CreateEntry {
        group: group.id.clone(),
        patch: EntryPatch {
            title: FieldUpdate::Set("PUBLIC original entry".into()),
            ..Default::default()
        },
        operation: operation(),
    };
    let entry: EntryId = request(&client, &create);
    let update = Command::UpdateEntry {
        id: entry.clone(),
        patch: EntryPatch {
            title: FieldUpdate::Set("PUBLIC updated entry".into()),
            ..Default::default()
        },
        operation: operation(),
    };
    let metadata = Command::SetDatabaseInfo {
        name: "PUBLIC changed database".into(),
        description: None,
        operation: operation(),
    };
    let mut commands = vec![create_group, rename, create, update, metadata];
    let mut results: Vec<Value> = commands
        .iter()
        .map(|command| request(&client, command))
        .collect();
    client
        .request(&Command::BeginEdit(entry.clone()), false)
        .unwrap();
    client
        .request(
            &Command::PatchDraft(EntryPatch {
                title: FieldUpdate::Set("PUBLIC latest entry".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    commands.push(save(&operation()));
    results.push(request(&client, commands.last().unwrap()));
    let bytes = std::fs::read(&path).unwrap();
    for (command, expected) in commands.iter().zip(&results) {
        assert_eq!(request::<Value>(&client, command), *expected);
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    stop(&client);
    let restarted = open(&sessions, &path, false);
    for (command, expected) in commands.iter().zip(&results) {
        assert_eq!(request::<Value>(&restarted, command), *expected);
    }
    let view: taypeer_services::EntryView = request(&restarted, &Command::Entry(entry.clone()));
    assert_eq!(view.title, "PUBLIC latest entry");
    let history: Vec<taypeer_services::RevisionSummary> =
        request(&restarted, &Command::History(entry));
    assert_eq!(history.len(), 3);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    stop(&restarted);
    let reader = open_mode(&sessions, &path, false, true);
    for command in &commands {
        assert_eq!(
            reader.request(command, false),
            Err(RuntimeError::Service(ServiceError::ReadOnly))
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    stop(&reader);
}

#[test]
fn editing_after_cleanup_failure_starts_a_new_revision_context() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    edit(&client);
    let first = taypeer_services::new_operation_id().unwrap();
    mark(&path, "fail-cleanup");
    assert!(client.request(&save(&first), false).is_err());
    unmark(&path, "fail-cleanup");
    client
        .request(
            &Command::PatchDraft(EntryPatch {
                title: FieldUpdate::Set("PUBLIC next intent".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    let _: EntryId = request(&client, &save(&first));
    let next = taypeer_services::new_operation_id().unwrap();
    let entry: EntryId = request(&client, &save(&next));
    stop(&client);
    let reopened = open(&sessions, &path, false);
    let view: taypeer_services::EntryView = request(&reopened, &Command::Entry(entry.clone()));
    assert_eq!(view.title, "PUBLIC next intent");
    let history: Vec<taypeer_services::RevisionSummary> =
        request(&reopened, &Command::History(entry));
    assert_eq!(history.len(), 2);
    stop(&reopened);
}

#[test]
fn an_unsaved_sidecar_remains_recoverable_when_another_command_claims_its_id() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    edit(&client);
    let operation = taypeer_services::new_operation_id().unwrap();
    mark(&path, "fail-before");
    assert!(client.request(&save(&operation), false).is_err());
    unmark(&path, "fail-before");
    client
        .request(
            &Command::PatchDraft(EntryPatch {
                title: FieldUpdate::Set("PUBLIC corrected form".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        client.request(&save(&operation), false),
        Err(RuntimeError::Service(ServiceError::OperationConflict))
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    client
        .request(
            &Command::CreateGroup {
                name: "PUBLIC distinct command".into(),
                parent: None,
                operation: operation.clone(),
            },
            false,
        )
        .unwrap();
    stop(&client);
    let reopened = open(&sessions, &path, false);
    let status: Value = request(&reopened, &Command::DraftStatus);
    assert!(!status["pending"].is_null());
    reopened.request(&Command::RestoreDraft, false).unwrap();
    let entry: EntryId = request(
        &reopened,
        &save(&taypeer_services::new_operation_id().unwrap()),
    );
    let view: taypeer_services::EntryView = request(&reopened, &Command::Entry(entry));
    assert_eq!(view.title, "PUBLIC corrected form");
    stop(&reopened);
}
