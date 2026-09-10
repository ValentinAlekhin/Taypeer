//! Public synthetic reference scenarios; these do not exercise network admission.
use super::*;
use taypeer_core::{Color, OperationId};

fn fixture() -> (Document, GroupId, EntryId, AttachmentId) {
    let mut doc = Document::new("PUBLIC binaries", 1).unwrap();
    let group = doc.create_group("PUBLIC group".into(), None, 2).unwrap().id;
    let mut draft = doc.begin_create_entry(group.clone()).unwrap();
    draft.fields_mut().title = "PUBLIC entry".into();
    let attachment = draft.add_attachment("PUBLIC file".into(), BlobId::new("PUBLIC content A"));
    let entry = doc.save_entry(draft, 3).unwrap();
    (doc, group, entry, attachment)
}

#[test]
fn concurrent_replacement_rename_and_deletion_preserve_variants_and_references() {
    let (mut left, _, entry, attachment) = fixture();
    let mut right = left.fork();
    let mut a = left.begin_edit_entry(&entry).unwrap();
    a.fields_mut()
        .attachments
        .get_mut(&attachment)
        .unwrap()
        .name = "PUBLIC renamed".into();
    left.save_entry(a, 4).unwrap();
    let mut b = right.begin_edit_entry(&entry).unwrap();
    b.fields_mut()
        .attachments
        .get_mut(&attachment)
        .unwrap()
        .blob = BlobId::new("PUBLIC content B");
    right.save_entry(b, 4).unwrap();
    left.merge(&right).unwrap();
    let fields = left.entry(&entry).unwrap().fields.unwrap();
    assert_eq!(fields.attachments[&attachment].name, "PUBLIC renamed");
    assert_eq!(
        fields.attachments[&attachment].blob.as_str(),
        "PUBLIC content B"
    );
    let mut deleted = left.fork();
    let mut replaced = left.fork();
    let mut a = deleted.begin_edit_entry(&entry).unwrap();
    a.fields_mut().attachments.clear();
    deleted.save_entry(a, 5).unwrap();
    let mut b = replaced.begin_edit_entry(&entry).unwrap();
    b.fields_mut()
        .attachments
        .get_mut(&attachment)
        .unwrap()
        .blob = BlobId::new("PUBLIC content C");
    replaced.save_entry(b, 5).unwrap();
    deleted.merge(&replaced).unwrap();
    assert!(deleted.entry(&entry).unwrap().has_conflicts());
    let refs = deleted.blob_references().unwrap();
    for id in ["PUBLIC content A", "PUBLIC content B", "PUBLIC content C"] {
        assert!(refs.attachments.contains(&BlobId::new(id)));
    }
    assert_eq!(deleted.history(&entry).unwrap().len(), 5);
}

#[test]
fn clone_renews_attachment_ids_restore_preserves_them_and_receipts_are_atomic() {
    let (mut doc, group, entry, attachment) = fixture();
    let first = doc.history(&entry).unwrap()[0].id.clone();
    let clone = doc
        .clone_entry(
            &entry,
            group.clone(),
            None,
            &OperationId::new("PUBLIC clone"),
            4,
        )
        .unwrap();
    let fields = doc.entry(&clone).unwrap().fields.unwrap();
    assert!(!fields.attachments.contains_key(&attachment));
    assert_eq!(
        fields.attachments.values().next().unwrap().blob.as_str(),
        "PUBLIC content A"
    );
    let mut draft = doc.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().attachments.clear();
    doc.save_entry(draft, 5).unwrap();
    doc.restore_revision(
        &entry,
        &first,
        group,
        &OperationId::new("PUBLIC restore"),
        6,
    )
    .unwrap();
    assert!(
        doc.entry(&entry)
            .unwrap()
            .fields
            .unwrap()
            .attachments
            .contains_key(&attachment)
    );
    let mut draft = doc.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().appearance.foreground = Some(Color([1, 2, 3, 255]));
    let before = doc.doc.get_changes(&[]).len();
    let intent = serde_json::json!({"public_action":"color"});
    let operation = OperationId::new("PUBLIC binary commit");
    doc.save_binary_entry(draft, &operation, &intent, 7)
        .unwrap();
    assert_eq!(doc.doc.get_changes(&[]).len(), before + 1);
    assert!(doc.binary_receipt(&operation, &intent).unwrap().is_some());
    assert!(
        doc.binary_receipt(&operation, &serde_json::json!({"other":true}))
            .is_err()
    );
}

