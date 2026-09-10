//! Public artificial data; replica delivery and availability assertions.

use super::*;

fn fixture() -> (Document, GroupId, GroupId, EntryId) {
    let mut doc = Document::new("PUBLIC lifecycle", 1).unwrap();
    let group = doc
        .create_group("PUBLIC source".into(), None, 2)
        .unwrap()
        .id;
    let other = doc
        .create_group("PUBLIC destination".into(), None, 2)
        .unwrap()
        .id;
    let mut draft = doc.begin_create_entry(group.clone()).unwrap();
    draft.fields_mut().title = "PUBLIC entry".into();
    draft.fields_mut().password = Some("PUBLIC password".into());
    draft.add_attribute("PUBLIC attr".into(), "PUBLIC value".into(), true);
    let entry = doc.save_entry(draft, 3).unwrap();
    (doc, group, other, entry)
}

fn op(id: &str) -> OperationId {
    OperationId::new(format!("PUBLIC {id}"))
}
fn transition(
    doc: &mut Document,
    action: LifecycleAction,
    target: ObjectId,
    destination: Option<GroupId>,
    id: &str,
) {
    let prepared = doc.prepare_lifecycle(action, target, destination).unwrap();
    doc.confirm_lifecycle(&prepared, &op(id), 10).unwrap();
}
fn edit(doc: &mut Document, entry: &EntryId, value: &str) {
    let mut draft = doc.begin_edit_entry(entry).unwrap();
    draft.fields_mut().password = Some(value.into());
    doc.save_entry(draft, 20).unwrap();
}
fn moving(id: &GroupId, parent: Option<GroupId>) -> GroupMove {
    GroupMove {
        group: id.clone(),
        parent,
        position: SiblingPosition::Last,
        review: None,
        name: None,
    }
}

#[test]
fn exact_selection_retains_late_children_and_delete_edit_conflicts() {
    let (mut doc, group, destination, entry) = fixture();
    let mut peer = doc.fork();
    let prepared = doc
        .prepare_lifecycle(LifecycleAction::Trash, ObjectId::Group(group.clone()), None)
        .unwrap();
    let encoded = serde_json::to_string(&prepared).unwrap();
    let prepared: PreparedLifecycle = serde_json::from_str(&encoded).unwrap();
    let child = peer
        .create_group("PUBLIC unseen".into(), Some(group.clone()), 4)
        .unwrap()
        .id;
    edit(&mut peer, &entry, "PUBLIC offline edit");
    doc.merge(&peer).unwrap();
    doc.confirm_lifecycle(&prepared, &op("trash"), 30).unwrap();
    let states = doc.object_states().unwrap();
    assert_eq!(
        states
            .iter()
            .filter(|s| s.status == ObjectStatus::Trashed)
            .count(),
        3
    );
    assert!(
        states
            .iter()
            .find(|s| s.address.object == ObjectId::Entry(entry.clone()))
            .unwrap()
            .conflicted
    );
    assert!(
        !prepared
            .affected
            .keys()
            .any(|a| a.object == ObjectId::Group(child.clone()))
    );
    assert!(doc.entries().unwrap().is_empty());
    assert!(doc.entry(&entry).is_err());
    assert!(
        doc.prepare_lifecycle(LifecycleAction::Purge, ObjectId::Group(group.clone()), None)
            .is_err()
    );
    transition(
        &mut doc,
        LifecycleAction::Restore,
        ObjectId::Group(group.clone()),
        Some(destination),
        "restore",
    );
    assert_eq!(
        doc.entry(&entry)
            .unwrap()
            .fields
            .unwrap()
            .password
            .as_deref(),
        Some("PUBLIC offline edit")
    );
    assert_eq!(doc.history(&entry).unwrap().len(), 3);
    assert!(
        doc.history(&entry)
            .unwrap()
            .iter()
            .any(|r| r.kind == RevisionKind::Restore)
    );
    assert!(doc.groups().unwrap().iter().any(|g| g.id == child));
    let before = doc.export();
    doc.confirm_lifecycle(&prepared, &op("trash"), 99).unwrap();
    assert_eq!(doc.export(), before);
}

