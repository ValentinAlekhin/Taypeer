//! Real worker protocol coverage for the macOS editor, with public fixture credentials.
use super::*;
use serde::de::DeserializeOwned;
use taypeer_core::{EntryId, GroupId, IconRef, OperationId};
use taypeer_services::{
    AttributePatch, BinaryEdit, BinaryRequest, BinaryTarget, EditorView, EntryPatch, FieldUpdate,
    GroupForm,
};

fn command<T: DeserializeOwned>(client: &Client, mut command: Command) -> T {
    let mut value = client.request(&command, false).unwrap();
    command.erase_input();
    let decoded = T::deserialize(&value).unwrap();
    crate::erase_view(&mut value);
    decoded
}
fn group_form(id: Option<GroupId>, name: &str) -> GroupForm {
    GroupForm {
        id,
        parent: None,
        name: name.into(),
        description: Some("PUBLIC group description".into()),
        icon: IconRef::Lucide("folder".to_owned().try_into().unwrap()),
    }
}
#[test]
fn editor_commands_preserve_masked_fields_and_confirm_all_tabs_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC-ui.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    let info: taypeer_services::DatabaseInfo = command(&client, Command::DatabaseInfo);
    assert!(info.writable && info.managing);
    assert_eq!(
        info.description.as_deref(),
        Some("PUBLIC initial description")
    );
    command::<()>(
        &client,
        Command::SetDatabaseInfo {
            name: "PUBLIC renamed database".into(),
            description: Some("PUBLIC exact description\n второй ряд  ".into()),
        },
    );
    let operation = OperationId::new("PUBLIC group form");
    let group: GroupId = command(
        &client,
        Command::SaveGroup {
            form: group_form(None, "PUBLIC group"),
            operation: operation.clone(),
        },
    );
    let retry: GroupId = command(
        &client,
        Command::SaveGroup {
            form: group_form(None, "PUBLIC group"),
            operation,
        },
    );
    assert_eq!(group, retry);
    assert!(
        client
            .request(
                &Command::SaveGroup {
                    form: group_form(Some(group.clone()), ""),
                    operation: OperationId::new("PUBLIC invalid group")
                },
                false
            )
            .is_err()
    );
    let groups: Vec<taypeer_services::GroupInfo> = command(&client, Command::GroupInfo);
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].description.as_deref(),
        Some("PUBLIC group description")
    );
    command::<()>(&client, Command::BeginCreate(group.clone()));
    command::<()>(
        &client,
        Command::PatchDraft(EntryPatch {
            title: FieldUpdate::Set("PUBLIC entry".into()),
            password: FieldUpdate::Set("PUBLIC_PASSWORD_DO_NOT_PROJECT".into()),
            expires_at: FieldUpdate::Set(1_900_000_000_123),
            ..Default::default()
        }),
    );
    command::<()>(
        &client,
        Command::PatchAttribute {
            patch: AttributePatch {
                id: None,
                name: "PUBLIC protected".into(),
                value: FieldUpdate::Set("PUBLIC_ATTRIBUTE_DO_NOT_PROJECT".into()),
                protected: true,
            },
            remove: false,
        },
    );
    let draft: EditorView = command(&client, Command::EditorView);
    assert!(draft.has_password && draft.fields.password.is_none());
    assert!(draft.fields.attributes[0].value.is_empty());
    assert!(
        client
            .request(
                &Command::PatchAttribute {
                    patch: AttributePatch {
                        id: None,
                        name: "PUBLIC protected".into(),
                        value: FieldUpdate::Set("PUBLIC duplicate rejected".into()),
                        protected: false,
                    },
                    remove: false,
                },
                false
            )
            .is_err()
    );
    let after_invalid: EditorView = command(&client, Command::EditorView);
    assert_eq!(after_invalid.fields.attributes.len(), 1);
    let attribute = draft.fields.attributes[0].id.clone().unwrap();
    command::<()>(
        &client,
        Command::PatchAttribute {
            patch: AttributePatch {
                id: Some(attribute.clone()),
                name: "PUBLIC renamed attribute".into(),
                value: FieldUpdate::Keep,
                protected: true,
            },
            remove: false,
        },
    );
    command::<()>(
        &client,
        Command::PatchDraft(EntryPatch {
            notes: FieldUpdate::Set("  PUBLIC notes\nТочно é  ".into()),
            ..Default::default()
        }),
    );
    let file = directory.path().join("PUBLIC.txt");
    let icon_path = directory.path().join("PUBLIC.svg");
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#abcdef"/></svg>"##;
    std::fs::write(&icon_path, svg).unwrap();
    command::<()>(
        &client,
        Command::EditBinary {
            request: BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Icon(taypeer_services::IconInput::File(icon_path)),
                review: None,
            },
            operation: OperationId::new("PUBLIC local icon"),
        },
    );
    let preview: Option<taypeer_services::IconPreview> =
        command(&client, Command::IconPreview(BinaryTarget::Draft));
    let preview = preview.unwrap();
    assert!(matches!(
        preview.encoding,
        taypeer_services::IconEncoding::Svg
    ));
    assert_eq!(preview.bytes.as_slice(), svg);
    std::fs::write(&file, b"PUBLIC attachment bytes").unwrap();
    command::<()>(
        &client,
        Command::EditBinary {
            request: BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Attachment(taypeer_services::AttachmentEdit::Add {
                    path: file,
                    name: None,
                }),
                review: None,
            },
            operation: OperationId::new("PUBLIC add file"),
        },
    );
    command::<()>(
        &client,
        Command::EditBinary {
            request: BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Appearance {
                    foreground: FieldUpdate::Set(taypeer_core::Color([1, 2, 3, 255])),
                    background: FieldUpdate::Keep,
                },
                review: None,
            },
            operation: OperationId::new("PUBLIC color"),
        },
    );
    command::<()>(
        &client,
        Command::DraftExpiry(Some("PUBLIC unfinished expiration".into())),
    );
    assert!(client.request(&Command::SaveDraft, false).is_err());
    let draft: EditorView = command(&client, Command::EditorView);
    assert_eq!(
        draft.expiry_input.as_deref(),
        Some("PUBLIC unfinished expiration")
    );
    assert_eq!(draft.attachments.len(), 1);
    assert_eq!(
        draft.fields.notes.as_deref(),
        Some("  PUBLIC notes\nТочно é  ")
    );
    command::<()>(&client, Command::DraftExpiry(None));
    let entry: EntryId = command(&client, Command::SaveDraft);
    let history: Vec<taypeer_services::RevisionSummary> =
        command(&client, Command::History(entry.clone()));
    assert_eq!(history.len(), 1);
    assert_eq!(
        command::<String>(&client, Command::RevealPassword(entry.clone())),
        "PUBLIC_PASSWORD_DO_NOT_PROJECT"
    );
    assert_eq!(
        command::<String>(
            &client,
            Command::RevealAttribute {
                entry: entry.clone(),
                attribute
            }
        ),
        "PUBLIC_ATTRIBUTE_DO_NOT_PROJECT"
    );
    let view: taypeer_services::EntryView = command(&client, Command::Entry(entry.clone()));
    assert_eq!(view.expires_at, Some(1_900_000_000_123));
    assert_eq!(
        view.appearance.foreground,
        Some(taypeer_core::Color([1, 2, 3, 255]))
    );
    let target = BinaryTarget::Revision {
        entry: entry.clone(),
        revision: history[0].id.clone(),
    };
    let binary: taypeer_services::BinaryView =
        command(&client, Command::BinaryView(target.clone()));
    assert_eq!(binary.attachments[0].contents[0].bytes, Some(23));
    let output = directory.path().join("PUBLIC export.txt");
    command::<()>(
        &client,
        Command::ExportBinary {
            target,
            blob: binary.attachments[0].contents[0].id.clone(),
            path: output.clone(),
            overwrite: false,
        },
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"PUBLIC attachment bytes");
    command::<()>(&client, Command::BeginEdit(entry.clone()));
    command::<()>(
        &client,
        Command::PatchDraft(EntryPatch {
            title: FieldUpdate::Set("PUBLIC second title".into()),
            ..Default::default()
        }),
    );
    command::<EntryId>(&client, Command::SaveDraft);
    let history: Vec<taypeer_services::RevisionSummary> =
        command(&client, Command::History(entry.clone()));
    assert_eq!(history.len(), 2);
    client.control.invalidate(LockReason::Manual);
    client.control.wait_closed().unwrap();
    let reopened = open(&sessions, &path, false);
    let info: taypeer_services::DatabaseInfo = command(&reopened, Command::DatabaseInfo);
    assert_eq!(info.name, "PUBLIC renamed database");
    assert_eq!(
        info.description.as_deref(),
        Some("PUBLIC exact description\n второй ряд  ")
    );
    assert_eq!(
        command::<String>(&reopened, Command::RevealPassword(entry)),
        "PUBLIC_PASSWORD_DO_NOT_PROJECT"
    );
    reopened.control.invalidate(LockReason::Manual);
    reopened.control.wait_closed().unwrap();
}

