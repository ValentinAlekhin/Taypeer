use super::*;

fn first(catalog: &CatalogStore) -> (DatabaseId, EntryId) {
    let db = catalog
        .databases()
        .find(|db| !db.entries.is_empty())
        .unwrap();
    (db.id, *db.entries.keys().next().unwrap())
}

#[test]
fn draft_spans_fields_attributes_and_appearance_without_changing_saved_content() {
    let mut catalog = CatalogStore::samples();
    let (db, id) = first(&catalog);
    let original = catalog.entry(db, id).unwrap().content.clone();
    let mut editor = EditorStore::existing(db, catalog.entry(db, id).unwrap());
    editor.edit(|content| {
        EntryField::Title.set(content, "Changed title".into());
        content.attributes.push(Attribute {
            key: "Extra".into(),
            value: "PUBLIC".into(),
            protected: true,
        });
        content.background = Some(0x123456);
    });
    assert!(editor.dirty());
    assert!(catalog.entry(db, id).unwrap().content == original);
    editor.validate().unwrap();
    catalog
        .save_entry(db, editor.group(), Some(id), editor.content().clone())
        .unwrap();
    assert!(catalog.entry(db, id).unwrap().content == *editor.content());
    let after = catalog.entry(db, id).unwrap().revisions.len();
    catalog
        .save_entry(db, editor.group(), Some(id), editor.content().clone())
        .unwrap();
    assert_eq!(catalog.entry(db, id).unwrap().revisions.len(), after);
}

#[test]
fn failed_form_validation_preserves_input_and_catalog() {
    let catalog = CatalogStore::samples();
    let (db, id) = first(&catalog);
    let mut editor = EditorStore::existing(db, catalog.entry(db, id).unwrap());
    editor.edit(|content| {
        content.title.clear();
        content.notes = "Keep my unfinished input".into();
    });
    assert_eq!(editor.validate(), Err(FormError::RequiredName));
    assert!(editor.content().notes == "Keep my unfinished input");
    assert!(!catalog.entry(db, id).unwrap().content.title.is_empty());
}

#[test]
fn search_excludes_protected_values_and_locked_databases() {
    let catalog = CatalogStore::samples();
    let (db, _) = first(&catalog);
    let mut nav = NavigationState::default();
    nav.select_database(db, &catalog);
    nav.unlocked.insert(db);
    nav.scope = SearchScope::AllUnlocked;
    for query in ["PUBLIC-UI", "PUBLIC-RECOVERY", "PUBLIC-PERSONAL"] {
        nav.query = query.into();
        assert!(nav.rows(&catalog).is_empty());
    }
    nav.query = "Example Studio".into();
    assert_eq!(nav.rows(&catalog).len(), 1);
    nav.query = "long-public-username".into();
    assert!(nav.rows(&catalog).is_empty());
    for database in catalog.databases() {
        nav.unlocked.insert(database.id);
    }
    assert_eq!(nav.rows(&catalog).len(), 1);
}

#[test]
fn sorting_keeps_selection_and_search_result_navigation_has_a_return_path() {
    let catalog = CatalogStore::samples();
    let (db, id) = first(&catalog);
    let mut nav = NavigationState::default();
    nav.select_database(db, &catalog);
    nav.unlocked.insert(db);
    nav.selected = Some(id);
    let initial = nav.rows(&catalog);
    nav.sort_by(Column::Title);
    let mut expected = initial;
    expected.reverse();
    assert_eq!(nav.rows(&catalog), expected);
    assert_eq!(nav.selected, Some(id));
    for database in catalog.databases() {
        nav.unlocked.insert(database.id);
    }
    nav.scope = SearchScope::AllUnlocked;
    nav.query = "long-public-username".into();
    let result = nav.rows(&catalog)[0];
    nav.select_entry(result.0, result.1, &catalog);
    assert_eq!(nav.database, Some(result.0));
    assert!(nav.query.is_empty());
    nav.return_to_search();
    assert_eq!(nav.database, Some(db));
    assert_eq!(nav.query, "long-public-username");
    assert_eq!(nav.rows(&catalog), vec![result]);
}

#[test]
fn lock_hides_selection_and_keeps_only_the_current_database_draft() {
    let catalog = CatalogStore::samples();
    let (db, id) = first(&catalog);
    let mut nav = NavigationState::default();
    nav.select_database(db, &catalog);
    nav.unlocked.insert(db);
    nav.selected = Some(id);
    let mut editor = EditorStore::existing(db, catalog.entry(db, id).unwrap());
    editor.edit(|content| content.notes = "Unsaved UI draft".into());
    nav.lock(Some(editor));
    assert!(!nav.is_unlocked());
    assert!(nav.selected.is_none());
    assert!(nav.rows(&catalog).is_empty());
    let recovered = nav.suspended.remove(&db).unwrap();
    assert!(recovered.content().notes == "Unsaved UI draft");
    assert!(catalog.entry(db, id).unwrap().content.notes != "Unsaved UI draft");
    nav.unlocked.insert(db);
    assert!(!nav.rows(&catalog).is_empty());
    nav.close_database(&catalog);
    assert_eq!(nav.route, Route::Welcome);
    assert!(nav.opened.is_empty());
}

#[test]
fn history_restores_content_and_clears_only_the_selected_entry() {
    let mut catalog = CatalogStore::samples();
    let (db, id) = first(&catalog);
    let entry = catalog.entry(db, id).unwrap();
    let group = entry.group;
    let original = entry.content.clone();
    let sequence = entry.revisions[0].sequence;
    let mut changed = original.clone();
    changed.password = "PUBLIC-NEW-VALUE".into();
    catalog.save_entry(db, group, Some(id), changed).unwrap();
    catalog.restore_revision(db, id, sequence).unwrap();
    assert!(catalog.entry(db, id).unwrap().content == original);
    assert_eq!(catalog.entry(db, id).unwrap().revisions.len(), 3);
    let other = catalog
        .database(db)
        .unwrap()
        .entries
        .values()
        .find(|e| e.id != id)
        .unwrap();
    let other_id = other.id;
    let other_count = other.revisions.len();
    catalog.clear_history(db, id).unwrap();
    assert!(catalog.entry(db, id).unwrap().revisions.is_empty());
    assert_eq!(
        catalog.entry(db, other_id).unwrap().revisions.len(),
        other_count
    );
}

#[test]
fn new_database_has_no_implicit_group_and_ids_are_never_reused() {
    let mut catalog = CatalogStore::samples();
    let db = catalog
        .create_database("New demo".into(), String::new())
        .unwrap();
    assert!(catalog.database(db).unwrap().groups.is_empty());
    let group = catalog
        .save_group(
            db,
            None,
            None,
            "Group".into(),
            String::new(),
            "folder".into(),
        )
        .unwrap();
    let a = catalog
        .save_entry(
            db,
            group,
            None,
            EntryContent {
                title: "A".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let b = catalog
        .save_entry(
            db,
            group,
            None,
            EntryContent {
                title: "B".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_ne!(a, b);
    let other = catalog
        .create_database("Other".into(), String::new())
        .unwrap();
    assert_eq!(
        catalog.save_entry(
            other,
            group,
            Some(a),
            EntryContent {
                title: "Bad reference".into(),
                ..Default::default()
            }
        ),
        Err(FormError::MissingObject)
    );
}
