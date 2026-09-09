use super::*;

fn setup() -> (Document, GroupId, EntryId) {
    let mut document = Document::new("PUBLIC test database", 1_000).unwrap();
    let group = document
        .create_group("PUBLIC group".into(), None, 1_000)
        .unwrap();
    let mut draft = document.begin_create_entry(group.id.clone()).unwrap();
    draft.fields_mut().title = "PUBLIC entry".into();
    draft.fields_mut().username = Some("PUBLIC original login".into());
    draft.fields_mut().password = Some("PUBLIC original password".into());
    let entry = document.save_entry(draft, 1_000).unwrap();
    (document, group.id, entry)
}

fn state(document: &Document, entry: &EntryId, field: EntryField) -> FieldState {
    document
        .entry(entry)
        .unwrap()
        .values
        .into_iter()
        .find(|value| value.field == field)
        .unwrap()
}

fn add_attribute(document: &mut Document, entry: &EntryId) -> AttributeId {
    let mut draft = document.begin_edit_entry(entry).unwrap();
    let attr = draft.add_attribute("PUBLIC attribute".into(), "PUBLIC old value".into(), true);
    document.save_entry(draft, 2_000).unwrap();
    attr
}

#[test]
fn empty_database_requires_an_explicit_group_and_validates_before_commit() {
    let mut document = Document::new("PUBLIC database", 1_000).unwrap();
    assert!(document.groups().unwrap().is_empty());
    assert!(document.entries().unwrap().is_empty());
    assert_eq!(
        document
            .begin_create_entry(GroupId::new("missing"))
            .unwrap_err(),
        Error::NotFound
    );
    assert_eq!(
        document
            .create_group(String::new(), None, 1_000)
            .unwrap_err(),
        Error::Validation(ValidationError::EmptyGroupName)
    );
    let group = document
        .create_group("  PUBLIC группа  ".into(), None, 1_000)
        .unwrap();
    assert_eq!(group.name, "  PUBLIC группа  ");
    let invalid = document.begin_create_entry(group.id).unwrap();
    assert_eq!(
        document.save_entry(invalid, 1_000).unwrap_err(),
        Error::Validation(ValidationError::EmptyTitle)
    );
    assert!(document.entries().unwrap().is_empty());
}

#[test]
fn group_nesting_append_order_and_rename_preserve_identity() {
    let mut document = Document::new("PUBLIC database", 1_000).unwrap();
    let root = document
        .create_group("PUBLIC root".into(), None, 1_000)
        .unwrap();
    let first = document
        .create_group("PUBLIC first".into(), Some(root.id.clone()), 2_000)
        .unwrap();
    let second = document
        .create_group("PUBLIC second".into(), Some(root.id.clone()), 3_000)
        .unwrap();
    assert!(first.order < second.order);
    document
        .rename_group(&first.id, "PUBLIC renamed".into(), 4_000)
        .unwrap();
    let renamed = document
        .groups()
        .unwrap()
        .into_iter()
        .find(|group| group.id == first.id)
        .unwrap();
    assert_eq!(renamed.parent, first.parent);
    assert_eq!(renamed.order, first.order);
    assert_eq!(renamed.created_at, first.created_at);
    assert_eq!(renamed.modified_at, 4_000);
    assert_eq!(
        document
            .create_group(
                "PUBLIC bad parent".into(),
                Some(GroupId::new("missing")),
                5_000
            )
            .unwrap_err(),
        Error::NotFound
    );
}

