//! PUBLIC artificial scenarios for selected projections, causal continuation and history cleanup.

use super::*;
use taypeer_core::{
    DatabaseMetadataPatch, FieldState, FieldUpdate, GroupMetadataPatch, OperationId, ValueVariant,
};

fn fixture() -> (Document, GroupId, EntryId) {
    let mut document = Document::new("PUBLIC automatic database", 1).unwrap();
    let group = document
        .create_group("PUBLIC group".into(), None, 2)
        .unwrap()
        .id;
    let mut draft = document.begin_create_entry(group.clone()).unwrap();
    draft.fields_mut().title = "PUBLIC entry".into();
    let entry = document.save_entry(draft, 3).unwrap();
    (document, group, entry)
}

#[test]
fn operation_selection_compares_numeric_counters_then_actor_bytes() {
    let mut state = FieldState {
        field: EntryField::Notes,
        variants: vec![
            ValueVariant {
                value: FieldValue::Text(Some("PUBLIC nine".into())),
                origins: vec!["9@ff".into()],
            },
            ValueVariant {
                value: FieldValue::Text(Some("PUBLIC ten".into())),
                origins: vec!["10@00".into()],
            },
        ],
    };
    assert_eq!(
        fields::selected_variant(&state).unwrap().value,
        FieldValue::Text(Some("PUBLIC ten".into()))
    );
    state.variants.push(ValueVariant {
        value: FieldValue::Text(Some("PUBLIC tied".into())),
        origins: vec!["10@01".into()],
    });
    assert_eq!(
        fields::selected_variant(&state).unwrap().value,
        FieldValue::Text(Some("PUBLIC tied".into()))
    );
}

#[test]
fn three_replica_values_and_group_names_converge_after_reload_in_every_delivery_order() {
    let (original, group, entry) = fixture();
    let mut replicas = Vec::new();
    for (value, time) in [
        ("PUBLIC A", 90_000),
        ("PUBLIC B", -90_000),
        ("PUBLIC C", 100),
    ] {
        let mut replica = original.fork();
        let mut draft = replica.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().notes = Some(value.into());
        replica.save_entry(draft, time).unwrap();
        replica.rename_group(&group, value.into(), time).unwrap();
        replica
            .update_metadata_at(value.into(), Some(value.into()), time)
            .unwrap();
        replicas.push(replica);
    }
    let mut expected = None;
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut merged = original.fork();
        for index in order {
            merged.merge(&replicas[index]).unwrap();
        }
        let merged = Document::load(&merged.export()).unwrap();
        let observed = (
            merged.entry(&entry).unwrap(),
            merged.groups().unwrap(),
            merged.display_name().unwrap(),
            merged.description().unwrap(),
        );
        assert!(observed.0.fields.is_some());
        assert_eq!(observed.0.conflicts.len(), 1);
        assert_eq!(merged.history(&entry).unwrap().len(), 4);
        assert_eq!(merged.group_history(&group).unwrap().len(), 4);
        assert_eq!(merged.database_history().unwrap().len(), 4);
        if let Some(expected) = &expected {
            assert_eq!(&observed, expected);
        } else {
            expected = Some(observed);
        }
    }
}

#[test]
fn protected_branch_wins_concurrently_and_later_explicit_unprotection_is_allowed() {
    let (mut local, _, entry) = fixture();
    let mut draft = local.begin_edit_entry(&entry).unwrap();
    let attribute = draft.add_attribute("PUBLIC attribute".into(), "PUBLIC original".into(), true);
    local.save_entry(draft, 4).unwrap();
    let mut remote = local.fork();
    let mut exposed = local.begin_edit_entry(&entry).unwrap();
    exposed
        .fields_mut()
        .attributes
        .get_mut(&attribute)
        .unwrap()
        .value
        .protected = false;
    let mut protected = remote.begin_edit_entry(&entry).unwrap();
    protected
        .fields_mut()
        .attributes
        .get_mut(&attribute)
        .unwrap()
        .value
        .value = "PUBLIC protected branch".into();
    local.save_entry(exposed, 99_000).unwrap();
    remote.save_entry(protected, -99_000).unwrap();
    local.merge(&remote).unwrap();
    let selected = local.entry(&entry).unwrap().fields.unwrap();
    assert!(selected.attributes[&attribute].value.protected);
    assert_eq!(
        selected.attributes[&attribute].value.value,
        "PUBLIC protected branch"
    );
    let mut unprotect = local.begin_edit_entry(&entry).unwrap();
    unprotect
        .fields_mut()
        .attributes
        .get_mut(&attribute)
        .unwrap()
        .value
        .protected = false;
    local.save_entry(unprotect, 5).unwrap();
    assert!(
        !local.entry(&entry).unwrap().fields.unwrap().attributes[&attribute]
            .value
            .protected
    );
}