#[test]
fn purge_old_delivery_and_late_source_recovery_preserve_generations() {
    let (mut doc, _, destination, entry) = fixture();
    let old = doc.fork();
    let old_context = old.conflict_context(&entry).unwrap();
    let mut late = old.fork();
    edit(&mut late, &entry, "PUBLIC late one");
    transition(
        &mut doc,
        LifecycleAction::Trash,
        ObjectId::Entry(entry.clone()),
        None,
        "trash",
    );
    transition(
        &mut doc,
        LifecycleAction::Purge,
        ObjectId::Entry(entry.clone()),
        None,
        "purge",
    );
    doc.merge(&old).unwrap();
    assert!(doc.pending_sources().unwrap().is_empty());
    doc.merge(&late).unwrap();
    let source = doc.pending_sources().unwrap().pop().unwrap();
    assert!(doc.entry(&entry).is_err());
    assert!(doc.history(&entry).is_err());
    assert!(doc.inspect_entry(&source.address).is_err());
    assert!(
        doc.object_states()
            .unwrap()
            .iter()
            .all(|s| s.address.object != ObjectId::Entry(entry.clone()))
    );
    let request = RecoveryRequest {
        source: source.id.clone(),
        mode: RecoveryMode::Restore,
        destination: Some(destination.clone()),
        name: None,
        fields: None,
    };
    doc.recover_source(&request, &op("recover"), 40).unwrap();
    let restored = doc.entry(&entry).unwrap();
    assert_ne!(restored.generation, source.address.generation);
    assert_eq!(restored.group_id, Some(destination.clone()));
    assert_eq!(restored.created_at, 3);
    assert_eq!(restored.modified_at, 40);
    assert_eq!(doc.history(&entry).unwrap().len(), 1);
    let mut reopened = Document::load(&doc.export()).unwrap();
    let before = reopened.export();
    reopened
        .recover_source(&request, &op("recover"), 99)
        .unwrap();
    assert_eq!(before, reopened.export());
    assert!(
        reopened
            .resolve_fields(
                &old_context,
                vec![Resolution {
                    field: EntryField::Password,
                    value: FieldValue::Text(Some("PUBLIC stale".into()))
                }],
                &op("old context"),
                50
            )
            .is_err()
    );
    edit(&mut late, &entry, "PUBLIC late two");
    reopened.merge(&late).unwrap();
    reopened.merge(&late).unwrap();
    assert_eq!(
        reopened
            .entry(&entry)
            .unwrap()
            .fields
            .unwrap()
            .password
            .as_deref(),
        Some("PUBLIC late one")
    );
    let second = reopened.pending_sources().unwrap();
    assert_eq!(second.len(), 1);
    assert_ne!(second[0].id, source.id);
    assert_eq!(second[0].actor, source.actor);
    let clone = reopened
        .recover_source(
            &RecoveryRequest {
                source: second[0].id.clone(),
                mode: RecoveryMode::Clone,
                destination: Some(destination),
                name: None,
                fields: None,
            },
            &op("clone source"),
            60,
        )
        .unwrap();
    assert_ne!(clone[0], ObjectId::Entry(entry));
    assert!(reopened.pending_sources().unwrap().is_empty());
    assert_eq!(reopened.entries().unwrap().len(), 2);
}

#[test]
fn concurrent_moves_cycles_and_late_resolution_keep_every_placement() {
    let (mut left, a, b, entry) = fixture();
    let mut right = left.fork();
    left.move_group(&moving(&a, Some(b.clone())), &op("a in b"), 5)
        .unwrap();
    right
        .move_group(&moving(&b, Some(a.clone())), &op("b in a"), 5)
        .unwrap();
    left.merge(&right).unwrap();
    right.merge(&left).unwrap();
    assert!(left.groups().unwrap().is_empty());
    assert!(left.entries().unwrap().is_empty());
    assert!(
        left.tree()
            .unwrap()
            .iter()
            .all(|n| n.status == ObjectStatus::Unplaced)
    );
    assert!(Document::load(&left.export()).is_ok());
    let mut fix = moving(&a, None);
    fix.review = Some(left.review_heads());
    left.move_group(&fix, &op("break cycle"), 6).unwrap();
    assert_eq!(left.groups().unwrap().len(), 2);
    assert_eq!(left.entries().unwrap().len(), 1);
    let c = left.create_group("PUBLIC C".into(), None, 7).unwrap().id;
    let mut other = left.fork();
    let mut late = left.fork();
    left.move_entry(&entry, b.clone(), None, &op("entry B"), 8)
        .unwrap();
    other
        .move_entry(&entry, c.clone(), None, &op("entry C"), 8)
        .unwrap();
    late.move_entry(&entry, b.clone(), None, &op("entry late"), 9)
        .unwrap();
    left.merge(&other).unwrap();
    let reviewed = left.review_heads();
    left.merge(&late).unwrap();
    left.move_entry(&entry, a, Some(reviewed), &op("resolve placement"), 10)
        .unwrap();
    let address = objects::single(&left.doc, &ObjectId::Entry(entry)).unwrap();
    assert_eq!(left.inspect_entry(&address).unwrap().placements.len(), 2);
    assert_eq!(
        left.object_status(&address).unwrap().0,
        ObjectStatus::Unplaced
    );
}

