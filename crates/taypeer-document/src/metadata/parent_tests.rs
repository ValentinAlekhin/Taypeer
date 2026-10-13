//! PUBLIC group forms exercise one atomic placement/text/appearance revision.
use super::*;

fn group(document: &Document, id: &GroupId) -> Group {
    document
        .groups()
        .unwrap()
        .into_iter()
        .find(|group| &group.id == id)
        .unwrap()
}

fn transition(document: &mut Document, action: LifecycleAction, id: &GroupId, operation: &str) {
    let selection = document
        .prepare_lifecycle(action, ObjectId::Group(id.clone()), None)
        .unwrap();
    document
        .confirm_lifecycle(&selection, &OperationId::new(operation), 10)
        .unwrap();
}

#[test]
fn group_parent_name_description_and_icon_share_one_atomic_history_and_order() {
    let mut document = Document::new("PUBLIC parent form", 1).unwrap();
    let parent = document
        .create_group("PUBLIC parent".into(), None, 2)
        .unwrap()
        .id;
    let sibling = document
        .create_group("PUBLIC sibling".into(), Some(parent.clone()), 3)
        .unwrap()
        .id;
    let source = document
        .create_group("PUBLIC source".into(), None, 4)
        .unwrap()
        .id;
    let heads = document.heads();
    let operation = OperationId::new("PUBLIC combined parent Save");
    let fingerprint = serde_json::json!({"PUBLIC": "atomic parent form"});
    let receipt = CommandReceipt {
        operation: &operation,
        kind: "PUBLIC parent Save",
        fingerprint: &fingerprint,
    };
    let icon = IconRef::Lucide("folder".to_owned().try_into().unwrap());
    let saved = document
        .update_group_metadata_command(
            &source,
            &GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC renamed source".into()),
                description: FieldUpdate::Set("  PUBLIC description\nТекст  ".into()),
                icon: FieldUpdate::Set(icon.clone()),
                parent: FieldUpdate::Set(parent.clone()),
            },
            &heads,
            20,
            Some(&receipt),
        )
        .unwrap();
    assert_eq!(saved.id, source);
    assert_eq!(saved.parent.as_ref(), Some(&parent));
    assert_eq!(saved.name, "PUBLIC renamed source");
    assert!(saved.icon == icon);
    let sibling = group(&document, &sibling);
    assert!(
        sibling
            .order
            .compare(sibling.id.as_str(), &saved.order, saved.id.as_str())
            .is_lt()
    );
    let history = document.group_history(&source).unwrap();
    assert_eq!(history.len(), 2);
    let revision = history
        .iter()
        .find(|revision| revision.saved_at == 20)
        .unwrap();
    assert_eq!(revision.kind, RevisionKind::Save);
    assert_eq!(revision.snapshot.group.parent.as_ref(), Some(&parent));
    assert_eq!(
        revision.snapshot.description.as_deref(),
        Some("  PUBLIC description\nТекст  ")
    );
    assert!(revision.snapshot.group.icon == icon);
    assert_eq!(
        document
            .command_receipt(&operation, "PUBLIC parent Save")
            .unwrap()
            .unwrap()
            .0,
        fingerprint
    );
    let reloaded = Document::load(&document.export()).unwrap();
    assert_eq!(group(&reloaded, &source), saved);
}

#[test]
fn selecting_the_same_parent_or_keep_never_reorders_or_creates_history() {
    let mut document = Document::new("PUBLIC no reorder", 1).unwrap();
    let parent = document
        .create_group("PUBLIC parent".into(), None, 2)
        .unwrap()
        .id;
    let source = document
        .create_group("PUBLIC first child".into(), Some(parent.clone()), 3)
        .unwrap()
        .id;
    document
        .create_group("PUBLIC later sibling".into(), Some(parent.clone()), 4)
        .unwrap();
    for update in [FieldUpdate::Keep, FieldUpdate::Set(parent.clone())] {
        let before = document.export();
        let original = group(&document, &source);
        let saved = document
            .update_group_metadata_command(
                &source,
                &GroupMetadataPatch {
                    parent: update,
                    ..Default::default()
                },
                &document.heads(),
                10,
                None,
            )
            .unwrap();
        assert_eq!(saved, original);
        assert_eq!(document.export(), before);
        assert_eq!(document.group_history(&source).unwrap().len(), 1);
    }
}

