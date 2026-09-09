//! Public synthetic scenarios for the in-memory application boundary.

use taypeer_services::{
    DEMO_PASSWORD, DemoService, EditableAttribute, EditableEntry, EntryId, GroupId, ServiceError,
    SessionToken,
};

const PASSWORD: &str = "PUBLIC-UNSEARCHABLE-PASSWORD-Жук";
const PROTECTED: &str = "PUBLIC-UNSEARCHABLE-PROTECTED-Ёж";

fn setup() -> (DemoService, SessionToken, GroupId) {
    let mut service = DemoService::with_clock(|| 1_700_000_000_000);
    let database = service
        .create_database("Public synthetic database")
        .unwrap();
    let session = service.unlock(&database, DEMO_PASSWORD).unwrap();
    let group = service
        .create_group(&session, "Examples".into(), None)
        .unwrap()
        .value
        .id;
    (service, session, group)
}

fn form(title: &str) -> EditableEntry {
    EditableEntry {
        title: title.into(),
        username: Some("demo@example.invalid".into()),
        password: Some(PASSWORD.into()),
        url: Some("https://example.invalid".into()),
        notes: Some("Public notes / Публичная заметка".into()),
        tags: vec!["demo".into(), "образец".into()],
        expires_at: Some(2_000_000_000_000),
        attributes: vec![
            EditableAttribute {
                id: None,
                name: "PROTECTED-NAME-NOT-SEARCHED".into(),
                value: PROTECTED.into(),
                protected: true,
            },
            EditableAttribute {
                id: None,
                name: "UNPROTECTED-NAME-NOT-SEARCHED".into(),
                value: "public-searchable-attribute".into(),
                protected: false,
            },
        ],
    }
}

fn save_example(
    service: &mut DemoService,
    session: &SessionToken,
    group: &GroupId,
    title: &str,
) -> (EntryId, EditableEntry) {
    service.start_create_entry(session, group.clone()).unwrap();
    let fields = service
        .update_draft(session, form(title))
        .unwrap()
        .value
        .fields;
    let entry = service.save_draft(session).unwrap().value;
    (entry, fields)
}

#[test]
fn new_database_is_locked_empty_and_requires_a_real_group_from_that_database() {
    let mut service = DemoService::new();
    let first = service.create_database("Public empty database").unwrap();
    assert!(service.databases()[0].locked);
    let first_session = service.unlock(&first, DEMO_PASSWORD).unwrap();
    assert!(service.groups(&first_session).unwrap().value.is_empty());
    assert!(
        service
            .entries(&first_session, None, "")
            .unwrap()
            .value
            .is_empty()
    );
    let second = service.create_database("Second public database").unwrap();
    let second_session = service.unlock(&second, DEMO_PASSWORD).unwrap();
    let foreign_group = service
        .create_group(&second_session, "Other group".into(), None)
        .unwrap()
        .value
        .id;
    assert_eq!(
        service
            .start_create_entry(&first_session, foreign_group)
            .unwrap_err(),
        ServiceError::NotFound
    );
    assert!(service.draft(&first_session).unwrap().value.is_none());
    let group = service
        .create_group(&first_session, "Top level".into(), None)
        .unwrap()
        .value;
    assert_eq!(group.parent, None);
    assert!(service.start_create_entry(&first_session, group.id).is_ok());
}