#[test]
fn subtree_clone_remaps_all_identities_and_move_noops_keep_history() {
    let (mut doc, group, destination, entry) = fixture();
    let child = doc
        .create_group("PUBLIC child".into(), Some(group.clone()), 4)
        .unwrap()
        .id;
    doc.move_entry(&entry, child, None, &op("move"), 5).unwrap();
    let before = doc.history(&entry).unwrap().len();
    doc.move_entry(
        &entry,
        doc.entry(&entry).unwrap().group_id.unwrap(),
        None,
        &op("no-op"),
        6,
    )
    .unwrap();
    assert_eq!(doc.history(&entry).unwrap().len(), before);
    let cloned = doc
        .clone_group(
            &group,
            Some(destination.clone()),
            Some("PUBLIC cloned tree".into()),
            &op("clone tree"),
            7,
        )
        .unwrap();
    assert_eq!(cloned.len(), 3);
    let ObjectId::Group(root) = &cloned[0] else {
        panic!()
    };
    assert_eq!(
        doc.groups()
            .unwrap()
            .iter()
            .find(|g| &g.id == root)
            .unwrap()
            .parent,
        Some(destination)
    );
    let source = doc.entry(&entry).unwrap();
    let clone = doc
        .entries()
        .unwrap()
        .into_iter()
        .find(|e| e.id != entry)
        .unwrap();
    assert_ne!(source.group_id, clone.group_id);
    assert!(
        source.fields.unwrap().attributes.keys().all(|id| !clone
            .fields
            .as_ref()
            .unwrap()
            .attributes
            .contains_key(id))
    );
    assert_eq!(doc.history(&clone.id).unwrap().len(), 1);
    assert_eq!(doc.history(&clone.id).unwrap()[0].kind, RevisionKind::Clone);
    let events = objects::events(
        &doc.doc,
        &objects::single(&doc.doc, &ObjectId::Group(root.clone())).unwrap(),
    )
    .unwrap();
    let parent = doc
        .groups()
        .unwrap()
        .iter()
        .find(|g| &g.id == root)
        .unwrap()
        .parent
        .clone();
    doc.move_group(&moving(root, parent), &op("group no-op"), 10)
        .unwrap();
    assert_eq!(
        events,
        objects::events(
            &doc.doc,
            &objects::single(&doc.doc, &ObjectId::Group(root.clone())).unwrap()
        )
        .unwrap()
    );
}

#[test]
fn purge_rejects_new_unreviewed_conflicts_without_mutation() {
    let (mut doc, _, _, entry) = fixture();
    let mut late = doc.fork();
    transition(
        &mut doc,
        LifecycleAction::Trash,
        ObjectId::Entry(entry.clone()),
        None,
        "trash",
    );
    let prepared = doc
        .prepare_lifecycle(LifecycleAction::Purge, ObjectId::Entry(entry.clone()), None)
        .unwrap();
    edit(&mut late, &entry, "PUBLIC unseen");
    doc.merge(&late).unwrap();
    let before = doc.export();
    assert_eq!(
        doc.confirm_lifecycle(&prepared, &op("purge"), 30)
            .unwrap_err(),
        Error::Conflict
    );
    assert_eq!(doc.export(), before);
    let mut forged = prepared;
    forged.affected.clear();
    assert_eq!(
        doc.confirm_lifecycle(&forged, &op("forged"), 30)
            .unwrap_err(),
        Error::InvalidContext
    );
    assert_eq!(doc.export(), before);
}