#[test]
fn confirmation_is_atomic_exact_and_retryable() {
    let (mut document, group, entry) = setup();
    let mut draft = document.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().password = Some("  PUBLIC e\u{301} пароль \n".into());
    draft.fields_mut().notes = Some(String::new());
    draft.fields_mut().expires_at = Some(1_234_567_890_123);
    draft.fields_mut().tags = BTreeSet::from(["Work".into(), "work".into()]);
    let retry = draft.clone();
    document.save_entry(draft, 2_000).unwrap();
    document.save_entry(retry.clone(), 9_000).unwrap();
    let snapshot = document.entry(&entry).unwrap();
    let fields = snapshot.fields.unwrap();
    assert_eq!(snapshot.group_id, group);
    assert_eq!(
        fields.password.as_deref(),
        Some("  PUBLIC e\u{301} пароль \n")
    );
    assert_eq!(fields.notes, Some(String::new()));
    assert_eq!(fields.expires_at, Some(1_234_567_890_123));
    assert_eq!(fields.tags.len(), 2);
    assert_eq!(document.history(&entry).unwrap().len(), 2);
    let mut different_retry = retry;
    different_retry.fields_mut().notes = None;
    assert_eq!(
        document.save_entry(different_retry, 3_000).unwrap_err(),
        Error::DuplicateId
    );
    let mut invalid = document.begin_edit_entry(&entry).unwrap();
    invalid.fields_mut().title.clear();
    assert_eq!(
        document.save_entry(invalid, 3_000).unwrap_err(),
        Error::Validation(ValidationError::EmptyTitle)
    );
    assert_eq!(document.history(&entry).unwrap().len(), 2);
}

#[test]
fn cancellation_and_unchanged_confirmation_do_not_create_history() {
    let (mut document, _, entry) = setup();
    let mut cancelled = document.begin_edit_entry(&entry).unwrap();
    cancelled.fields_mut().password = Some("PUBLIC cancelled".into());
    drop(cancelled);
    let unchanged = document.begin_edit_entry(&entry).unwrap();
    document.save_entry(unchanged, 2_000).unwrap();
    assert_eq!(document.history(&entry).unwrap().len(), 1);
    assert_eq!(
        document
            .entry(&entry)
            .unwrap()
            .fields
            .unwrap()
            .password
            .as_deref(),
        Some("PUBLIC original password")
    );
}

#[test]
fn independent_fields_merge_with_one_revision_per_confirmation() {
    let (mut mac, _, entry) = setup();
    let mut phone = mac.fork();
    let mut mac_draft = mac.begin_edit_entry(&entry).unwrap();
    let mut phone_draft = phone.begin_edit_entry(&entry).unwrap();
    mac_draft.fields_mut().username = Some("PUBLIC Mac login".into());
    phone_draft.fields_mut().password = Some("PUBLIC phone password".into());
    mac.save_entry(mac_draft, 2_000).unwrap();
    phone.save_entry(phone_draft, 3_000).unwrap();
    mac.merge(&phone).unwrap();
    phone.merge(&mac).unwrap();
    assert_eq!(mac.entry(&entry).unwrap(), phone.entry(&entry).unwrap());
    let fields = mac.entry(&entry).unwrap().fields.unwrap();
    assert_eq!(fields.username.as_deref(), Some("PUBLIC Mac login"));
    assert_eq!(fields.password.as_deref(), Some("PUBLIC phone password"));
    let history = mac.history(&entry).unwrap();
    assert_eq!(history.len(), 3); // Creation plus the two user confirmations.
    assert_eq!(
        history[1]
            .snapshot
            .fields
            .as_ref()
            .unwrap()
            .password
            .as_deref(),
        Some("PUBLIC original password")
    );
    mac.merge(&phone).unwrap();
    assert_eq!(mac.history(&entry).unwrap().len(), 3);
}

#[test]
fn stale_draft_changes_only_user_modified_fields() {
    let (mut document, _, entry) = setup();
    let mut stale = document.begin_edit_entry(&entry).unwrap();
    stale.fields_mut().username = Some("PUBLIC local login".into());
    let mut remote = document.fork();
    let mut incoming = remote.begin_edit_entry(&entry).unwrap();
    incoming.fields_mut().password = Some("PUBLIC incoming password".into());
    remote.save_entry(incoming, 2_000).unwrap();
    document.merge(&remote).unwrap();
    document.save_entry(stale, 3_000).unwrap();
    let fields = document.entry(&entry).unwrap().fields.unwrap();
    assert_eq!(fields.username.as_deref(), Some("PUBLIC local login"));
    assert_eq!(fields.password.as_deref(), Some("PUBLIC incoming password"));
    assert_eq!(document.history(&entry).unwrap().len(), 3);
}