#[test]
fn ungrouped_entries_survive_reload_and_group_moves_preserve_their_identity() {
    let mut document = Document::new("PUBLIC no group", 1).unwrap();
    let mut draft = document.begin_create_entry_ungrouped().unwrap();
    assert_eq!(draft.group_id(), None);
    draft.fields_mut().title = "PUBLIC ungrouped entry".into();
    let entry = document.save_entry(draft, 2).unwrap();
    document = Document::load(&document.export()).unwrap();
    assert_eq!(document.entries().unwrap().len(), 1);
    assert_eq!(document.entry(&entry).unwrap().group_id, None);
    let group = document
        .create_group("PUBLIC destination".into(), None, 3)
        .unwrap()
        .id;
    document
        .move_entry(
            &entry,
            group.clone(),
            None,
            &OperationId::new("PUBLIC into group"),
            4,
        )
        .unwrap();
    document
        .move_entry_to(&entry, None, None, &OperationId::new("PUBLIC ungroup"), 5)
        .unwrap();
    let mut edit = document.begin_edit_entry(&entry).unwrap();
    edit.fields_mut().notes = Some("PUBLIC still editable".into());
    document.save_entry(edit, 6).unwrap();
    assert_eq!(document.entries().unwrap()[0].id, entry);
    assert_eq!(document.entry(&entry).unwrap().group_id, None);
    assert_eq!(document.history(&entry).unwrap().len(), 4);
}

#[test]
fn cloning_restoring_and_extracting_ungrouped_entries_need_no_implicit_group() {
    let mut document = Document::new("PUBLIC ungrouped operations", 1).unwrap();
    let mut draft = document.begin_create_entry_ungrouped().unwrap();
    draft.fields_mut().title = "PUBLIC original".into();
    let entry = document.save_entry(draft, 2).unwrap();
    let first = document.history(&entry).unwrap()[0].id.clone();
    let cloned = document
        .clone_entry_in(
            &entry,
            None,
            None,
            &OperationId::new("PUBLIC ungrouped clone"),
            3,
        )
        .unwrap();
    let mut draft = document.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().notes = Some("PUBLIC later note".into());
    document.save_entry(draft, 4).unwrap();
    document
        .restore_revision_in(
            &entry,
            &first,
            None,
            &OperationId::new("PUBLIC ungrouped restore"),
            5,
        )
        .unwrap();
    assert_eq!(document.entry(&entry).unwrap().fields.unwrap().notes, None);
    let source = document.clone();
    let source_hash = source.heads()[0].clone();
    let operation = OperationId::new("PUBLIC ungrouped extract");
    let extracted = document
        .extract_entry_in(&source, (&entry, &source_hash), None, &operation, 6)
        .unwrap();
    assert_eq!(
        document
            .extracted_entry_in(&entry, &source_hash, None, &operation)
            .unwrap(),
        Some(extracted.clone())
    );
    for id in [entry, cloned, extracted] {
        assert_eq!(document.entry(&id).unwrap().group_id, None);
    }
    assert!(document.groups().unwrap().is_empty());
    assert_eq!(
        Document::load(&document.export())
            .unwrap()
            .entries()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn continuation_preserves_newer_input_and_does_not_acknowledge_unseen_remote_values() {
    let (mut document, _, entry) = fixture();
    let mut remote = document.fork();
    let mut confirmed = document.begin_edit_entry(&entry).unwrap();
    confirmed.fields_mut().password = Some("PUBLIC confirmed".into());
    document.save_entry(confirmed.clone(), 4).unwrap();
    let mut newer = confirmed.fields().clone();
    newer.password = Some("PUBLIC next local input".into());
    let mut incoming = remote.begin_edit_entry(&entry).unwrap();
    incoming.fields_mut().password = Some("PUBLIC unseen remote".into());
    incoming.fields_mut().title = "PUBLIC independent title".into();
    remote.save_entry(incoming, 5).unwrap();
    document.merge(&remote).unwrap();
    let continued = document.continue_entry_draft(&confirmed, newer).unwrap();
    assert_ne!(continued.revision_id(), confirmed.revision_id());
    document.save_entry(continued, 6).unwrap();
    let snapshot = document.entry(&entry).unwrap();
    assert_eq!(snapshot.fields.unwrap().title, "PUBLIC independent title");
    assert_eq!(
        snapshot
            .values
            .iter()
            .find(|state| state.field == EntryField::Password)
            .unwrap()
            .variants
            .len(),
        2
    );
}

#[test]
fn causal_metadata_patches_preserve_unseen_fields_and_create_one_revision_each() {
    let (mut document, group, _) = fixture();
    let heads = document.heads();
    let mut remote = document.fork();
    remote
        .set_group_description(&group, Some("PUBLIC incoming group description".into()), 4)
        .unwrap();
    remote
        .update_metadata_at(
            "PUBLIC automatic database".into(),
            Some("PUBLIC incoming DB description".into()),
            4,
        )
        .unwrap();
    document.merge(&remote).unwrap();
    document
        .update_group_metadata_command(
            &group,
            &GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC local group name".into()),
                ..Default::default()
            },
            &heads,
            5,
            None,
        )
        .unwrap();
    document
        .patch_metadata_command(
            &DatabaseMetadataPatch {
                name: FieldUpdate::Set("PUBLIC local DB name".into()),
                ..Default::default()
            },
            &heads,
            5,
            None,
        )
        .unwrap();
    assert_eq!(
        document.group_description(&group).unwrap().as_deref(),
        Some("PUBLIC incoming group description")
    );
    assert_eq!(
        document.description().unwrap().as_deref(),
        Some("PUBLIC incoming DB description")
    );
    assert_eq!(document.group_history(&group).unwrap().len(), 3);
    assert_eq!(document.database_history().unwrap().len(), 3);
    let before = document.heads();
    document
        .update_group_metadata_command(
            &group,
            &GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC local group name".into()),
                ..Default::default()
            },
            &before,
            6,
            None,
        )
        .unwrap();
    document
        .update_metadata_at(
            "PUBLIC local DB name".into(),
            Some("PUBLIC incoming DB description".into()),
            6,
        )
        .unwrap();
    assert_eq!(document.heads(), before);
}

