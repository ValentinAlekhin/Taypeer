//! PUBLIC metadata restores preserve identities, causal alternatives and exact receipts.
use super::*;
use taypeer_core::{FieldUpdate, GroupMetadataPatch};

fn restore_group(
    document: &mut Document,
    request: &GroupRevisionRestore,
    operation: &str,
) -> Result<Group, Error> {
    let operation = OperationId::new(operation);
    let fingerprint = serde_json::to_value(request).unwrap();
    document.restore_group_revision_command(
        request,
        50,
        &CommandReceipt {
            operation: &operation,
            kind: "restore_group_revision",
            fingerprint: &fingerprint,
        },
    )
}
fn restore_database(
    document: &mut Document,
    request: &DatabaseRevisionRestore,
    operation: &str,
) -> Result<(), Error> {
    let operation = OperationId::new(operation);
    let fingerprint = serde_json::to_value(request).unwrap();
    document.restore_database_revision_command(
        request,
        60,
        &CommandReceipt {
            operation: &operation,
            kind: "restore_database_revision",
            fingerprint: &fingerprint,
        },
    )
}
fn trash(document: &mut Document, id: &GroupId, operation: &str) {
    let review = document
        .prepare_lifecycle(LifecycleAction::Trash, ObjectId::Group(id.clone()), None)
        .unwrap();
    document
        .confirm_lifecycle(&review, &OperationId::new(operation), 40)
        .unwrap();
}

#[test]
fn group_snapshot_restore_is_one_confirmation_of_all_fields_with_explicit_parent() {
    let mut doc = Document::new("PUBLIC history database", 1).unwrap();
    let parent = doc
        .create_group("PUBLIC destination".into(), None, 2)
        .unwrap()
        .id;
    let group = doc
        .create_group("PUBLIC saved group".into(), None, 3)
        .unwrap()
        .id;
    let icon = IconRef::Lucide("folder".to_owned().try_into().unwrap());
    doc.update_group_metadata_command(
        &group,
        &GroupMetadataPatch {
            description: FieldUpdate::Set("  PUBLIC original\nОписание  ".into()),
            icon: FieldUpdate::Set(icon.clone()),
            ..Default::default()
        },
        &doc.heads(),
        4,
        None,
    )
    .unwrap();
    let original = doc.group_history(&group).unwrap().pop().unwrap();
    doc.update_group_metadata_command(
        &group,
        &GroupMetadataPatch {
            name: FieldUpdate::Set("PUBLIC current group".into()),
            description: FieldUpdate::Clear,
            icon: FieldUpdate::Clear,
            ..Default::default()
        },
        &doc.heads(),
        5,
        None,
    )
    .unwrap();
    let before = doc.export();
    let viewed = doc.group_revision(&group, &original.id).unwrap();
    assert_eq!(viewed, original);
    assert_eq!(
        doc.group_snapshot(&group).unwrap().group.name,
        "PUBLIC current group"
    );
    assert_eq!(doc.export(), before);
    let count = doc.group_history(&group).unwrap().len();
    let request = GroupRevisionRestore {
        group: group.clone(),
        revision: original.id.clone(),
        parent: Some(parent.clone()),
        heads: doc.heads(),
    };
    let restored = restore_group(&mut doc, &request, "PUBLIC restore all metadata").unwrap();
    assert_eq!(restored.id, group);
    assert_eq!(restored.parent, Some(parent));
    assert_eq!(restored.name, original.snapshot.group.name);
    assert_eq!(restored.icon, icon);
    assert_eq!(
        doc.group_description(&group).unwrap(),
        original.snapshot.description
    );
    let revisions = doc.group_history(&group).unwrap();
    assert_eq!(revisions.len(), count + 1);
    assert_eq!(revisions.last().unwrap().kind, RevisionKind::Restore);
    assert_eq!(doc.group_revision(&group, &original.id).unwrap(), original);
    assert_eq!(
        Document::load(&doc.export())
            .unwrap()
            .group_snapshot(&group)
            .unwrap(),
        doc.group_snapshot(&group).unwrap()
    );
}