#[test]
fn same_field_conflicts_remain_whole_values_despite_clock_skew() {
    let (mut document, _, entry) = setup();
    let mut remote = document.fork();
    let mut local = document.begin_edit_entry(&entry).unwrap();
    let mut incoming = remote.begin_edit_entry(&entry).unwrap();
    local.fields_mut().notes = Some("PUBLIC left complete note".into());
    incoming.fields_mut().notes = Some("PUBLIC right complete note".into());
    document.save_entry(local, 9_000).unwrap();
    remote.save_entry(incoming, -9_000).unwrap();
    document.merge(&remote).unwrap();
    remote.merge(&document).unwrap();
    let snapshot = document.entry(&entry).unwrap();
    assert!(snapshot.fields.is_none());
    assert!(snapshot.has_conflicts());
    let notes = state(&document, &entry, EntryField::Notes);
    assert_eq!(notes.variants.len(), 2);
    assert_eq!(notes, state(&remote, &entry, EntryField::Notes));
    let values: BTreeSet<_> = notes
        .variants
        .iter()
        .map(|variant| match &variant.value {
            FieldValue::Text(Some(value)) => value.as_str(),
            _ => panic!("unexpected type"),
        })
        .collect();
    assert_eq!(
        values,
        BTreeSet::from(["PUBLIC left complete note", "PUBLIC right complete note"])
    );
    assert_eq!(
        document.begin_edit_entry(&entry).unwrap_err(),
        Error::Conflict
    );
}

#[test]
fn stale_draft_does_not_resolve_an_unseen_same_field_change() {
    let (mut document, _, entry) = setup();
    let mut stale = document.begin_edit_entry(&entry).unwrap();
    stale.fields_mut().password = Some("PUBLIC stale choice".into());
    let mut recent = document.begin_edit_entry(&entry).unwrap();
    recent.fields_mut().password = Some("PUBLIC recent choice".into());
    document.save_entry(recent, 2_000).unwrap();
    document.save_entry(stale, 3_000).unwrap();
    assert_eq!(
        state(&document, &entry, EntryField::Password)
            .variants
            .len(),
        2
    );
}

#[test]
fn explicit_absence_conflicts_with_concurrent_new_value() {
    let (mut document, _, entry) = setup();
    let mut other = document.fork();
    let mut clear = document.begin_edit_entry(&entry).unwrap();
    let mut replace = other.begin_edit_entry(&entry).unwrap();
    clear.fields_mut().password = None;
    replace.fields_mut().password = Some(String::new());
    document.save_entry(clear, 2_000).unwrap();
    other.save_entry(replace, 2_000).unwrap();
    document.merge(&other).unwrap();
    let variants = state(&document, &entry, EntryField::Password).variants;
    assert_eq!(variants.len(), 2);
    assert!(
        variants
            .iter()
            .any(|value| value.value == FieldValue::Text(None))
    );
    assert!(
        variants
            .iter()
            .any(|value| value.value == FieldValue::Text(Some(String::new())))
    );
}

#[test]
fn identical_parallel_values_share_one_presentation_but_keep_origins() {
    let (mut document, _, entry) = setup();
    let mut other = document.fork();
    for replica in [&mut document, &mut other] {
        let mut draft = replica.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().password = Some("PUBLIC same result".into());
        replica.save_entry(draft, 2_000).unwrap();
    }
    document.merge(&other).unwrap();
    let variants = state(&document, &entry, EntryField::Password).variants;
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].origins.len(), 2);
    for operation in &variants[0].origins {
        assert!(document.source_change(operation).is_some());
    }
    assert!(!document.entry(&entry).unwrap().has_conflicts());
    assert_eq!(document.history(&entry).unwrap().len(), 3);
}