#[test]
fn history_cleanup_does_not_copy_original_alternatives_into_later_versions() {
    let (mut document, _, entry) = fixture();
    let mut remote = document.fork();
    for (replica, value) in [
        (&mut document, "PUBLIC left"),
        (&mut remote, "PUBLIC right"),
    ] {
        let mut draft = replica.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().password = Some(value.into());
        replica.save_entry(draft, 4).unwrap();
    }
    document.merge(&remote).unwrap();
    let originals = document
        .history(&entry)
        .unwrap()
        .into_iter()
        .map(|revision| revision.id)
        .collect();
    document
        .purge_history(
            &entry,
            originals,
            &OperationId::new("PUBLIC exact history purge"),
        )
        .unwrap();
    let mut draft = document.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().notes = Some("PUBLIC new note".into());
    document.save_entry(draft, 5).unwrap();
    let reopened = Document::load(&document.export()).unwrap();
    let history = reopened.history(&entry).unwrap();
    assert_eq!(history.len(), 1);
    assert!(!history[0].snapshot.has_conflicts());
    assert!(
        history[0]
            .snapshot
            .values
            .iter()
            .all(|state| state.variants.len() == 1)
    );
    assert_eq!(
        reopened
            .entry(&entry)
            .unwrap()
            .values
            .iter()
            .find(|state| state.field == EntryField::Password)
            .unwrap()
            .variants
            .len(),
        2
    );
}

