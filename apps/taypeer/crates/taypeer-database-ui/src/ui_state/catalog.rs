//! Masked presentation cache. Confirmed data and validation belong to Rust services.
use std::{collections::BTreeMap, path::PathBuf};
pub(crate) use taypeer_core::{DatabaseId, EntryId, GroupId, RevisionId};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Attribute {
    pub has_value: bool,
    pub id: Option<taypeer_core::AttributeId>,
    pub key: String,
    pub value: String,
    pub protected: bool,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub id: taypeer_core::AttachmentId,
    pub name: String,
    pub bytes: Option<u64>,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct EntryContent {
    pub title: String,
    pub username: String,
    pub password: String,
    pub has_password: bool,
    pub url: String,
    pub tags: String,
    pub notes: String,
    pub expires: String,
    pub attributes: Vec<Attribute>,
    pub attachments: Vec<Attachment>,
    pub icon: String,
    pub icon_blob: Option<taypeer_core::BlobId>,
    pub foreground: Option<u32>,
    pub background: Option<u32>,
}
#[derive(Clone)]
pub(crate) struct Revision {
    pub sequence: RevisionId,
    pub title: String,
    pub saved_at: i64,
    pub content: Option<EntryContent>,
}
#[derive(Clone)]
pub(crate) struct Entry {
    pub id: EntryId,
    pub group: Option<GroupId>,
    pub content: EntryContent,
    pub revisions: Vec<Revision>,
    pub created: i64,
    pub modified: i64,
    pub conflicted: bool,
}
#[derive(Clone)]
pub(crate) struct Group {
    pub entry_count: usize,
    pub source_icon: taypeer_core::IconRef,
    pub description_conflict: bool,
    pub id: GroupId,
    pub parent: Option<GroupId>,
    pub name: String,
    pub description: Option<String>,
    pub icon: String,
    pub icon_blob: Option<taypeer_core::BlobId>,
}
pub(crate) struct Database {
    pub id: DatabaseId,
    pub path: PathBuf,
    pub name: String,
    pub description: Option<String>,
    pub groups: Vec<Group>,
    pub entries: BTreeMap<EntryId, Entry>,
    pub row_ids: std::collections::BTreeSet<EntryId>,
    pub query: String,
    pub file_bytes: u64,
    pub policy: taypeer_core::DatabasePolicy,
    pub writable: bool,
    pub managing: bool,
    pub metadata_conflict: bool,
}
pub(crate) use taypeer_ui::{FormError, require_name};
#[derive(Default)]
pub(crate) struct CatalogStore {
    databases: BTreeMap<DatabaseId, Database>,
    version: u64,
}
impl CatalogStore {
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn databases(&self) -> impl Iterator<Item = &Database> {
        self.databases.values()
    }
    pub fn database(&self, id: &DatabaseId) -> Option<&Database> {
        self.databases.get(id)
    }
    pub fn entry(&self, db: &DatabaseId, id: &EntryId) -> Option<&Entry> {
        self.database(db)?.entries.get(id)
    }
    pub fn add_path(&mut self, id: DatabaseId, path: PathBuf) {
        self.databases
            .entry(id.clone())
            .or_insert_with(|| Database {
                id,
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path,
                description: None,
                groups: Vec::new(),
                entries: BTreeMap::new(),
                row_ids: Default::default(),
                query: String::new(),
                file_bytes: 0,
                policy: Default::default(),
                writable: false,
                managing: false,
                metadata_conflict: false,
            });
        self.version += 1;
    }
    pub fn apply(&mut self, db: &DatabaseId, snapshot: crate::backend::Snapshot) {
        let Some(database) = self.databases.get_mut(db) else {
            return;
        };
        database.name = snapshot.info.name;
        database.description = snapshot.info.description;
        database.groups = snapshot
            .groups
            .into_iter()
            .map(|info| Group {
                entry_count: info.entry_count,
                source_icon: info.group.icon.clone(),
                description_conflict: info.description_conflict,
                id: info.group.id,
                parent: info.group.parent,
                name: info.group.name,
                description: info.description,
                icon: icon_name(&info.group.icon),
                icon_blob: info.group.icon.blob().cloned(),
            })
            .collect();
        database.query = snapshot.query.search;
        database.row_ids = snapshot.rows.iter().map(|r| r.id.clone()).collect();
        database.entries = snapshot
            .rows
            .into_iter()
            .map(|row| {
                (
                    row.id.clone(),
                    Entry {
                        id: row.id,
                        group: row.group_id,
                        content: {
                            let mut content = EntryContent::default();
                            content.title = row.title;
                            content.username = row.username.unwrap_or_default();
                            content.url = row.url.unwrap_or_default();
                            content.notes = row.notes.unwrap_or_default();
                            content.icon = icon_name(&row.appearance.icon);
                            content.icon_blob = row.appearance.icon.blob().cloned();
                            content.foreground = color_value(row.appearance.foreground);
                            content.background = color_value(row.appearance.background);
                            content
                        },
                        revisions: Vec::new(),
                        created: 0,
                        modified: row.modified_at,
                        conflicted: row.has_conflicts,
                    },
                )
            })
            .collect();
        if let Some(view) = snapshot.entry {
            let entry = Entry {
                id: view.id.clone(),
                group: view.group_id.clone(),
                created: view.created_at,
                modified: view.modified_at,
                conflicted: view.has_conflicts,
                content: {
                    let mut content = content(&view);
                    if let Some(binary) = &snapshot.binary {
                        set_attachment_sizes(&mut content, binary);
                    }
                    content
                },
                revisions: snapshot
                    .history
                    .into_iter()
                    .map(|r| Revision {
                        sequence: r.id,
                        title: r.title,
                        saved_at: r.saved_at,
                        content: None,
                    })
                    .collect(),
            };
            database.entries.insert(view.id, entry);
        }
        database.file_bytes = snapshot.usage.file_bytes;
        database.policy = snapshot.policy;
        database.writable = snapshot.writable;
        database.managing = snapshot.info.managing;
        database.metadata_conflict = snapshot.info.metadata_conflict;
        self.version += 1;
    }
    pub fn set_revision(
        &mut self,
        db: &DatabaseId,
        entry: &EntryId,
        revision: &RevisionId,
        view: taypeer_services::EntryView,
    ) {
        if let Some(row) = self
            .databases
            .get_mut(db)
            .and_then(|d| d.entries.get_mut(entry))
            .and_then(|e| e.revisions.iter_mut().find(|r| &r.sequence == revision))
        {
            row.content = Some(content(&view));
            self.version += 1;
        }
    }
    pub fn clear(&mut self, db: &DatabaseId) {
        if let Some(database) = self.databases.get_mut(db) {
            database.groups.clear();
            database.entries.clear();
            database.row_ids.clear();
            database.query.clear();
            database.description = None;
            database.writable = false;
            database.managing = false;
        }
        self.version += 1;
    }
    pub fn group_path(&self, db: &DatabaseId, group: Option<&GroupId>) -> String {
        let Some(db) = self.database(db) else {
            return String::new();
        };
        let mut cursor = group;
        let mut parts = Vec::new();
        for _ in 0..db.groups.len() {
            let Some(item) = cursor.and_then(|id| db.groups.iter().find(|g| &g.id == id)) else {
                break;
            };
            parts.push(item.name.as_str());
            cursor = item.parent.as_ref();
        }
        parts.reverse();
        parts.join(" / ")
    }
}
pub(crate) fn icon_name(icon: &taypeer_core::IconRef) -> String {
    match icon {
        taypeer_core::IconRef::Lucide(key) => String::from(key.clone()),
        _ => "key-round".into(),
    }
}
pub(crate) fn color_value(color: Option<taypeer_core::Color>) -> Option<u32> {
    color.map(|c| u32::from_be_bytes(c.0))
}
pub(crate) fn format_date(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|time| time.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}
pub(crate) fn content(view: &taypeer_services::EntryView) -> EntryContent {
    EntryContent {
        title: view.title.clone(),
        username: view.username.clone().unwrap_or_default(),
        password: String::new(),
        has_password: view.has_password,
        url: view.url.clone().unwrap_or_default(),
        tags: view.tags.join("\n"),
        notes: view.notes.clone().unwrap_or_default(),
        expires: view.expires_at.map(format_date).unwrap_or_default(),
        attributes: view
            .attributes
            .iter()
            .map(|a| Attribute {
                has_value: a.has_value,
                id: Some(a.id.clone()),
                key: a.name.clone(),
                value: a.value.clone().unwrap_or_default(),
                protected: a.protected,
            })
            .collect(),
        attachments: view
            .attachments
            .iter()
            .map(|a| Attachment {
                id: a.id.clone(),
                name: a.name.clone(),
                bytes: None,
            })
            .collect(),
        icon: icon_name(&view.appearance.icon),
        icon_blob: view.appearance.icon.blob().cloned(),
        foreground: color_value(view.appearance.foreground),
        background: color_value(view.appearance.background),
    }
}

impl Drop for EntryContent {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
        for attribute in &mut self.attributes {
            attribute.value.zeroize();
        }
    }
}

pub(crate) fn set_attachment_sizes(
    content: &mut EntryContent,
    binary: &taypeer_services::BinaryView,
) {
    for attachment in &mut content.attachments {
        attachment.bytes = binary
            .attachments
            .iter()
            .find(|a| a.id == attachment.id)
            .and_then(|a| match a.contents.as_slice() {
                [blob] => blob.bytes,
                _ => None,
            });
    }
}
