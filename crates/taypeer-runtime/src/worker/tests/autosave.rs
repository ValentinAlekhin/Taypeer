//! Observable autosave behavior through the real isolated worker on PUBLIC data.
use super::*;
use taypeer_services::{DraftSaveOutcome, EditorView, EntryPatch, FieldUpdate, GroupSummary};

fn view(client: &Client) -> EditorView {
    serde_json::from_value(client.request(&Command::EditorView, false).unwrap()).unwrap()
}

fn save(
    client: &Client,
    view: &EditorView,
    operation: &taypeer_core::OperationId,
) -> DraftSaveOutcome {
    serde_json::from_value(
        client
            .request(
                &Command::SaveDraftSnapshot {
                    draft: view.identity.draft.clone(),
                    revision: view.identity.revision,
                    operation: operation.clone(),
                },
                false,
            )
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn snapshot_acknowledges_exact_input_and_automatically_resumes_after_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC autosave.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    let group: GroupSummary = serde_json::from_value(
        client
            .request(
                &Command::CreateGroup {
                    name: "PUBLIC group".into(),
                    parent: None,
                    operation: taypeer_services::new_operation_id().unwrap(),
                },
                false,
            )
            .unwrap(),
    )
    .unwrap();
    client
        .request(&Command::BeginCreate(group.id), false)
        .unwrap();
    client
        .request(
            &Command::PatchDraft(EntryPatch {
                title: FieldUpdate::Set("PUBLIC autosaved entry".into()),
                password: FieldUpdate::Set("PUBLIC password".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    let captured = view(&client);
    assert!(captured.fields.password.is_none());
    let operation = taypeer_services::new_operation_id().unwrap();
    assert!(matches!(
        save(&client, &captured, &operation),
        DraftSaveOutcome::Saved { .. }
    ));
    let confirmed = view(&client);
    let entry = confirmed.entry.unwrap();
    assert!(!confirmed.dirty);
    assert_eq!(confirmed.identity.draft, captured.identity.draft);
    client
        .request(
            &Command::PatchDraft(EntryPatch {
                username: FieldUpdate::Set("PUBLIC later input".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    assert_eq!(
        save(&client, &captured, &operation),
        DraftSaveOutcome::Saved {
            identity: captured.identity.clone(),
            operation,
        }
    );
    let newer = view(&client);
    assert!(newer.dirty);
    assert_eq!(newer.fields.username.as_deref(), Some("PUBLIC later input"));
    assert!(newer.identity.revision > captured.identity.revision);
    client.control.invalidate(LockReason::SystemLocked);
    client.control.wait_closed().unwrap();
    let reopened = open(&sessions, &path, false);
    // Opening the object continues its local form without RestoreDraft or a question.
    reopened
        .request(&Command::BeginEdit(entry.clone()), false)
        .unwrap();
    let restored = view(&reopened);
    assert_eq!(restored.identity.draft, captured.identity.draft);
    assert_eq!(
        restored.fields.username.as_deref(),
        Some("PUBLIC later input")
    );
    assert!(restored.dirty);
    save(
        &reopened,
        &restored,
        &taypeer_services::new_operation_id().unwrap(),
    );
    reopened.control.invalidate(LockReason::Manual);
    reopened.control.wait_closed().unwrap();
    let final_reader = open(&sessions, &path, false);
    let persisted = final_reader.request(&Command::Entry(entry), false).unwrap();
    assert_eq!(persisted["username"], "PUBLIC later input");
    final_reader.control.invalidate(LockReason::Manual);
    final_reader.control.wait_closed().unwrap();
}

#[test]
fn empty_ungrouped_form_creates_nothing_and_incomplete_input_is_local_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC ungrouped.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    client
        .request(&Command::BeginCreateUngrouped, false)
        .unwrap();
    let empty = view(&client);
    assert!(matches!(
        save(
            &client,
            &empty,
            &taypeer_services::new_operation_id().unwrap()
        ),
        DraftSaveOutcome::Unchanged { .. }
    ));
    client
        .request(
            &Command::PatchDraft(EntryPatch {
                username: FieldUpdate::Set("PUBLIC unfinished".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    let unfinished = view(&client);
    assert!(matches!(
        save(
            &client,
            &unfinished,
            &taypeer_services::new_operation_id().unwrap()
        ),
        DraftSaveOutcome::LocalDraftSaved { .. }
    ));
    assert_eq!(
        client
            .request(
                &Command::Entries {
                    group: None,
                    query: String::new()
                },
                false
            )
            .unwrap(),
        serde_json::json!([])
    );
    client.control.invalidate(LockReason::Background);
    client.control.wait_closed().unwrap();
    let reopened = open(&sessions, &path, false);
    reopened
        .request(&Command::ResumeDraft(unfinished.identity.draft), false)
        .unwrap();
    let resumed = view(&reopened);
    assert_eq!(
        resumed.fields.username.as_deref(),
        Some("PUBLIC unfinished")
    );
    assert!(resumed.group.is_none());
    reopened
        .request(
            &Command::PatchDraft(EntryPatch {
                title: FieldUpdate::Set("PUBLIC completed".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    save(
        &reopened,
        &view(&reopened),
        &taypeer_services::new_operation_id().unwrap(),
    );
    let entries = reopened
        .request(
            &Command::Entries {
                group: None,
                query: String::new(),
            },
            false,
        )
        .unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert!(entries[0]["group_id"].is_null());
    reopened.control.invalidate(LockReason::Manual);
    reopened.control.wait_closed().unwrap();
}