#[test]
fn clearing_group_and_entry_history_releases_losing_blobs_after_later_saves() {
    let (mut document, group, entry) = fixture();
    let mut draft = document.begin_edit_entry(&entry).unwrap();
    let attachment = draft.add_attachment(
        "PUBLIC attachment".into(),
        BlobId::new("PUBLIC initial blob"),
    );
    document.save_entry(draft, 4).unwrap();
    let mut remote = document.fork();
    for (replica, label) in [
        (&mut document, "PUBLIC blob A"),
        (&mut remote, "PUBLIC blob B"),
    ] {
        let mut draft = replica.begin_edit_entry(&entry).unwrap();
        draft
            .fields_mut()
            .attachments
            .get_mut(&attachment)
            .unwrap()
            .blob = BlobId::new(label);
        replica.save_entry(draft, 5).unwrap();
        replica
            .update_group_metadata_command(
                &group,
                &GroupMetadataPatch {
                    icon: FieldUpdate::Set(IconRef::Image {
                        blob: BlobId::new(label),
                        source: taypeer_core::IconSource::File,
                    }),
                    ..Default::default()
                },
                &replica.heads(),
                5,
                None,
            )
            .unwrap();
    }
    document.merge(&remote).unwrap();
    assert_eq!(document.blob_references().unwrap().retained.len(), 3);
    let selected_attachment = document.entry(&entry).unwrap().fields.unwrap().attachments
        [&attachment]
        .blob
        .clone();
    let selected_icon = document.groups().unwrap()[0].icon.blob().unwrap().clone();
    let entry_versions = document
        .history(&entry)
        .unwrap()
        .into_iter()
        .map(|revision| revision.id)
        .collect();
    document
        .purge_history(
            &entry,
            entry_versions,
            &OperationId::new("PUBLIC purge entry versions"),
        )
        .unwrap();
    let group_versions: BTreeSet<_> = document
        .group_history(&group)
        .unwrap()
        .into_iter()
        .map(|revision| revision.id)
        .collect();
    let purge = OperationId::new("PUBLIC purge group versions");
    document
        .purge_group_history(&group, group_versions.clone(), &purge)
        .unwrap();
    document
        .purge_group_history(&group, group_versions, &purge)
        .unwrap();
    let mut draft = document.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().notes = Some("PUBLIC later selected state".into());
    document.save_entry(draft, 6).unwrap();
    document
        .set_group_description(&group, Some("PUBLIC later selected group".into()), 6)
        .unwrap();
    let reopened = Document::load(&document.export()).unwrap();
    assert_eq!(
        reopened.blob_references().unwrap().retained,
        BTreeSet::from([selected_attachment, selected_icon])
    );
    assert_eq!(reopened.group_history(&group).unwrap().len(), 1);
}

#[test]
fn concurrent_subtree_moves_cannot_escape_a_delete_and_late_descendants_stay_in_trash() {
    let (mut document, parent, entry) = fixture();
    let destination = document
        .create_group("PUBLIC other group".into(), None, 4)
        .unwrap()
        .id;
    let child = document
        .create_group("PUBLIC child".into(), Some(parent.clone()), 4)
        .unwrap()
        .id;
    let prepared = document
        .prepare_lifecycle(
            LifecycleAction::Trash,
            ObjectId::Group(parent.clone()),
            None,
        )
        .unwrap();
    let mut remote = document.fork();
    remote
        .move_group(
            &GroupMove {
                group: child.clone(),
                parent: Some(destination.clone()),
                position: SiblingPosition::Last,
                name: None,
                review: None,
            },
            &OperationId::new("PUBLIC move child"),
            5,
        )
        .unwrap();
    remote
        .move_entry(
            &entry,
            destination,
            None,
            &OperationId::new("PUBLIC move entry"),
            5,
        )
        .unwrap();
    let late = remote
        .create_group("PUBLIC late descendant".into(), Some(parent), 5)
        .unwrap()
        .id;
    document
        .confirm_lifecycle(&prepared, &OperationId::new("PUBLIC delete subtree"), 6)
        .unwrap();
    document.merge(&remote).unwrap();
    for object in [
        ObjectId::Group(child),
        ObjectId::Group(late),
        ObjectId::Entry(entry),
    ] {
        assert_eq!(
            document
                .object_states()
                .unwrap()
                .iter()
                .find(|state| state.address.object == object)
                .unwrap()
                .status,
            ObjectStatus::Trashed
        );
    }
}