#[test]
fn a_later_unrelated_edit_preserves_conflicts_in_its_saved_snapshot() {
    let (mut document, _, entry) = setup();
    let mut other = document.fork();
    for (replica, value) in [(&mut document, "PUBLIC A"), (&mut other, "PUBLIC B")] {
        let mut draft = replica.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().password = Some(value.into());
        replica.save_entry(draft, 2_000).unwrap();
    }
    document.merge(&other).unwrap();
    let before = state(&document, &entry, EntryField::Password);
    document
        .edit_fields(
            &entry,
            BTreeMap::from([(
                EntryField::Notes,
                FieldValue::Text(Some("PUBLIC unrelated note".into())),
            )]),
            3_000,
        )
        .unwrap();
    assert_eq!(state(&document, &entry, EntryField::Password), before);
    let history = document.history(&entry).unwrap();
    assert_eq!(history.len(), 4);
    assert_eq!(
        history
            .last()
            .unwrap()
            .snapshot
            .values
            .iter()
            .find(|state| state.field == EntryField::Password),
        Some(&before)
    );
    assert_eq!(
        document
            .edit_fields(
                &entry,
                BTreeMap::from([(
                    EntryField::Password,
                    FieldValue::Text(Some("PUBLIC implicit resolution".into()))
                )]),
                4_000
            )
            .unwrap_err(),
        Error::Conflict
    );
}

#[test]
fn attribute_value_and_protection_are_one_register() {
    let (mut document, _, entry) = setup();
    let attr = add_attribute(&mut document, &entry);
    let mut other = document.fork();
    let mut unprotect = document.begin_edit_entry(&entry).unwrap();
    unprotect
        .fields_mut()
        .attributes
        .get_mut(&attr)
        .unwrap()
        .value
        .protected = false;
    let mut replace = other.begin_edit_entry(&entry).unwrap();
    replace
        .fields_mut()
        .attributes
        .get_mut(&attr)
        .unwrap()
        .value
        .value = "PUBLIC new protected value".into();
    document.save_entry(unprotect, 3_000).unwrap();
    other.save_entry(replace, 3_000).unwrap();
    document.merge(&other).unwrap();
    let variants = state(&document, &entry, EntryField::AttributeValue(attr)).variants;
    assert_eq!(variants.len(), 2);
    assert!(variants.iter().any(|variant| variant.value
        == FieldValue::Attribute(AttributeValue {
            value: "PUBLIC new protected value".into(),
            protected: true
        })));
    assert!(document.entry(&entry).unwrap().fields.is_none());
}

#[test]
fn attribute_rename_and_value_edit_merge_under_the_same_id() {
    let (mut document, _, entry) = setup();
    let attr = add_attribute(&mut document, &entry);
    let mut other = document.fork();
    let mut rename = document.begin_edit_entry(&entry).unwrap();
    rename.fields_mut().attributes.get_mut(&attr).unwrap().name = "PUBLIC renamed".into();
    let mut replace = other.begin_edit_entry(&entry).unwrap();
    replace
        .fields_mut()
        .attributes
        .get_mut(&attr)
        .unwrap()
        .value
        .value = "PUBLIC changed value".into();
    document.save_entry(rename, 3_000).unwrap();
    other.save_entry(replace, 3_000).unwrap();
    document.merge(&other).unwrap();
    let fields = document.entry(&entry).unwrap().fields.unwrap();
    assert_eq!(fields.attributes.len(), 1);
    assert_eq!(fields.attributes[&attr].name, "PUBLIC renamed");
    assert_eq!(fields.attributes[&attr].value.value, "PUBLIC changed value");
    assert_eq!(
        state(&document, &entry, EntryField::AttributePresence(attr))
            .variants
            .len(),
        1
    );
}

#[test]
fn concurrent_duplicate_attribute_names_keep_both_ids_for_review() {
    let (mut document, _, entry) = setup();
    let mut other = document.fork();
    for replica in [&mut document, &mut other] {
        let mut draft = replica.begin_edit_entry(&entry).unwrap();
        draft.add_attribute("PUBLIC same name".into(), "PUBLIC value".into(), true);
        replica.save_entry(draft, 2_000).unwrap();
    }
    document.merge(&other).unwrap();
    let snapshot = document.entry(&entry).unwrap();
    assert!(snapshot.fields.is_none());
    assert_eq!(snapshot.conflicts.len(), 2);
    assert_eq!(
        snapshot
            .values
            .iter()
            .filter(|state| matches!(state.field, EntryField::AttributeName(_)))
            .count(),
        2
    );
}

