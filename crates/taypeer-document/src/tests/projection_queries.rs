//! PUBLIC list-query clock and lifecycle regressions; reads never publish document changes.
use super::*;
use taypeer_core::OperationId;

fn projected(document: &Document, entry: &EntryId) -> EntrySnapshot {
    document
        .entries()
        .unwrap()
        .into_iter()
        .find(|row| &row.id == entry)
        .unwrap()
}

#[test]
fn list_projection_keeps_causal_clock_skew_and_refreshes_after_later_input() {
    let (mut document, group, entry) = setup();
    let mut future = document.begin_edit_entry(&entry).unwrap();
    future.fields_mut().password = Some("PUBLIC overwritten future-clock password".into());
    let removed = future.add_attribute(
        "PUBLIC removed attribute".into(),
        "PUBLIC removed protected value".into(),
        true,
    );
    document.save_entry(future, 90_000).unwrap();
    let mut correction = document.begin_edit_entry(&entry).unwrap();
    correction.fields_mut().password = Some("PUBLIC corrected-clock password".into());
    correction.fields_mut().attributes.remove(&removed);
    document.save_entry(correction, 2_000).unwrap();

    let mut unrelated = Vec::new();
    for index in 0..12 {
        let mut draft = document.begin_create_entry(group.clone()).unwrap();
        draft.fields_mut().title = format!("PUBLIC unrelated query row {index}");
        unrelated.push(document.save_entry(draft, 200_000 + index).unwrap());
    }
    let before = document.export();
    let row = projected(&document, &entry);
    assert_eq!(row.modified_at, 2_000);
    assert_eq!(document.entry(&entry).unwrap(), row);
    assert_eq!(document.entries().unwrap().len(), 13);
    assert_eq!(projected(&document, &unrelated[11]).modified_at, 200_011);
    assert_eq!(document.export(), before);

    let mut peer = document.fork();
    let mut local = document.begin_edit_entry(&entry).unwrap();
    local.fields_mut().notes = Some("PUBLIC local concurrent note".into());
    document.save_entry(local, 5_000).unwrap();
    let mut remote = peer.begin_edit_entry(&entry).unwrap();
    remote.fields_mut().notes = Some("PUBLIC peer concurrent note".into());
    peer.save_entry(remote, -5_000).unwrap();
    document.merge(&peer).unwrap();
    let before = document.export();
    let row = projected(&document, &entry);
    assert!(row.has_conflicts());
    assert_eq!(document.entry(&entry).unwrap(), row);
    let notes = row
        .values
        .iter()
        .find(|field| field.field == EntryField::Notes)
        .unwrap();
    assert_eq!(notes.variants.len(), 2);
    assert_eq!(row.modified_at, 5_000);
    assert!(row.fields.unwrap().attributes.is_empty());
    assert_eq!(document.history(&entry).unwrap().len(), 5);
    assert_eq!(document.export(), before);

    // A subsequent query gets a new metadata index; neither older variants nor a prior
    // query's result may hide a freshly acknowledged source operation.
    let mut later = document.begin_edit_entry(&entry).unwrap();
    later.fields_mut().username = Some("PUBLIC later query input".into());
    document.save_entry(later, 7_000).unwrap();
    let reloaded = Document::load(&document.export()).unwrap();
    assert_eq!(projected(&reloaded, &entry).modified_at, 7_000);
    assert_eq!(projected(&reloaded, &unrelated[11]).modified_at, 200_011);
}

#[test]
fn list_projection_keeps_placement_and_closed_generation_rules_after_recovery() {
    let (mut document, group, moved) = setup();
    let destination = document
        .create_group("PUBLIC query destination".into(), None, 2_000)
        .unwrap()
        .id;
    let mut draft = document.begin_create_entry(group).unwrap();
    draft.fields_mut().title = "PUBLIC closed query entry".into();
    draft.fields_mut().password = Some("PUBLIC original closed query password".into());
    let closed = document.save_entry(draft, 1_000).unwrap();
    document
        .move_entry(
            &moved,
            destination.clone(),
            None,
            &OperationId::new("PUBLIC query move"),
            4_000,
        )
        .unwrap();
    let mut late = document.fork();
    let mut unseen = late.begin_edit_entry(&closed).unwrap();
    unseen.fields_mut().password = Some("PUBLIC first late closed query password".into());
    late.save_entry(unseen, 90_000).unwrap();

    for (action, operation, time) in [
        (LifecycleAction::Trash, "PUBLIC query trash", 5_000),
        (LifecycleAction::Purge, "PUBLIC query purge", 6_000),
    ] {
        let prepared = document
            .prepare_lifecycle(action, ObjectId::Entry(closed.clone()), None)
            .unwrap();
        document
            .confirm_lifecycle(&prepared, &OperationId::new(operation), time)
            .unwrap();
    }
    document.merge(&late).unwrap();
    let before = document.export();
    let rows = document.entries().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, moved);
    assert_eq!(rows[0].group_id, destination);
    assert_eq!(rows[0].modified_at, 4_000);
    assert_eq!(document.export(), before);

    let source = document.pending_sources().unwrap().pop().unwrap();
    let request = RecoveryRequest {
        icon: None,
        source: source.id,
        mode: RecoveryMode::Restore,
        destination: Some(destination.clone()),
        name: None,
        fields: None,
    };
    document
        .recover_source(&request, &OperationId::new("PUBLIC query recover"), 7_000)
        .unwrap();
    let restored = projected(&document, &closed);
    assert_ne!(restored.generation, source.address.generation);
    assert_eq!(restored.modified_at, 7_000);
    assert_eq!(restored.group_id, destination);
    assert_eq!(document.entries().unwrap().len(), 2);
    assert_eq!(document.history(&closed).unwrap().len(), 1);

    let mut later = late.begin_edit_entry(&closed).unwrap();
    later.fields_mut().password = Some("PUBLIC second late closed query password".into());
    late.save_entry(later, 800_000).unwrap();
    document.merge(&late).unwrap();
    let reloaded = Document::load(&document.export()).unwrap();
    let before = reloaded.export();
    let current = projected(&reloaded, &closed);
    assert_eq!(current.generation, restored.generation);
    assert_eq!(current.modified_at, 7_000);
    assert_eq!(reloaded.entry(&closed).unwrap(), current);
    assert_eq!(projected(&reloaded, &moved).modified_at, 4_000);
    assert_eq!(reloaded.pending_sources().unwrap().len(), 1);
    assert_eq!(reloaded.export(), before);
}