#[test]
fn save_edit_cancel_and_history_use_real_document_state() {
    let (mut service, session, group) = setup();
    let (entry, fields) = save_example(&mut service, &session, &group, "Public original");
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    let original_revision = service.history(&session, &entry).unwrap().value[0]
        .id
        .clone();
    let editing = service.start_edit_entry(&session, &entry).unwrap().value;
    assert_eq!(editing.fields.title, fields.title);
    assert!(!editing.dirty);
    let mut changed = editing.fields;
    changed.title = "Unsaved title".into();
    service.update_draft(&session, changed).unwrap();
    assert_eq!(
        service.view_entry(&session, &entry).unwrap().value.title,
        "Public original"
    );
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    service.cancel_draft(&session).unwrap();
    assert_eq!(
        service.view_entry(&session, &entry).unwrap().value.title,
        "Public original"
    );
    let mut changed = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .fields;
    changed.title = "Public changed".into();
    changed.password = Some("PUBLIC second password".into());
    service.update_draft(&session, changed).unwrap();
    assert_eq!(service.save_draft(&session).unwrap().value, entry);
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 2);
    let old = service
        .revision(&session, &entry, &original_revision)
        .unwrap()
        .value;
    assert_eq!(old.title, "Public original");
    assert!(old.has_password);
    assert_eq!(
        service
            .reveal_revision_password(&session, &entry, &original_revision)
            .unwrap()
            .value
            .expose(),
        PASSWORD
    );
    assert_eq!(
        service
            .reveal_password(&session, &entry)
            .unwrap()
            .value
            .expose(),
        "PUBLIC second password"
    );
    service.start_edit_entry(&session, &entry).unwrap();
    service.save_draft(&session).unwrap();
    assert_eq!(
        service.history(&session, &entry).unwrap().value.len(),
        2,
        "no-op save must not invent a revision"
    );
}

#[test]
fn lock_revokes_responses_and_requires_explicit_draft_restoration() {
    let (mut service, session, group) = setup();
    let (entry, _) = save_example(&mut service, &session, &group, "Public original");
    let revealed = service.reveal_password(&session, &entry).unwrap();
    let mut fields = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .fields;
    fields.notes = Some("PUBLIC interrupted draft".into());
    service.update_draft(&session, fields.clone()).unwrap();
    service.lock(&session).unwrap();
    assert!(!service.is_current(&revealed.session));
    assert_eq!(
        service.reveal_password(&session, &entry).unwrap_err(),
        ServiceError::Locked
    );
    assert_eq!(service.draft(&session).unwrap_err(), ServiceError::Locked);
    let reopened = service.unlock(&session.database, DEMO_PASSWORD).unwrap();
    assert_eq!(
        service.view_entry(&session, &entry).unwrap_err(),
        ServiceError::ExpiredSession
    );
    assert!(service.draft(&reopened).unwrap().value.is_none());
    assert_eq!(
        service
            .pending_draft(&reopened)
            .unwrap()
            .value
            .unwrap()
            .entry_id,
        Some(entry.clone())
    );
    assert_eq!(
        service.save_draft(&reopened).unwrap_err(),
        ServiceError::DraftNeedsRestore
    );
    assert_eq!(
        service.start_edit_entry(&reopened, &entry).unwrap_err(),
        ServiceError::DraftNeedsRestore
    );
    let restored = service.restore_draft(&reopened).unwrap();
    assert_eq!(restored.session, reopened);
    assert_eq!(restored.value.fields, fields);
    assert!(restored.value.dirty);
    assert_eq!(service.history(&reopened, &entry).unwrap().value.len(), 1);
    assert!(service.accepts_response(&reopened, &restored.session));
    assert!(!service.accepts_response(&reopened, &revealed.session));
    service.save_draft(&reopened).unwrap();
    assert_eq!(service.history(&reopened, &entry).unwrap().value.len(), 2);
}

#[test]
fn clean_editors_are_discarded_and_pending_drafts_can_be_cancelled() {
    let (mut service, session, group) = setup();
    let (entry, _) = save_example(&mut service, &session, &group, "Public example");
    service.start_edit_entry(&session, &entry).unwrap();
    service.lock(&session).unwrap();
    let session = service.unlock(&session.database, DEMO_PASSWORD).unwrap();
    assert!(service.pending_draft(&session).unwrap().value.is_none());
    service.start_create_entry(&session, group).unwrap();
    service
        .update_draft(&session, form("Public pending creation"))
        .unwrap();
    service.lock(&session).unwrap();
    let session = service.unlock(&session.database, DEMO_PASSWORD).unwrap();
    assert!(service.pending_draft(&session).unwrap().value.is_some());
    service.cancel_draft(&session).unwrap();
    assert!(service.pending_draft(&session).unwrap().value.is_none());
    assert_eq!(service.entries(&session, None, "").unwrap().value.len(), 1);
}