#[test]
fn recovered_parent_never_adopts_late_children_of_its_old_generation() {
    let (mut doc, group, destination, _) = fixture();
    let mut offline = doc.fork();
    let child = offline
        .create_group("PUBLIC late child".into(), Some(group.clone()), 4)
        .unwrap()
        .id;
    offline
        .rename_group(&group, "PUBLIC late parent".into(), 5)
        .unwrap();
    transition(
        &mut doc,
        LifecycleAction::Trash,
        ObjectId::Group(group.clone()),
        None,
        "trash parent",
    );
    transition(
        &mut doc,
        LifecycleAction::Purge,
        ObjectId::Group(group.clone()),
        None,
        "purge parent",
    );
    doc.merge(&offline).unwrap();
    let source = doc
        .pending_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.address.object == ObjectId::Group(group.clone()))
        .unwrap();
    doc.recover_source(
        &RecoveryRequest {
            source: source.id,
            mode: RecoveryMode::Restore,
            destination: Some(destination.clone()),
            name: None,
            fields: None,
        },
        &op("recover parent"),
        30,
    )
    .unwrap();
    let child_state = doc
        .tree()
        .unwrap()
        .into_iter()
        .find(|n| n.address.object == ObjectId::Group(child.clone()))
        .unwrap();
    assert_eq!(child_state.status, ObjectStatus::Unplaced);
    assert!(
        child_state.placements[0]
            .parent
            .as_ref()
            .unwrap()
            .generation
            != doc
                .groups()
                .unwrap()
                .iter()
                .find(|g| g.id == group)
                .unwrap()
                .generation
    );
    doc.move_group(&moving(&child, Some(group)), &op("place late child"), 31)
        .unwrap();
    assert!(doc.groups().unwrap().iter().any(|g| g.id == child));
}

#[test]
fn concurrent_recoveries_and_generation_review_keep_an_unseen_third_choice() {
    let (mut doc, _, destination, entry) = fixture();
    let mut offline = doc.fork();
    edit(&mut offline, &entry, "PUBLIC late recovery");
    transition(
        &mut doc,
        LifecycleAction::Trash,
        ObjectId::Entry(entry.clone()),
        None,
        "trash",
    );
    transition(
        &mut doc,
        LifecycleAction::Purge,
        ObjectId::Entry(entry.clone()),
        None,
        "purge",
    );
    doc.merge(&offline).unwrap();
    let request = RecoveryRequest {
        source: doc.pending_sources().unwrap()[0].id.clone(),
        mode: RecoveryMode::Restore,
        destination: Some(destination),
        name: None,
        fields: None,
    };
    let mut second = doc.fork();
    let mut third = doc.fork();
    doc.recover_source(&request, &op("recover 1"), 30).unwrap();
    second
        .recover_source(&request, &op("recover 2"), 30)
        .unwrap();
    third
        .recover_source(&request, &op("recover 3"), 30)
        .unwrap();
    let chosen = objects::single(&doc.doc, &ObjectId::Entry(entry.clone())).unwrap();
    doc.merge(&second).unwrap();
    assert!(doc.entries().unwrap().is_empty());
    assert_eq!(
        doc.object_states()
            .unwrap()
            .iter()
            .filter(|s| s.address.object == ObjectId::Entry(entry.clone()))
            .count(),
        2
    );
    let heads = doc.review_heads();
    doc.merge(&third).unwrap();
    doc.resolve_generation(&chosen, &heads, &op("choose reviewed"), 31)
        .unwrap();
    assert!(doc.entries().unwrap().is_empty());
    assert_eq!(
        objects::current(&doc.doc, &ObjectId::Entry(entry.clone()))
            .unwrap()
            .len(),
        2
    );
    doc.resolve_generation(&chosen, &doc.review_heads(), &op("choose all"), 32)
        .unwrap();
    assert_eq!(doc.entry(&entry).unwrap().generation, chosen.generation);
    assert_eq!(
        Document::load(&doc.export())
            .unwrap()
            .pending_sources()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn restoring_a_subtree_keeps_a_late_source_for_every_concurrently_purged_child() {
    let (mut doc, group, _, _) = fixture();
    let child = doc
        .create_group("PUBLIC nested child".into(), Some(group.clone()), 4)
        .unwrap()
        .id;
    transition(
        &mut doc,
        LifecycleAction::Trash,
        ObjectId::Group(group.clone()),
        None,
        "trash tree",
    );
    let mut other = doc.fork();
    transition(
        &mut doc,
        LifecycleAction::Purge,
        ObjectId::Group(child.clone()),
        None,
        "purge child",
    );
    transition(
        &mut other,
        LifecycleAction::Restore,
        ObjectId::Group(group),
        None,
        "restore tree",
    );
    doc.merge(&other).unwrap();
    let sources = doc.pending_sources().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].address.object, ObjectId::Group(child));
    assert!(matches!(
        doc.preview_source(&sources[0].id).unwrap(),
        SourcePreview::Group(_)
    ));
}

