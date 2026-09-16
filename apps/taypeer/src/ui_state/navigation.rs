//! Window navigation and projections over loaded service results.

use super::{CatalogStore, DatabaseId, EntryId, GroupId, RevisionId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum EntryTab {
    #[default]
    Overview,
    Advanced,
    Appearance,
    Properties,
    History,
}
impl EntryTab {
    pub const ALL: [Self; 5] = [
        Self::Overview,
        Self::Advanced,
        Self::Appearance,
        Self::Properties,
        Self::History,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Advanced => "ui.advanced",
            Self::Appearance => "ui.appearance",
            Self::Properties => "ui.properties",
            Self::History => "history",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchScope {
    Current,
    AllUnlocked,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    Title,
    Username,
    Url,
    Notes,
    Modified,
    Location,
}
impl Column {
    pub const ALL: [Self; 6] = [
        Self::Title,
        Self::Username,
        Self::Url,
        Self::Notes,
        Self::Modified,
        Self::Location,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "name",
            Self::Username => "username",
            Self::Url => "url",
            Self::Notes => "notes",
            Self::Modified => "ui.modified",
            Self::Location => "ui.location",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Welcome,
    Workspace,
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    Database(DatabaseId),
    Group(GroupId),
    Entry(DatabaseId, EntryId),
    ClearEntry,
    NewEntry,
    CreateDatabase,
    GroupForm {
        id: Option<GroupId>,
        parent: Option<GroupId>,
    },
    CloneGroup,
    TrashGroup,
    CloseDatabase,
    CancelEdit,
    SearchResults,
    RestoreRevision(RevisionId),
    ClearHistory,
    Quit,
}

#[derive(Clone)]
pub(crate) struct SearchBookmark {
    pub database: Option<DatabaseId>,
    pub group: Option<GroupId>,
    pub query: String,
    pub scope: SearchScope,
}

pub(crate) struct NavigationState {
    pub route: Route,
    pub database: Option<DatabaseId>,
    pub group: Option<GroupId>,
    pub selected: Option<EntryId>,
    pub opened: BTreeSet<DatabaseId>,
    pub unlocked: BTreeSet<DatabaseId>,
    pub suspended: BTreeMap<DatabaseId, taypeer_services::PendingDraftSummary>,
    pub query: String,
    pub scope: SearchScope,
    pub sort: Column,
    pub descending: bool,
    pub columns: Vec<Column>,
    pub tab: EntryTab,
    pub pending: Option<Destination>,
    pub bookmark: Option<SearchBookmark>,
}

impl Default for NavigationState {
    fn default() -> Self {
        Self {
            route: Route::Welcome,
            database: None,
            group: None,
            selected: None,
            opened: BTreeSet::new(),
            unlocked: BTreeSet::new(),
            suspended: BTreeMap::new(),
            query: String::new(),
            scope: SearchScope::Current,
            sort: Column::Title,
            descending: false,
            columns: vec![Column::Title, Column::Username, Column::Location],
            tab: EntryTab::Overview,
            pending: None,
            bookmark: None,
        }
    }
}

impl NavigationState {
    pub fn is_unlocked(&self) -> bool {
        self.database
            .as_ref()
            .is_some_and(|db| self.unlocked.contains(db))
    }
    pub fn select_database(&mut self, db: DatabaseId, catalog: &CatalogStore) {
        let Some(item) = catalog.database(&db) else {
            return;
        };
        self.opened.insert(db.clone());
        self.database = Some(db);
        self.group = item.groups.first().map(|g| g.id.clone());
        self.selected = None;
        self.query.clear();
        self.bookmark = None;
        self.tab = EntryTab::Overview;
        self.route = Route::Workspace;
    }
    pub fn close_database(&mut self, catalog: &CatalogStore) {
        if let Some(db) = self.database.take() {
            self.opened.remove(&db);
            self.unlocked.remove(&db);
            self.suspended.remove(&db);
        }
        if let Some(db) = self.opened.first().cloned() {
            self.select_database(db, catalog);
        } else {
            self.route = Route::Welcome;
            self.group = None;
            self.selected = None;
            self.query.clear();
            self.bookmark = None;
        }
    }
    pub fn lock(&mut self, db: &DatabaseId) {
        self.unlocked.remove(db);
        self.suspended.remove(db);
        if self.database.as_ref() == Some(db) {
            self.selected = None;
            self.query.clear();
            self.bookmark = None;
            self.pending = None;
            self.tab = EntryTab::Overview;
            self.route = Route::Workspace;
        }
    }
    pub fn select_entry(&mut self, db: DatabaseId, id: EntryId, catalog: &CatalogStore) {
        if !self.unlocked.contains(&db) {
            return;
        }
        let Some(entry) = catalog.entry(&db, &id) else {
            return;
        };
        if !self.query.is_empty() && self.bookmark.is_none() {
            self.bookmark = Some(SearchBookmark {
                database: self.database.clone(),
                group: self.group.clone(),
                query: self.query.clone(),
                scope: self.scope,
            });
        }
        self.database = Some(db);
        self.group = entry.group.clone();
        self.selected = Some(id);
        self.tab = EntryTab::Overview;
        if self.bookmark.is_some() {
            self.query.clear();
        }
    }
    pub fn return_to_search(&mut self) {
        if let Some(bookmark) = self.bookmark.take() {
            self.database = bookmark.database;
            self.group = bookmark.group;
            self.query = bookmark.query;
            self.scope = bookmark.scope;
            self.selected = None;
        }
    }
    pub fn sort_by(&mut self, column: Column) {
        self.descending = self.sort == column && !self.descending;
        self.sort = column;
    }
    pub fn move_column(&mut self, from: usize, to: usize) {
        if from == to || from >= self.columns.len() || to >= self.columns.len() {
            return;
        }
        let column = self.columns.remove(from);
        self.columns.insert(to, column);
    }
    pub fn rows(&self, catalog: &CatalogStore) -> Vec<(DatabaseId, EntryId)> {
        let mut rows = Vec::new();
        for db in catalog.databases() {
            if db.query != self.query
                || !self.unlocked.contains(&db.id)
                || ((self.query.is_empty() || self.scope == SearchScope::Current)
                    && self.database.as_ref() != Some(&db.id))
            {
                continue;
            }
            for entry in db.entries.values() {
                // Search membership comes from the service response; never reimplement its matching rules.
                if db.row_ids.contains(&entry.id)
                    && (!self.query.is_empty() || entry.group == self.group)
                {
                    rows.push((db.id.clone(), entry.id.clone()));
                }
            }
        }
        rows.sort_by_cached_key(|(db, id)| {
            let Some(entry) = catalog.entry(db, id) else {
                return String::new();
            };
            match self.sort {
                Column::Title => entry.content.title.to_lowercase(),
                Column::Username => entry.content.username.to_lowercase(),
                Column::Url => entry.content.url.to_lowercase(),
                Column::Notes => entry.content.notes.to_lowercase(),
                Column::Modified => format!("{:020}", entry.modified),
                Column::Location => catalog.group_path(db, entry.group.as_ref()).to_lowercase(),
            }
        });
        if self.descending {
            rows.reverse();
        }
        rows
    }
}
