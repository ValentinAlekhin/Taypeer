//! Real saved-secret dispatch preserves legacy reveals and refuses ineligible/revoked access.
use super::*;
use serde::de::DeserializeOwned;
use taypeer_core::{AttributeId, EntryId, GroupId};
use taypeer_services::{
    AttributePatch, DraftTarget, EditorView, EntryPatch, FieldUpdate, RevisionSummary,
};

fn request<T: DeserializeOwned>(client: &Client, mut command: Command) -> T {
    let mut value = client.request(&command, false).unwrap();
    command.erase_input();
    let result = T::deserialize(&value).unwrap();
    crate::erase_view(&mut value);
    result
}
fn reveal(
    entry: &EntryId,
    revision: Option<taypeer_core::RevisionId>,
    attribute: Option<AttributeId>,
) -> Command {
    Command::RevealSavedForAccess {
        entry: entry.clone(),
        revision,
        attribute,
    }
}
#[test]
fn saved_access_reveal_is_atomic_eligible_read_only_and_revoked_with_its_worker() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC-access.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    let group: serde_json::Value = request(
        &client,
        Command::CreateGroup {
            operation: taypeer_services::new_operation_id().unwrap(),
            name: "PUBLIC access group".into(),
            parent: None,
        },
    );
    request::<()>(
        &client,
        Command::BeginCreate(serde_json::from_value::<GroupId>(group["id"].clone()).unwrap()),
    );
    request::<()>(
        &client,
        Command::PatchDraft(EntryPatch {
            title: FieldUpdate::Set("PUBLIC access entry".into()),
            password: FieldUpdate::Set("PUBLIC access password".into()),
            ..Default::default()
        }),
    );
    for (name, protected) in [("PUBLIC protected", true), ("PUBLIC ordinary", false)] {
        request::<()>(
            &client,
            Command::PatchAttribute {
                patch: AttributePatch {
                    id: None,
                    name: name.into(),
                    value: FieldUpdate::Set(format!("{name} value")),
                    protected,
                },
                remove: false,
            },
        );
    }
    let draft: EditorView = request(&client, Command::EditorView);
    let DraftTarget::NewEntry { entry: unsaved, .. } = draft.identity.target else {
        panic!("PUBLIC new-entry fixture missing");
    };
    assert!(
        client
            .request(&reveal(&unsaved, None, None), false)
            .is_err()
    );
    let protected = draft
        .fields
        .attributes
        .iter()
        .find(|value| value.protected)
        .unwrap()
        .id
        .clone()
        .unwrap();
    let ordinary = draft
        .fields
        .attributes
        .iter()
        .find(|value| !value.protected)
        .unwrap()
        .id
        .clone()
        .unwrap();
    let entry: EntryId = request(
        &client,
        Command::SaveDraft {
            operation: taypeer_services::new_operation_id().unwrap(),
        },
    );
    let history: Vec<RevisionSummary> = request(&client, Command::History(entry.clone()));
    assert_eq!(history.len(), 1);
    let saved_bytes = std::fs::read(&path).unwrap();
    for revision in [None, Some(history[0].id.clone())] {
        assert_eq!(
            request::<String>(&client, reveal(&entry, revision.clone(), None)),
            "PUBLIC access password"
        );
        assert_eq!(
            request::<String>(
                &client,
                reveal(&entry, revision.clone(), Some(protected.clone()))
            ),
            "PUBLIC protected value"
        );
        assert!(matches!(
            client.request(&reveal(&entry, revision, Some(ordinary.clone())), false),
            Err(RuntimeError::Service(
                taypeer_services::ServiceError::InvalidInput
            ))
        ));
    }
    assert_eq!(
        request::<String>(
            &client,
            Command::RevealAttribute {
                entry: entry.clone(),
                attribute: ordinary
            }
        ),
        "PUBLIC ordinary value"
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    assert_eq!(
        request::<Vec<RevisionSummary>>(&client, Command::History(entry.clone())).len(),
        1
    );
    client.control.invalidate(LockReason::Manual);
    assert!(matches!(
        client.request(&reveal(&entry, None, Some(protected)), false),
        Err(RuntimeError::SessionClosed(LockReason::Manual) | RuntimeError::Closed)
    ));
    client.control.wait_closed().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
}