#[test]
fn attribute_deletion_and_concurrent_rename_retain_the_object() {
    let (mut document, _, entry) = setup();
    let attr = add_attribute(&mut document, &entry);
    let mut other = document.fork();
    let mut remove = document.begin_edit_entry(&entry).unwrap();
    remove.fields_mut().attributes.remove(&attr);
    let mut rename = other.begin_edit_entry(&entry).unwrap();
    rename.fields_mut().attributes.get_mut(&attr).unwrap().name = "PUBLIC concurrent name".into();
    document.save_entry(remove, 3_000).unwrap();
    assert!(
        document
            .entry(&entry)
            .unwrap()
            .fields
            .unwrap()
            .attributes
            .is_empty()
    );
    other.save_entry(rename, 3_000).unwrap();
    document.merge(&other).unwrap();
    assert_eq!(
        state(
            &document,
            &entry,
            EntryField::AttributePresence(attr.clone())
        )
        .variants
        .len(),
        2
    );
    assert!(
        state(&document, &entry, EntryField::AttributeValue(attr))
            .variants
            .iter()
            .any(|variant| matches!(variant.value, FieldValue::Attribute(_)))
    );
}

#[test]
fn drafts_and_merges_cannot_cross_database_boundaries() {
    let (mut document, group, _) = setup();
    let mut other = Document::new("PUBLIC unrelated", 1_000).unwrap();
    let mut draft = document.begin_create_entry(group).unwrap();
    draft.fields_mut().title = "PUBLIC entry".into();
    assert_eq!(
        other.save_entry(draft, 2_000).unwrap_err(),
        Error::InvalidContext
    );
    assert_eq!(document.merge(&other).unwrap_err(), Error::InvalidContext);
}

#[test]
fn unknown_properties_survive_addressed_updates_and_history_is_not_self_referential() {
    let (mut document, _, entry) = setup();
    let entries = object(&document.doc, &ROOT, "entries").unwrap();
    let object = object(&document.doc, &entries, entry.as_str()).unwrap();
    let mut tx = document.doc.transaction();
    tx.put(&object, "future_extension", "PUBLIC untouched extension")
        .unwrap();
    tx.commit();
    let mut draft = document.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().username = Some("PUBLIC new login".into());
    document.save_entry(draft, 2_000).unwrap();
    assert_eq!(
        document
            .doc
            .get(&object, "future_extension")
            .unwrap()
            .unwrap()
            .0
            .to_str(),
        Some("PUBLIC untouched extension")
    );
    let history = document.history(&entry).unwrap();
    let revision = history.last().unwrap();
    let login = revision
        .snapshot
        .values
        .iter()
        .find(|state| state.field == EntryField::Username)
        .unwrap();
    let source = document
        .source_change(&login.variants[0].origins[0])
        .unwrap();
    assert!(!revision.base.contains(&source));
    assert!(!serde_json::to_string(revision).unwrap().contains(&source));
}

#[test]
fn display_time_tracks_current_source_operations_not_delivery_or_overwritten_values() {
    let (mut document, _, entry) = setup();
    let mut future = document.begin_edit_entry(&entry).unwrap();
    future.fields_mut().password = Some("PUBLIC future-clock value".into());
    document.save_entry(future, 90_000).unwrap();
    let mut correction = document.begin_edit_entry(&entry).unwrap();
    correction.fields_mut().password = Some("PUBLIC corrected-clock value".into());
    document.save_entry(correction, 2_000).unwrap();
    assert_eq!(document.entry(&entry).unwrap().modified_at, 2_000);
}

#[test]
fn diagnostic_output_does_not_include_document_contents() {
    let (document, _, entry) = setup();
    let draft = document.begin_edit_entry(&entry).unwrap();
    let rendered = format!(
        "{document:?} {draft:?} {:?} {:?}",
        document.entry(&entry).unwrap(),
        document.history(&entry).unwrap()
    );
    assert!(!rendered.contains("PUBLIC"));
    assert!(rendered.contains("redacted"));
}

