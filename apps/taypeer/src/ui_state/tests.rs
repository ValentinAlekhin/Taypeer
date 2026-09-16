use super::*;
use crate::backend::{Query, Snapshot};

fn snapshot(
    database: DatabaseId,
    rows: Vec<taypeer_services::EntrySummary>,
    query: &str,
) -> Snapshot {
    Snapshot {
        query: Query {
            search: query.into(),
            ..Default::default()
        },
        info: taypeer_services::DatabaseInfo {
            id: database,
            name: "PUBLIC database".into(),
            description: None,
            writable: true,
            managing: false,
            metadata_conflict: false,
        },
        groups: Vec::new(),
        previews: Vec::new(),
        binary: None,
        rows,
        entry: None,
        history: Vec::new(),
        pending: None,
        usage: taypeer_services::StorageUsage {
            attachment_bytes: 0,
            attachment_limit: 100,
            over_limit: false,
            retained_bytes: 0,
            draft_bytes: 0,
            file_bytes: 42,
            missing: Vec::new(),
            unknown_references: false,
        },
        policy: Default::default(),
        writable: true,
    }
}
fn row(id: &str) -> taypeer_services::EntrySummary {
    taypeer_services::EntrySummary {
        id: EntryId::new(id),
        group_id: Some(GroupId::new("PUBLIC group")),
        title: "PUBLIC result".into(),
        username: None,
        url: None,
        has_conflicts: false,
        notes: None,
        modified_at: 1000,
        appearance: Default::default(),
    }
}
#[test]
fn only_matching_service_results_from_unlocked_databases_are_visible() {
    let db = DatabaseId::new("PUBLIC db");
    let other = DatabaseId::new("PUBLIC other db");
    let mut catalog = CatalogStore::default();
    catalog.add_path(db.clone(), "PUBLIC.taypeer".into());
    catalog.add_path(other.clone(), "PUBLIC-other.taypeer".into());
    catalog.apply(
        &db,
        snapshot(db.clone(), vec![row("PUBLIC a")], "first query"),
    );
    catalog.apply(
        &other,
        snapshot(other.clone(), vec![row("PUBLIC b")], "second query"),
    );
    let mut nav = NavigationState::default();
    nav.select_database(db.clone(), &catalog);
    nav.unlocked.extend([db.clone(), other.clone()]);
    nav.scope = SearchScope::AllUnlocked;
    nav.query = "second query".into();
    assert_eq!(
        nav.rows(&catalog),
        vec![(other.clone(), EntryId::new("PUBLIC b"))]
    );
    nav.lock(&other);
    catalog.clear(&other);
    assert!(nav.rows(&catalog).is_empty());
    assert!(catalog.database(&other).unwrap().entries.is_empty());
    assert!(!catalog.database(&other).unwrap().writable);
}
#[test]
fn lock_clears_selection_and_pending_navigation_but_keeps_the_file_open() {
    let db = DatabaseId::new("PUBLIC db");
    let mut catalog = CatalogStore::default();
    catalog.add_path(db.clone(), "PUBLIC.taypeer".into());
    let mut nav = NavigationState::default();
    nav.select_database(db.clone(), &catalog);
    nav.unlocked.insert(db.clone());
    nav.selected = Some(EntryId::new("PUBLIC selected"));
    nav.pending = Some(Destination::Quit);
    nav.lock(&db);
    assert!(!nav.is_unlocked());
    assert!(nav.selected.is_none() && nav.pending.is_none());
    assert!(nav.opened.contains(&db));
    nav.close_database(&catalog);
    assert_eq!(nav.route, Route::Welcome);
    assert!(nav.opened.is_empty());
}

#[test]
fn visible_columns_can_be_reordered_without_losing_the_title() {
    let mut nav = NavigationState::default();
    nav.move_column(2, 0);
    assert_eq!(
        nav.columns,
        vec![Column::Location, Column::Title, Column::Username]
    );

    nav.move_column(10, 0);
    nav.move_column(0, 10);
    assert_eq!(
        nav.columns,
        vec![Column::Location, Column::Title, Column::Username]
    );
    assert!(nav.columns.contains(&Column::Title));
}
