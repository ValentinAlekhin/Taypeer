//! In-memory presentation data. These types are deliberately not service DTOs.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DatabaseId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GroupId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EntryId(pub u64);

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Attribute {
    pub key: String,
    pub value: String,
    pub protected: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct EntryContent {
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub tags: String,
    pub notes: String,
    pub expires: String,
    pub attributes: Vec<Attribute>,
    pub attachments: Vec<Attachment>,
    pub icon: String,
    pub foreground: Option<u32>,
    pub background: Option<u32>,
}

#[derive(Clone)]
pub(crate) struct Revision {
    pub sequence: u64,
    pub content: EntryContent,
}

#[derive(Clone)]
pub(crate) struct Entry {
    pub id: EntryId,
    pub group: GroupId,
    pub content: EntryContent,
    pub revisions: Vec<Revision>,
    pub created: u64,
    pub modified: u64,
}

#[derive(Clone)]
pub(crate) struct Group {
    pub id: GroupId,
    pub parent: Option<GroupId>,
    pub name: String,
    pub description: String,
    pub icon: String,
}

pub(crate) struct Database {
    pub id: DatabaseId,
    pub name: String,
    pub description: String,
    pub groups: Vec<Group>,
    pub entries: BTreeMap<EntryId, Entry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormError {
    RequiredName,
    DuplicateAttribute,
    MissingObject,
    PasswordConfirmation,
    InvalidColor,
    InvalidNumber,
}

impl FormError {
    pub fn key(self) -> &'static str {
        match self {
            Self::RequiredName => "ui.required_name",
            Self::DuplicateAttribute => "ui.duplicate_attribute",
            Self::MissingObject => "ui.missing_object",
            Self::PasswordConfirmation => "ui.password_confirmation",
            Self::InvalidColor => "ui.invalid_color",
            Self::InvalidNumber => "ui.invalid_number",
        }
    }
}

pub(crate) struct CatalogStore {
    databases: BTreeMap<DatabaseId, Database>,
    next_id: u64,
    sequence: u64,
    version: u64,
}

impl CatalogStore {
    pub fn samples() -> Self {
        super::fixtures::catalog()
    }

    pub(super) fn new() -> Self {
        Self {
            databases: BTreeMap::new(),
            next_id: 1,
            sequence: 1,
            version: 0,
        }
    }

    fn next(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn databases(&self) -> impl Iterator<Item = &Database> {
        self.databases.values()
    }
    pub fn database(&self, id: DatabaseId) -> Option<&Database> {
        self.databases.get(&id)
    }
    pub fn entry(&self, db: DatabaseId, id: EntryId) -> Option<&Entry> {
        self.database(db)?.entries.get(&id)
    }

    pub fn create_database(
        &mut self,
        name: String,
        description: String,
    ) -> Result<DatabaseId, FormError> {
        require_name(&name)?;
        let id = DatabaseId(self.next());
        self.databases.insert(
            id,
            Database {
                id,
                name,
                description,
                groups: Vec::new(),
                entries: BTreeMap::new(),
            },
        );
        self.version += 1;
        Ok(id)
    }

    pub fn update_database(
        &mut self,
        id: DatabaseId,
        name: String,
        description: String,
    ) -> Result<(), FormError> {
        require_name(&name)?;
        let db = self
            .databases
            .get_mut(&id)
            .ok_or(FormError::MissingObject)?;
        db.name = name;
        db.description = description;
        self.version += 1;
        Ok(())
    }

    pub fn save_group(
        &mut self,
        db: DatabaseId,
        id: Option<GroupId>,
        parent: Option<GroupId>,
        name: String,
        description: String,
        icon: String,
    ) -> Result<GroupId, FormError> {
        require_name(&name)?;
        let database = self.database(db).ok_or(FormError::MissingObject)?;
        if parent.is_some_and(|p| !database.groups.iter().any(|g| g.id == p))
            || id.is_some_and(|id| !database.groups.iter().any(|g| g.id == id))
        {
            return Err(FormError::MissingObject);
        }
        let group_id = id.unwrap_or_else(|| GroupId(self.next()));
        let database = self
            .databases
            .get_mut(&db)
            .ok_or(FormError::MissingObject)?;
        if let Some(group) = database.groups.iter_mut().find(|g| g.id == group_id) {
            group.name = name;
            group.description = description;
            group.icon = icon;
        } else {
            database.groups.push(Group {
                id: group_id,
                parent,
                name,
                description,
                icon,
            });
        }
        self.version += 1;
        Ok(group_id)
    }

    pub fn save_entry(
        &mut self,
        db: DatabaseId,
        group: GroupId,
        id: Option<EntryId>,
        content: EntryContent,
    ) -> Result<EntryId, FormError> {
        validate_content(&content)?;
        let database = self.database(db).ok_or(FormError::MissingObject)?;
        if !database.groups.iter().any(|g| g.id == group)
            || id.is_some_and(|id| !database.entries.contains_key(&id))
        {
            return Err(FormError::MissingObject);
        }
        if let Some(id) = id
            && self
                .entry(db, id)
                .is_some_and(|entry| entry.content == content)
        {
            return Ok(id);
        }
        let id = id.unwrap_or_else(|| EntryId(self.next()));
        self.sequence += 1;
        let sequence = self.sequence;
        let database = self
            .databases
            .get_mut(&db)
            .ok_or(FormError::MissingObject)?;
        let entry = database.entries.entry(id).or_insert_with(|| Entry {
            id,
            group,
            content: content.clone(),
            revisions: Vec::new(),
            created: sequence,
            modified: sequence,
        });
        entry.content = content.clone();
        entry.modified = sequence;
        entry.revisions.push(Revision { sequence, content });
        self.version += 1;
        Ok(id)
    }

    pub fn restore_revision(
        &mut self,
        db: DatabaseId,
        id: EntryId,
        sequence: u64,
    ) -> Result<(), FormError> {
        let entry = self.entry(db, id).ok_or(FormError::MissingObject)?;
        let revision = entry
            .revisions
            .iter()
            .find(|r| r.sequence == sequence)
            .ok_or(FormError::MissingObject)?;
        self.save_entry(db, entry.group, Some(id), revision.content.clone())?;
        Ok(())
    }

    pub fn clear_history(&mut self, db: DatabaseId, id: EntryId) -> Result<(), FormError> {
        self.databases
            .get_mut(&db)
            .and_then(|db| db.entries.get_mut(&id))
            .ok_or(FormError::MissingObject)?
            .revisions
            .clear();
        self.version += 1;
        Ok(())
    }

    pub fn group_path(&self, db: DatabaseId, group: GroupId) -> String {
        let Some(db) = self.database(db) else {
            return String::new();
        };
        let mut names = Vec::new();
        let mut current = Some(group);
        while let Some(id) = current {
            let Some(group) = db.groups.iter().find(|g| g.id == id) else {
                break;
            };
            names.push(group.name.as_str());
            current = group.parent;
        }
        names.reverse();
        format!("{} / {}", db.name, names.join(" / "))
    }
}

pub(crate) fn require_name(name: &str) -> Result<(), FormError> {
    if name.trim().is_empty() {
        Err(FormError::RequiredName)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_content(content: &EntryContent) -> Result<(), FormError> {
    require_name(&content.title)?;
    let mut keys = std::collections::BTreeSet::new();
    for attribute in &content.attributes {
        require_name(&attribute.key)?;
        if !keys.insert(&attribute.key) {
            return Err(FormError::DuplicateAttribute);
        }
    }
    Ok(())
}