#[test]
fn group_rename_preserves_unknown_fields_and_coalesces_equal_names() {
    let (mut document, group, _) = setup();
    let groups = object(&document.doc, &ROOT, "groups").unwrap();
    let group_object = object(&document.doc, &groups, group.as_str()).unwrap();
    let mut tx = document.doc.transaction();
    tx.put(
        &group_object,
        "future_group_extension",
        "PUBLIC preserved extension",
    )
    .unwrap();
    tx.commit();
    let mut other = document.fork();
    document
        .rename_group(&group, "PUBLIC identical name".into(), 2_000)
        .unwrap();
    other
        .rename_group(&group, "PUBLIC identical name".into(), 3_000)
        .unwrap();
    document.merge(&other).unwrap();
    other.merge(&document).unwrap();
    assert_eq!(document.groups().unwrap(), other.groups().unwrap());
    let groups = document.groups().unwrap();
    assert_eq!(groups[0].name, "PUBLIC identical name");
    assert_eq!(groups[0].modified_at, 3_000);
    assert_eq!(
        document.doc.get_all(&group_object, "name").unwrap().len(),
        2
    );
    assert_eq!(
        document
            .doc
            .get(&group_object, "future_group_extension")
            .unwrap()
            .unwrap()
            .0
            .to_str(),
        Some("PUBLIC preserved extension")
    );
}

#[test]
fn conflicting_group_names_are_preserved_without_a_winner() {
    let (mut document, group, _) = setup();
    let mut other = document.fork();
    document
        .rename_group(&group, "PUBLIC left name".into(), 2_000)
        .unwrap();
    other
        .rename_group(&group, "PUBLIC right name".into(), 3_000)
        .unwrap();
    document.merge(&other).unwrap();
    assert_eq!(document.groups().unwrap_err(), Error::Conflict);
    let groups = object(&document.doc, &ROOT, "groups").unwrap();
    let group_object = object(&document.doc, &groups, group.as_str()).unwrap();
    assert_eq!(document.doc.get_all(group_object, "name").unwrap().len(), 2);
}

#[test]
fn concurrent_sibling_appends_have_a_stable_id_tiebreaker() {
    let (mut document, group, _) = setup();
    let mut other = document.fork();
    let left = document
        .create_group("PUBLIC child left".into(), Some(group.clone()), 2_000)
        .unwrap();
    let right = other
        .create_group("PUBLIC child right".into(), Some(group.clone()), 2_000)
        .unwrap();
    assert_eq!(left.order, right.order);
    document.merge(&other).unwrap();
    other.merge(&document).unwrap();
    assert_eq!(document.groups().unwrap(), other.groups().unwrap());
    let children: Vec<_> = document
        .groups()
        .unwrap()
        .into_iter()
        .filter(|child| child.parent.as_ref() == Some(&group))
        .collect();
    assert_eq!(children.len(), 2);
    assert!(children[0].id < children[1].id);
}

#[test]
fn a_new_attribute_cannot_reuse_another_entrys_identity() {
    let (mut document, group, entry) = setup();
    let attr = add_attribute(&mut document, &entry);
    let original = document.entry(&entry).unwrap().fields.unwrap().attributes[&attr].clone();
    let mut draft = document.begin_create_entry(group).unwrap();
    draft.fields_mut().title = "PUBLIC another entry".into();
    draft.fields_mut().attributes.insert(attr, original);
    assert_eq!(
        document.save_entry(draft, 3_000).unwrap_err(),
        Error::DuplicateId
    );
    assert_eq!(document.entries().unwrap().len(), 1);
}