#[test]
fn rejected_commands_keep_the_previous_saved_and_draft_state() {
    let (mut service, session, group) = setup();
    let (entry, _) = save_example(&mut service, &session, &group, "Public saved title");
    assert_eq!(
        service
            .unlock(&session.database, "PUBLIC incorrect input")
            .unwrap_err(),
        ServiceError::IncorrectDemoPassword
    );
    assert!(service.is_current(&session));
    assert_eq!(
        service
            .update_group(&session, &group, String::new())
            .unwrap_err(),
        ServiceError::InvalidInput
    );
    assert_eq!(service.groups(&session).unwrap().value[0].name, "Examples");
    let mut fields = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .fields;
    fields.title.clear();
    service.update_draft(&session, fields.clone()).unwrap();
    assert_eq!(
        service.save_draft(&session).unwrap_err(),
        ServiceError::InvalidInput
    );
    assert_eq!(
        service.draft(&session).unwrap().value.unwrap().fields,
        fields
    );
    assert_eq!(
        service.view_entry(&session, &entry).unwrap().value.title,
        "Public saved title"
    );
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    let duplicate = fields.attributes[0].clone();
    let mut invalid = fields.clone();
    invalid.attributes.push(duplicate);
    assert_eq!(
        service.update_draft(&session, invalid).unwrap_err(),
        ServiceError::InvalidInput
    );
    assert_eq!(
        service.draft(&session).unwrap().value.unwrap().fields,
        fields
    );
}

#[test]
fn one_editor_per_database_and_cross_database_identifiers_are_checked() {
    let (mut service, first, first_group) = setup();
    let (first_entry, _) = save_example(&mut service, &first, &first_group, "Public first");
    let (other_entry, _) = save_example(&mut service, &first, &first_group, "Public other");
    service.start_edit_entry(&first, &first_entry).unwrap();
    assert_eq!(
        service.start_edit_entry(&first, &other_entry).unwrap_err(),
        ServiceError::EditorAlreadyOpen
    );
    assert_eq!(
        service
            .start_create_entry(&first, first_group.clone())
            .unwrap_err(),
        ServiceError::EditorAlreadyOpen
    );
    assert!(service.start_edit_entry(&first, &first_entry).is_ok());
    let second_database = service.create_database("Public second database").unwrap();
    let second = service.unlock(&second_database, DEMO_PASSWORD).unwrap();
    let second_group = service
        .create_group(&second, "Second group".into(), None)
        .unwrap()
        .value
        .id;
    service.start_create_entry(&second, second_group).unwrap();
    assert!(service.draft(&first).unwrap().value.is_some());
    assert!(service.draft(&second).unwrap().value.is_some());
    assert_eq!(
        service.view_entry(&second, &first_entry).unwrap_err(),
        ServiceError::NotFound
    );
    assert_eq!(
        service.reveal_password(&second, &first_entry).unwrap_err(),
        ServiceError::NotFound
    );
    assert_eq!(
        service.history(&second, &first_entry).unwrap_err(),
        ServiceError::NotFound
    );
    assert_eq!(
        service
            .create_group(&second, "Invalid parent".into(), Some(first_group))
            .unwrap_err(),
        ServiceError::NotFound
    );
    let first_reply = service.groups(&first).unwrap();
    assert!(service.is_current(&first_reply.session));
    assert!(!service.accepts_response(&second, &first_reply.session));
    service.lock_all();
    assert!(!service.is_current(&first));
    assert!(!service.is_current(&second));
}