#[test]
fn malformed_order_keys_are_rejected_before_fractional_index_operations() {
    for raw in [
        "[]",
        "[[[],\"PUBLIC\"]]",
        "[[[1],\"PUBLIC\"]]",
        "[[[128],\"\"]]",
    ] {
        assert!(serde_json::from_str::<OrderKey>(raw).is_err());
    }
    let key = OrderKey::between(None, None, "PUBLIC key").unwrap();
    assert_eq!(
        serde_json::from_str::<OrderKey>(&serde_json::to_string(&key).unwrap()).unwrap(),
        key
    );
}

#[test]
fn old_forms_cannot_save_under_a_recovered_parent_or_closed_entry() {
    let (mut doc, group, _, entry) = fixture();
    let unchanged = doc.begin_edit_entry(&entry).unwrap();
    let mut create = doc.begin_create_entry(group.clone()).unwrap();
    create.fields_mut().title = "PUBLIC old create form".into();
    let mut late = doc.fork();
    late.rename_group(&group, "PUBLIC late name".into(), 4)
        .unwrap();
    transition(
        &mut doc,
        LifecycleAction::Trash,
        ObjectId::Group(group.clone()),
        None,
        "trash",
    );
    transition(
        &mut doc,
        LifecycleAction::Purge,
        ObjectId::Group(group),
        None,
        "purge",
    );
    doc.merge(&late).unwrap();
    let source = doc.pending_sources().unwrap()[0].id.clone();
    doc.recover_source(
        &RecoveryRequest {
            source,
            mode: RecoveryMode::Restore,
            destination: None,
            name: None,
            fields: None,
        },
        &op("recover"),
        5,
    )
    .unwrap();
    let before = doc.export();
    assert!(doc.save_entry(unchanged, 6).is_err());
    assert_eq!(
        doc.save_entry(create, 6).unwrap_err(),
        Error::InvalidContext
    );
    assert_eq!(doc.export(), before);
}

#[test]
fn historical_restore_imports_attributes_from_another_retained_generation() {
    let (mut doc, _, destination, entry) = fixture();
    let mut late = doc.fork();
    edit(&mut late, &entry, "PUBLIC late");
    transition(
        &mut doc,
        LifecycleAction::Trash,
        ObjectId::Entry(entry.clone()),
        None,
        "trash",
    );
    transition(
        &mut doc,
        LifecycleAction::Purge,
        ObjectId::Entry(entry.clone()),
        None,
        "purge",
    );
    doc.merge(&late).unwrap();
    let mut request = RecoveryRequest {
        source: doc.pending_sources().unwrap()[0].id.clone(),
        mode: RecoveryMode::Restore,
        destination: Some(destination.clone()),
        name: None,
        fields: None,
    };
    let mut other = doc.fork();
    doc.recover_source(&request, &op("recover local"), 30)
        .unwrap();
    let chosen = objects::single(&doc.doc, &ObjectId::Entry(entry.clone())).unwrap();
    let mut fields = doc.entry(&entry).unwrap().fields.unwrap();
    let id = AttributeId::new("PUBLIC new generation attribute");
    fields.attributes.insert(
        id.clone(),
        Attribute {
            id: id.clone(),
            name: "PUBLIC extra".into(),
            value: AttributeValue {
                value: "PUBLIC extra protected".into(),
                protected: true,
            },
        },
    );
    request.fields = Some(fields);
    other
        .recover_source(&request, &op("recover other"), 31)
        .unwrap();
    let revision = other.history(&entry).unwrap()[0].id.clone();
    doc.merge(&other).unwrap();
    doc.resolve_generation(&chosen, &doc.review_heads(), &op("choose local"), 32)
        .unwrap();
    assert!(
        !doc.entry(&entry)
            .unwrap()
            .fields
            .unwrap()
            .attributes
            .contains_key(&id)
    );
    doc.restore_revision(
        &entry,
        &revision,
        destination,
        &op("restore other revision"),
        33,
    )
    .unwrap();
    assert_eq!(
        doc.entry(&entry).unwrap().fields.unwrap().attributes[&id]
            .value
            .value,
        "PUBLIC extra protected"
    );
    assert_eq!(doc.entry(&entry).unwrap().generation, chosen.generation);
    assert!(Document::load(&doc.export()).is_ok());
}