#[test]
fn removed_attribute_content_is_only_in_history_and_does_not_set_current_display_time() {
    let (mut document, _, entry) = setup();
    let mut add = document.begin_edit_entry(&entry).unwrap();
    let attr = add.add_attribute("PUBLIC attribute".into(), "PUBLIC old content".into(), true);
    document.save_entry(add, 90_000).unwrap();
    let mut remove = document.begin_edit_entry(&entry).unwrap();
    remove.fields_mut().attributes.remove(&attr);
    document.save_entry(remove, 2_000).unwrap();
    let snapshot = document.entry(&entry).unwrap();
    assert_eq!(snapshot.modified_at, 2_000);
    assert!(
        snapshot
            .values
            .iter()
            .all(|state| state.field != EntryField::AttributeValue(attr.clone()))
    );
    assert!(document.history(&entry).unwrap().iter().any(|revision| {
        revision
            .snapshot
            .values
            .iter()
            .any(|state| state.field == EntryField::AttributeValue(attr.clone()))
    }));
}

#[test]
fn merging_reused_attribute_ids_rejects_atomically_including_deleted_attributes() {
    for remove_on_left in [false, true] {
        let (mut left, group, _) = setup();
        let mut right = left.fork();
        let colliding_id = AttributeId::new("PUBLIC deliberate identity collision");
        let mut created = Vec::new();
        for replica in [&mut left, &mut right] {
            let mut draft = replica.begin_create_entry(group.clone()).unwrap();
            draft.fields_mut().title = "PUBLIC independently created entry".into();
            draft.fields_mut().attributes.insert(
                colliding_id.clone(),
                Attribute {
                    id: colliding_id.clone(),
                    name: "PUBLIC attribute".into(),
                    value: AttributeValue {
                        value: "PUBLIC value".into(),
                        protected: true,
                    },
                },
            );
            created.push(replica.save_entry(draft, 2_000).unwrap());
        }
        assert_ne!(created[0], created[1]);
        if remove_on_left {
            let mut remove = left.begin_edit_entry(&created[0]).unwrap();
            remove.fields_mut().attributes.remove(&colliding_id);
            left.save_entry(remove, 3_000).unwrap();
        }
        let left_heads = left.doc.get_heads();
        let right_heads = right.doc.get_heads();
        let left_entries = left.entries().unwrap();
        let right_entries = right.entries().unwrap();
        assert_eq!(left.merge(&right), Err(Error::DuplicateId));
        assert_eq!(left.doc.get_heads(), left_heads);
        assert_eq!(right.doc.get_heads(), right_heads);
        assert_eq!(left.entries().unwrap(), left_entries);
        assert_eq!(right.entries().unwrap(), right_entries);
        assert_eq!(right.merge(&left), Err(Error::DuplicateId));
        assert_eq!(left.doc.get_heads(), left_heads);
        assert_eq!(right.doc.get_heads(), right_heads);
    }
}

#[test]
fn addressed_noop_edits_preserve_heads_and_history_even_with_unrelated_conflicts() {
    let (mut document, _, entry) = setup();
    let heads = document.doc.get_heads();
    document
        .edit_fields(
            &entry,
            BTreeMap::from([
                (
                    EntryField::Title,
                    FieldValue::Text(Some("PUBLIC entry".into())),
                ),
                (EntryField::Notes, FieldValue::Text(None)),
            ]),
            2_000,
        )
        .unwrap();
    assert_eq!(document.doc.get_heads(), heads);
    assert_eq!(document.history(&entry).unwrap().len(), 1);

    let mut other = document.fork();
    for (replica, password) in [(&mut document, "PUBLIC left"), (&mut other, "PUBLIC right")] {
        let mut draft = replica.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().password = Some(password.into());
        replica.save_entry(draft, 3_000).unwrap();
    }
    document.merge(&other).unwrap();
    let before = document.entry(&entry).unwrap();
    let heads = document.doc.get_heads();
    document
        .edit_fields(
            &entry,
            BTreeMap::from([(
                EntryField::Title,
                FieldValue::Text(Some("PUBLIC entry".into())),
            )]),
            4_000,
        )
        .unwrap();
    assert_eq!(document.doc.get_heads(), heads);
    assert_eq!(document.entry(&entry).unwrap(), before);
    assert_eq!(document.history(&entry).unwrap().len(), 3);
}