#[test]
fn masked_metadata_does_not_grant_a_read_only_copy_write_or_management_access() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC-read-only.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let writer = open(&sessions, &path, true);
    writer.control.invalidate(LockReason::Manual);
    writer.control.wait_closed().unwrap();
    let before = std::fs::read(&path).unwrap();
    let reader = open_mode(&sessions, &path, false, true);
    let info: taypeer_services::DatabaseInfo = command(&reader, Command::DatabaseInfo);
    assert!(!info.writable && !info.managing);
    assert!(
        reader
            .request(
                &Command::SetDatabaseInfo {
                    name: "PUBLIC forbidden".into(),
                    description: None
                },
                false
            )
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    reader.control.invalidate(LockReason::Manual);
    reader.control.wait_closed().unwrap();
}

#[test]
fn value_presence_is_available_without_revealing_current_or_historical_secrets() {
    let directory = tempfile::tempdir().unwrap();
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(
        &sessions,
        &directory.path().join("PUBLIC-presence.taypeer"),
        true,
    );
    let group: GroupId = command(
        &client,
        Command::SaveGroup {
            form: group_form(None, "PUBLIC presence"),
            operation: OperationId::new("PUBLIC presence group"),
        },
    );
    command::<()>(&client, Command::BeginCreate(group));
    command::<()>(
        &client,
        Command::PatchDraft(EntryPatch {
            title: FieldUpdate::Set("PUBLIC presence".into()),
            ..Default::default()
        }),
    );
    for (name, value, protected) in [
        ("PUBLIC empty secret", "", true),
        ("PUBLIC filled secret", "PUBLIC é 😀", true),
        ("PUBLIC empty ordinary", "", false),
        ("PUBLIC whitespace", "  ", false),
    ] {
        command::<()>(
            &client,
            Command::PatchAttribute {
                patch: AttributePatch {
                    id: None,
                    name: name.into(),
                    value: FieldUpdate::Set(value.into()),
                    protected,
                },
                remove: false,
            },
        );
    }
    command::<()>(
        &client,
        Command::EditBinary {
            request: BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Appearance {
                    foreground: FieldUpdate::Set(taypeer_core::Color([12, 34, 56, 78])),
                    background: FieldUpdate::Clear,
                },
                review: None,
            },
            operation: OperationId::new("PUBLIC translucent color"),
        },
    );
    let entry: EntryId = command(&client, Command::SaveDraft);
    let current: taypeer_services::EntryView = command(&client, Command::Entry(entry.clone()));
    let history: Vec<taypeer_services::RevisionSummary> =
        command(&client, Command::History(entry.clone()));
    let old: taypeer_services::EntryView = command(
        &client,
        Command::Revision {
            entry,
            revision: history[0].id.clone(),
        },
    );
    for view in [current, old] {
        assert!(!view.has_password);
        assert_eq!(
            view.appearance.foreground,
            Some(taypeer_core::Color([12, 34, 56, 78]))
        );
        for attribute in &view.attributes {
            assert_eq!(attribute.has_value, !attribute.name.contains("empty"));
            if attribute.protected {
                assert!(attribute.value.is_none());
            }
        }
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("PUBLIC é 😀"));
    }
}