#[test]
fn search_and_masked_views_exclude_secrets_and_respect_search_scope() {
    let (mut service, session, group) = setup();
    let (entry, fields) = save_example(&mut service, &session, &group, "Public English / Русский");
    let second_group = service
        .create_group(&session, "Another group".into(), None)
        .unwrap()
        .value
        .id;
    assert!(
        service
            .entries(&session, Some(&second_group), "")
            .unwrap()
            .value
            .is_empty()
    );
    for query in [
        PASSWORD,
        PROTECTED,
        "PROTECTED-NAME-NOT-SEARCHED",
        "UNPROTECTED-NAME-NOT-SEARCHED",
    ] {
        assert!(
            service
                .entries(&session, None, query)
                .unwrap()
                .value
                .is_empty(),
            "unexpected search match for a public test marker"
        );
    }
    for query in [
        "русский",
        "DEMO@",
        "example.invalid",
        "публичная",
        "ОБРАЗЕЦ",
        "SEARCHABLE-ATTRIBUTE",
    ] {
        let results = service
            .entries(&session, Some(&second_group), query)
            .unwrap()
            .value;
        assert_eq!(
            results.len(),
            1,
            "a nonempty query searches the whole database"
        );
        assert_eq!(results[0].id, entry);
    }
    let view = service.view_entry(&session, &entry).unwrap();
    assert!(view.value.has_password);
    assert!(
        view.value
            .attributes
            .iter()
            .filter(|attribute| attribute.protected)
            .all(|attribute| attribute.value.is_none())
    );
    let protected = fields
        .attributes
        .iter()
        .find(|attribute| attribute.protected)
        .unwrap()
        .id
        .as_ref()
        .unwrap();
    assert_eq!(
        service
            .reveal_attribute(&session, &entry, protected)
            .unwrap()
            .value
            .expose(),
        PROTECTED
    );
    let debug = format!(
        "{view:?} {:?} {:?}",
        service.entries(&session, None, "").unwrap(),
        service.reveal_password(&session, &entry).unwrap()
    );
    assert!(!debug.contains(PASSWORD));
    assert!(!debug.contains(PROTECTED));
    let results = service.search_unlocked("русский").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].value.group_name, "Examples");
    service.lock(&session).unwrap();
    assert!(service.search_unlocked("русский").unwrap().is_empty());
}

#[test]
fn attributes_keep_identity_and_atomic_values_while_optional_fields_remain_distinct() {
    let (mut service, session, group) = setup();
    service.start_create_entry(&session, group).unwrap();
    let mut fields = form("Public attribute example");
    fields.username = None;
    fields.notes = Some(String::new());
    let draft = service.update_draft(&session, fields).unwrap().value;
    assert_eq!(
        draft.fields.attributes[0].name,
        "PROTECTED-NAME-NOT-SEARCHED"
    );
    assert_eq!(
        draft.fields.attributes[1].name,
        "UNPROTECTED-NAME-NOT-SEARCHED"
    );
    let protected_id = draft.fields.attributes[0].id.clone().unwrap();
    let entry = service.save_draft(&session).unwrap().value;
    let mut editing = service
        .start_edit_entry(&session, &entry)
        .unwrap()
        .value
        .fields;
    assert_eq!(editing.username, None);
    assert_eq!(editing.notes.as_deref(), Some(""));
    assert_eq!(editing.expires_at, Some(2_000_000_000_000));
    let attribute = editing
        .attributes
        .iter_mut()
        .find(|attribute| attribute.id.as_ref() == Some(&protected_id))
        .unwrap();
    attribute.name = "Renamed attribute".into();
    attribute.value = "Public now visible".into();
    attribute.protected = false;
    service.update_draft(&session, editing).unwrap();
    service.save_draft(&session).unwrap();
    let view = service.view_entry(&session, &entry).unwrap().value;
    let attribute = view
        .attributes
        .iter()
        .find(|attribute| attribute.id == protected_id)
        .unwrap();
    assert_eq!(attribute.name, "Renamed attribute");
    assert_eq!(attribute.value.as_deref(), Some("Public now visible"));
    assert!(!attribute.protected);
    assert_eq!(view.username, None);
    assert_eq!(view.notes.as_deref(), Some(""));
}