#[test]
fn equal_restore_records_once_without_reordering_and_exact_retry_survives_later_edit_and_purge() {
    let mut doc = Document::new("PUBLIC replay history", 1).unwrap();
    let parent = doc
        .create_group("PUBLIC parent".into(), None, 2)
        .unwrap()
        .id;
    let group = doc
        .create_group("PUBLIC first child".into(), Some(parent.clone()), 3)
        .unwrap()
        .id;
    doc.create_group("PUBLIC later sibling".into(), Some(parent.clone()), 4)
        .unwrap();
    let saved = doc.group_history(&group).unwrap().pop().unwrap();
    let order = doc.group_snapshot(&group).unwrap().group.order;
    let request = GroupRevisionRestore {
        group: group.clone(),
        revision: saved.id.clone(),
        parent: Some(parent),
        heads: doc.heads(),
    };
    let result = restore_group(&mut doc, &request, "PUBLIC equal restore").unwrap();
    assert_eq!(result.order, order);
    assert_eq!(doc.group_history(&group).unwrap().len(), 2);
    assert_eq!(
        doc.group_history(&group).unwrap().last().unwrap().kind,
        RevisionKind::Restore
    );
    doc.rename_group(&group, "PUBLIC later name".into(), 70)
        .unwrap();
    doc.purge_group_history(
        &group,
        [saved.id].into_iter().collect(),
        &OperationId::new("PUBLIC purge restore source"),
    )
    .unwrap();
    let before = doc.export();
    let replay = restore_group(&mut doc, &request, "PUBLIC equal restore").unwrap();
    assert_eq!(replay, result);
    assert_eq!(doc.export(), before);
    assert_eq!(
        doc.group_snapshot(&group).unwrap().group.name,
        "PUBLIC later name"
    );
    let mut changed = request;
    changed.parent = None;
    assert_eq!(
        restore_group(&mut doc, &changed, "PUBLIC equal restore").err(),
        Some(Error::DuplicateId)
    );
    assert_eq!(doc.export(), before);
}

#[test]
fn invalid_or_closed_destinations_and_cycles_refuse_all_restored_fields_atomically() {
    let mut doc = Document::new("PUBLIC history guards", 1).unwrap();
    let group = doc
        .create_group("PUBLIC source".into(), None, 2)
        .unwrap()
        .id;
    let child = doc
        .create_group("PUBLIC child".into(), Some(group.clone()), 3)
        .unwrap()
        .id;
    let destination = doc
        .create_group("PUBLIC destination".into(), None, 4)
        .unwrap()
        .id;
    let saved = doc.group_history(&group).unwrap().pop().unwrap();
    doc.rename_group(&group, "PUBLIC retained current input".into(), 5)
        .unwrap();
    let heads = doc.heads();
    let candidates = doc.group_parent_candidates(Some(&group), &heads).unwrap();
    assert_eq!(
        candidates.iter().map(|row| &row.id).collect::<Vec<_>>(),
        vec![&destination]
    );
    for parent in [group.clone(), child, GroupId::new("PUBLIC nonexistent")] {
        let request = GroupRevisionRestore {
            group: group.clone(),
            revision: saved.id.clone(),
            parent: Some(parent),
            heads: heads.clone(),
        };
        let before = doc.export();
        assert!(restore_group(&mut doc, &request, "PUBLIC invalid restore").is_err());
        assert_eq!(doc.export(), before);
    }
    trash(&mut doc, &destination, "PUBLIC closed destination");
    let request = GroupRevisionRestore {
        group: group.clone(),
        revision: saved.id,
        parent: Some(destination),
        heads,
    };
    let before = doc.export();
    assert!(restore_group(&mut doc, &request, "PUBLIC trashed restore").is_err());
    assert_eq!(doc.export(), before);
    assert_eq!(
        doc.group_snapshot(&group).unwrap().group.name,
        "PUBLIC retained current input"
    );
    assert_eq!(doc.group_history(&group).unwrap().len(), 2);
    trash(&mut doc, &group, "PUBLIC closed source");
    let before = doc.export();
    assert!(restore_group(&mut doc, &request, "PUBLIC closed source restore").is_err());
    assert_eq!(doc.export(), before);
}