#[test]
fn three_group_cycle_lifts_only_the_least_ranked_selected_edge_in_any_order() {
    let mut original = Document::new("PUBLIC cycle", 1).unwrap();
    let groups: Vec<_> = ["PUBLIC A", "PUBLIC B", "PUBLIC C"]
        .into_iter()
        .map(|name| original.create_group(name.into(), None, 2).unwrap().id)
        .collect();
    let mut replicas = Vec::new();
    for index in 0..3 {
        let mut replica = original.fork();
        replica
            .move_group(
                &GroupMove {
                    group: groups[index].clone(),
                    parent: Some(groups[(index + 1) % 3].clone()),
                    position: SiblingPosition::Last,
                    name: None,
                    review: None,
                },
                &OperationId::new(format!("PUBLIC edge {index}")),
                3,
            )
            .unwrap();
        replicas.push(replica);
    }
    let mut expected = None;
    for order in [[0, 1, 2], [2, 1, 0], [1, 2, 0]] {
        let mut merged = original.fork();
        for index in order {
            merged.merge(&replicas[index]).unwrap();
        }
        let tree = merged.groups().unwrap();
        assert_eq!(tree.len(), 3);
        assert_eq!(
            tree.iter().filter(|group| group.parent.is_none()).count(),
            1
        );
        let least = groups
            .iter()
            .map(|group| {
                let address =
                    objects::single(&merged.doc, &ObjectId::Group(group.clone())).unwrap();
                let object = objects::generation_object(&merged.doc, &address).unwrap();
                (
                    merged
                        .doc
                        .get_all(object, "placement")
                        .unwrap()
                        .into_iter()
                        .map(|(_, rank)| rank)
                        .max()
                        .unwrap(),
                    group,
                )
            })
            .min_by(|left, right| left.0.cmp(&right.0))
            .unwrap()
            .1;
        assert_eq!(
            &tree.iter().find(|group| group.parent.is_none()).unwrap().id,
            least
        );
        if let Some(expected) = &expected {
            assert_eq!(&tree, expected);
        } else {
            expected = Some(tree);
        }
    }
}

#[test]
fn true_orphan_projection_retains_its_original_edge_without_creating_resolution_changes() {
    let (mut document, group, _) = fixture();
    let address = objects::single(&document.doc, &ObjectId::Group(group.clone())).unwrap();
    let node = groups::read_group(&document.doc, &address).unwrap();
    let mut tx = document.doc.transaction();
    let object = objects::generation_object(&tx, &address).unwrap();
    groups::put_placement(
        &mut tx,
        &object,
        &GroupPlacement {
            parent: Some(GroupRef {
                id: GroupId::new("PUBLIC absent parent"),
                generation: GenerationId::new("PUBLIC absent parent"),
            }),
            order: node.placement.order,
        },
        4,
    )
    .unwrap();
    tx.commit();
    let before = document.heads();
    assert_eq!(document.groups().unwrap()[0].parent, None);
    assert!(document.tree().unwrap()[0].placements[0].parent.is_some());
    assert_eq!(document.heads(), before);
    assert_eq!(
        Document::load(&document.export())
            .unwrap()
            .groups()
            .unwrap(),
        document.groups().unwrap()
    );
}

#[test]
fn atomic_new_group_forms_and_metadata_receipts_have_one_version_and_exact_causal_heads() {
    // Fresh branch identities exercise the actor ordering that previously exposed MissingOps.
    for _ in 0..16 {
        let mut document = Document::new("PUBLIC metadata receipts", 1).unwrap();
        let operation = OperationId::new("PUBLIC complete new group");
        let fingerprint = serde_json::json!("PUBLIC digest");
        let receipt = CommandReceipt {
            operation: &operation,
            kind: "PUBLIC create metadata",
            fingerprint: &fingerprint,
        };
        let reserved = GroupId::new("PUBLIC reserved group");
        let group = document
            .create_group_metadata_with_id_command(
                reserved.clone(),
                "PUBLIC complete name".into(),
                Some("PUBLIC complete description".into()),
                IconRef::Default,
                None,
                2,
                Some(&receipt),
            )
            .unwrap();
        assert_eq!(group.id, reserved);
        assert_eq!(document.group_history(&reserved).unwrap().len(), 1);
        let acknowledged = document.command_heads(&operation, receipt.kind).unwrap();
        let mut remote = document.fork();
        remote
            .rename_group(&reserved, "PUBLIC remote name".into(), 3)
            .unwrap();
        document.merge(&remote).unwrap();
        assert_eq!(
            document.command_heads(&operation, receipt.kind).unwrap(),
            acknowledged
        );
        assert_ne!(document.heads(), acknowledged);
        assert_eq!(
            document.command_heads(&operation, "PUBLIC other command"),
            Err(Error::DuplicateId)
        );
        let revisions: BTreeSet<_> = document
            .database_history()
            .unwrap()
            .into_iter()
            .map(|revision| revision.id)
            .collect();
        let purge = OperationId::new("PUBLIC purge metadata versions");
        document
            .purge_database_history(revisions.clone(), &purge)
            .unwrap();
        document.purge_database_history(revisions, &purge).unwrap();
        document
            .update_metadata_at("PUBLIC later name".into(), None, 4)
            .unwrap();
        assert_eq!(
            Document::load(&document.export())
                .unwrap()
                .database_history()
                .unwrap()
                .len(),
            1
        );
    }
}