#[test]
fn invalid_expiration_input_survives_lock_and_cannot_be_saved_as_a_timestamp() {
    let (mut service, session, group) = setup();
    let (entry, _) = save_example(&mut service, &session, &group, "Public date example");
    service.start_edit_entry(&session, &entry).unwrap();
    let invalid = "2030-0";
    service
        .set_draft_expiry_input(&session, Some(invalid.into()))
        .unwrap();
    assert_eq!(
        service.save_draft(&session).unwrap_err(),
        ServiceError::InvalidInput
    );
    service.lock(&session).unwrap();
    let session = service.unlock(&session.database, DEMO_PASSWORD).unwrap();
    let restored = service.restore_draft(&session).unwrap().value;
    assert_eq!(restored.expiry_input.as_deref(), Some(invalid));
    assert!(restored.dirty);
    assert_eq!(restored.fields.expires_at, Some(2_000_000_000_000));
    service.set_draft_expiry_input(&session, None).unwrap();
    service.save_draft(&session).unwrap();
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
}

#[test]
fn demo_seed_uses_only_public_data_and_reopening_service_does_not_fake_persistence() {
    let mut service = DemoService::with_sample_database().unwrap();
    let catalog = service.databases();
    assert_eq!(catalog.len(), 1);
    assert!(catalog[0].locked);
    let session = service.unlock(&catalog[0].id, DEMO_PASSWORD).unwrap();
    let entries = service.entries(&session, None, "").unwrap().value;
    assert_eq!(entries.len(), 1);
    assert!(
        service
            .reveal_password(&session, &entries[0].id)
            .unwrap()
            .value
            .expose()
            .starts_with("PUBLIC ")
    );
    drop(service);
    assert!(DemoService::new().databases().is_empty());
}

#[test]
fn nested_debug_redacts_all_document_content_and_invalid_form_input() {
    let (mut service, session, group) = setup();
    let (entry, _) = save_example(&mut service, &session, &group, "PUBLIC unique debug title");
    let revision = service.history(&session, &entry).unwrap().value[0]
        .id
        .clone();
    service.start_edit_entry(&session, &entry).unwrap();
    service
        .set_draft_expiry_input(&session, Some("PUBLIC invalid expiry input".into()))
        .unwrap();
    let debug = format!(
        "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        service.databases(),
        service.groups(&session).unwrap(),
        service.entries(&session, None, "").unwrap(),
        service.view_entry(&session, &entry).unwrap(),
        service.history(&session, &entry).unwrap(),
        service.revision(&session, &entry, &revision).unwrap(),
        service.draft(&session).unwrap(),
        service.search_unlocked("debug title").unwrap(),
        service.reveal_password(&session, &entry).unwrap(),
    );
    for marker in [
        "Public synthetic database",
        "Examples",
        "PUBLIC unique debug title",
        "demo@example.invalid",
        "https://example.invalid",
        "Публичная заметка",
        "PROTECTED-NAME-NOT-SEARCHED",
        "public-searchable-attribute",
        "PUBLIC invalid expiry input",
        PASSWORD,
        PROTECTED,
    ] {
        assert!(
            !debug.contains(marker),
            "document contents escaped through nested Debug"
        );
    }
}

#[test]
fn global_search_retains_each_source_and_excludes_independently_locked_databases() {
    let (mut service, first, group) = setup();
    save_example(&mut service, &first, &group, "Shared public query");
    let second_database = service.create_database("Public second database").unwrap();
    let second = service.unlock(&second_database, DEMO_PASSWORD).unwrap();
    let second_group = service
        .create_group(&second, "Second group".into(), None)
        .unwrap()
        .value
        .id;
    save_example(&mut service, &second, &second_group, "Shared public query");
    let results = service.search_unlocked("shared").unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .any(|row| row.session == first && row.value.group_name == "Examples")
    );
    assert!(
        results
            .iter()
            .any(|row| row.session == second && row.value.group_name == "Second group")
    );
    service.lock(&first).unwrap();
    let remaining = service.search_unlocked("shared").unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].session, second);
    assert!(
        !service.accepts_response(
            &second,
            &results
                .iter()
                .find(|row| row.session == first)
                .unwrap()
                .session
        )
    );
}