#[test]
fn cycles_and_recovered_destinations_refuse_the_whole_metadata_patch() {
    let mut document = Document::new("PUBLIC parent guards", 1).unwrap();
    let source = document
        .create_group("PUBLIC source".into(), None, 2)
        .unwrap()
        .id;
    let child = document
        .create_group("PUBLIC child".into(), Some(source.clone()), 3)
        .unwrap()
        .id;
    let destination = document
        .create_group("PUBLIC destination".into(), None, 4)
        .unwrap()
        .id;
    let heads = document.heads();
    let candidates = document
        .group_parent_candidates(Some(&source), &heads)
        .unwrap();
    assert_eq!(
        candidates.iter().map(|group| &group.id).collect::<Vec<_>>(),
        vec![&destination]
    );
    for parent in [source.clone(), child] {
        let before = document.export();
        assert_eq!(
            document
                .update_group_metadata_command(
                    &source,
                    &GroupMetadataPatch {
                        name: FieldUpdate::Set("PUBLIC must not partially apply".into()),
                        parent: FieldUpdate::Set(parent),
                        ..Default::default()
                    },
                    &heads,
                    10,
                    None
                )
                .err(),
            Some(Error::InvalidContext)
        );
        assert_eq!(document.export(), before);
    }
    let mut late = document.fork();
    late.rename_group(&destination, "PUBLIC late destination".into(), 5)
        .unwrap();
    transition(
        &mut document,
        LifecycleAction::Trash,
        &destination,
        "PUBLIC trash destination",
    );
    transition(
        &mut document,
        LifecycleAction::Purge,
        &destination,
        "PUBLIC purge destination",
    );
    document.merge(&late).unwrap();
    let source_event = document
        .pending_sources()
        .unwrap()
        .into_iter()
        .find(|source| source.address.object == ObjectId::Group(destination.clone()))
        .unwrap();
    document
        .recover_source(
            &RecoveryRequest {
                source: source_event.id,
                mode: RecoveryMode::Restore,
                destination: None,
                name: None,
                fields: None,
                icon: None,
            },
            &OperationId::new("PUBLIC recover destination"),
            20,
        )
        .unwrap();
    assert!(
        document
            .groups()
            .unwrap()
            .iter()
            .any(|group| group.id == destination)
    );
    assert!(
        document
            .group_parent_candidates(Some(&source), &heads)
            .unwrap()
            .is_empty()
    );
    let before = document.export();
    assert_eq!(
        document
            .update_group_metadata_command(
                &source,
                &GroupMetadataPatch {
                    name: FieldUpdate::Set("PUBLIC must not redirect generation".into()),
                    parent: FieldUpdate::Set(destination),
                    ..Default::default()
                },
                &heads,
                30,
                None
            )
            .err(),
        Some(Error::InvalidContext)
    );
    assert_eq!(document.export(), before);
    assert_eq!(document.group_history(&source).unwrap().len(), 1);
}

#[test]
fn addressed_parent_save_preserves_unseen_text_and_keep_preserves_unseen_placement() {
    let mut document = Document::new("PUBLIC causal parent", 1).unwrap();
    let source = document
        .create_group("PUBLIC source".into(), None, 2)
        .unwrap()
        .id;
    let destination = document
        .create_group("PUBLIC destination".into(), None, 3)
        .unwrap()
        .id;
    let heads = document.heads();
    let mut incoming = document.fork();
    incoming
        .set_group_description(&source, Some("PUBLIC incoming description".into()), 4)
        .unwrap();
    incoming
        .rename_group(&destination, "PUBLIC newer destination label".into(), 5)
        .unwrap();
    document.merge(&incoming).unwrap();
    assert_eq!(
        document
            .group_parent_candidates(Some(&source), &heads)
            .unwrap()[0]
            .name,
        "PUBLIC destination"
    );
    document
        .update_group_metadata_command(
            &source,
            &GroupMetadataPatch {
                name: FieldUpdate::Set("PUBLIC local name".into()),
                parent: FieldUpdate::Set(destination.clone()),
                ..Default::default()
            },
            &heads,
            6,
            None,
        )
        .unwrap();
    assert_eq!(
        document.group_description(&source).unwrap().as_deref(),
        Some("PUBLIC incoming description")
    );
    assert_eq!(
        group(&document, &source).parent.as_ref(),
        Some(&destination)
    );
    let later_heads = document.heads();
    document
        .move_group(
            &GroupMove {
                group: source.clone(),
                parent: None,
                position: SiblingPosition::Last,
                review: None,
                name: None,
            },
            &OperationId::new("PUBLIC incoming move"),
            7,
        )
        .unwrap();
    document
        .update_group_metadata_command(
            &source,
            &GroupMetadataPatch {
                description: FieldUpdate::Set("PUBLIC later addressed text".into()),
                ..Default::default()
            },
            &later_heads,
            8,
            None,
        )
        .unwrap();
    assert_eq!(group(&document, &source).parent, None);
    assert_eq!(
        document.group_description(&source).unwrap().as_deref(),
        Some("PUBLIC later addressed text")
    );
}