#[test]
fn database_restore_changes_only_descriptive_fields_and_replay_preserves_later_metadata() {
    let mut doc = Document::new("PUBLIC saved database", 1).unwrap();
    doc.update_metadata_at(
        "PUBLIC saved database".into(),
        Some("  PUBLIC exact\nТекст  ".into()),
        2,
    )
    .unwrap();
    let saved = doc.database_history().unwrap().pop().unwrap();
    doc.update_metadata_at("PUBLIC current database".into(), None, 3)
        .unwrap();
    let before = doc.export();
    assert_eq!(doc.database_revision(&saved.id).unwrap(), saved);
    assert_eq!(doc.export(), before);
    let identity = doc.database_id().clone();
    let request = DatabaseRevisionRestore {
        revision: saved.id.clone(),
        heads: doc.heads(),
    };
    restore_database(&mut doc, &request, "PUBLIC database Restore").unwrap();
    assert_eq!(doc.database_id(), &identity);
    assert_eq!(doc.database_snapshot().unwrap(), saved.snapshot);
    assert_eq!(doc.database_history().unwrap().len(), 4);
    assert_eq!(
        doc.database_history().unwrap().last().unwrap().kind,
        RevisionKind::Restore
    );
    doc.update_metadata_at(
        "PUBLIC latest database".into(),
        Some("PUBLIC latest description".into()),
        70,
    )
    .unwrap();
    doc.purge_database_history(
        [saved.id].into_iter().collect(),
        &OperationId::new("PUBLIC purge database source"),
    )
    .unwrap();
    let before = doc.export();
    restore_database(&mut doc, &request, "PUBLIC database Restore").unwrap();
    assert_eq!(doc.export(), before);
    assert_eq!(doc.display_name().unwrap(), "PUBLIC latest database");
    let equal = DatabaseRevisionRestore {
        revision: doc.database_history().unwrap().last().unwrap().id.clone(),
        heads: doc.heads(),
    };
    let count = doc.database_history().unwrap().len();
    restore_database(&mut doc, &equal, "PUBLIC equal database Restore").unwrap();
    assert_eq!(doc.database_history().unwrap().len(), count + 1);
}

#[test]
fn restore_at_reviewed_heads_preserves_unseen_metadata_alternatives() {
    let mut doc = Document::new("PUBLIC causal history", 1).unwrap();
    let group = doc
        .create_group("PUBLIC original group".into(), None, 2)
        .unwrap()
        .id;
    let saved = doc.group_history(&group).unwrap().pop().unwrap();
    let request = GroupRevisionRestore {
        group: group.clone(),
        revision: saved.id.clone(),
        parent: None,
        heads: doc.heads(),
    };
    let mut incoming = doc.fork();
    incoming
        .set_group_description(&group, Some("PUBLIC unseen description".into()), 10)
        .unwrap();
    incoming
        .rename_group(&group, "PUBLIC unseen name".into(), 11)
        .unwrap();
    doc.merge(&incoming).unwrap();
    restore_group(&mut doc, &request, "PUBLIC causal restore").unwrap();
    let history = doc.group_history(&group).unwrap();
    assert!(
        history
            .iter()
            .any(|revision| revision.snapshot.description.as_deref()
                == Some("PUBLIC unseen description"))
    );
    assert!(
        history
            .iter()
            .any(|revision| revision.snapshot.group.name == "PUBLIC unseen name")
    );
    let restored = history
        .iter()
        .find(|revision| revision.kind == RevisionKind::Restore)
        .unwrap();
    assert_eq!(restored.snapshot.group.name, saved.snapshot.group.name);
    assert_eq!(restored.snapshot.description, saved.snapshot.description);
    assert_eq!(restored.base, request.heads);
}