#[test]
fn purged_history_releases_old_blobs_but_unprocessed_late_sources_retain_theirs() {
    let (mut doc, _, entry, attachment) = fixture();
    let original = doc.history(&entry).unwrap()[0].id.clone();
    let mut draft = doc.begin_edit_entry(&entry).unwrap();
    draft
        .fields_mut()
        .attachments
        .get_mut(&attachment)
        .unwrap()
        .blob = BlobId::new("PUBLIC content B");
    doc.save_entry(draft, 4).unwrap();
    doc.purge_history(
        &entry,
        BTreeSet::from([original]),
        &OperationId::new("PUBLIC purge history"),
    )
    .unwrap();
    assert!(
        !doc.blob_references()
            .unwrap()
            .retained
            .contains(&BlobId::new("PUBLIC content A"))
    );
    let mut offline = doc.fork();
    let target = ObjectId::Entry(entry.clone());
    let trash = doc
        .prepare_lifecycle(LifecycleAction::Trash, target.clone(), None)
        .unwrap();
    doc.confirm_lifecycle(&trash, &OperationId::new("PUBLIC trash"), 5)
        .unwrap();
    let purge = doc
        .prepare_lifecycle(LifecycleAction::Purge, target, None)
        .unwrap();
    doc.confirm_lifecycle(&purge, &OperationId::new("PUBLIC purge"), 6)
        .unwrap();
    assert!(doc.blob_references().unwrap().retained.is_empty());
    let mut draft = offline.begin_edit_entry(&entry).unwrap();
    draft
        .fields_mut()
        .attachments
        .get_mut(&attachment)
        .unwrap()
        .blob = BlobId::new("PUBLIC late content");
    offline.save_entry(draft, 7).unwrap();
    doc.merge(&offline).unwrap();
    let refs = doc.blob_references().unwrap();
    assert!(refs.retained.contains(&BlobId::new("PUBLIC late content")));
    assert!(refs.attachments.is_empty());
    assert!(refs.required.is_empty());
}

#[test]
fn attachment_identity_cannot_be_borrowed_from_another_entry() {
    let (mut doc, group, entry, _) = fixture();
    let mut draft = doc.begin_create_entry(group).unwrap();
    *draft.fields_mut() = doc.entry(&entry).unwrap().fields.unwrap();
    assert_eq!(doc.save_entry(draft, 5), Err(Error::DuplicateId));
}

#[test]
fn appearance_fields_merge_independently_and_group_icon_review_preserves_unseen_values() {
    let (mut left, group, entry, _) = fixture();
    let icon = |key: &str| IconRef::Lucide(key.to_owned().try_into().unwrap());
    let mut right = left.fork();
    let mut draft = left.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().appearance.icon = icon("key-round");
    draft.fields_mut().appearance.foreground = Some(Color([10, 20, 30, 255]));
    left.save_entry(draft, 4).unwrap();
    let mut draft = right.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().appearance.background = Some(Color([40, 50, 60, 255]));
    right.save_entry(draft, 4).unwrap();
    left.merge(&right).unwrap();
    let appearance = left.entry(&entry).unwrap().fields.unwrap().appearance;
    assert!(appearance.icon == icon("key-round"));
    assert_eq!(appearance.foreground, Some(Color([10, 20, 30, 255])));
    assert_eq!(appearance.background, Some(Color([40, 50, 60, 255])));
    let intent = serde_json::json!({"PUBLIC": "group icon"});
    let mut right = left.fork();
    left.set_group_icon(
        &group,
        icon("folder"),
        None,
        &OperationId::new("PUBLIC a"),
        &intent,
        5,
    )
    .unwrap();
    right
        .set_group_icon(
            &group,
            icon("star"),
            None,
            &OperationId::new("PUBLIC b"),
            &intent,
            6,
        )
        .unwrap();
    left.merge(&right).unwrap();
    let group_icons = |doc: &Document| {
        doc.tree()
            .unwrap()
            .into_iter()
            .find(|node| node.address.object == ObjectId::Group(group.clone()))
            .unwrap()
            .icons
    };
    assert_eq!(group_icons(&left).len(), 2);
    let heads = left.review_heads();
    let mut unseen = left.fork();
    unseen
        .set_group_icon(
            &group,
            icon("heart"),
            None,
            &OperationId::new("PUBLIC unseen"),
            &intent,
            8,
        )
        .unwrap();
    left.set_group_icon(
        &group,
        icon("folder-open"),
        Some(&heads),
        &OperationId::new("PUBLIC review"),
        &intent,
        7,
    )
    .unwrap();
    left.merge(&unseen).unwrap();
    let icons = group_icons(&left);
    assert_eq!(icons.len(), 2);
    assert!(icons.contains(&icon("heart")));
    assert!(icons.contains(&icon("folder-open")));
}
