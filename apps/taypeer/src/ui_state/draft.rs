//! A single draft spans all editable tabs. Widget state is a presentation adapter.

use super::{DatabaseId, Entry, EntryContent, EntryId, FormError, GroupId, validate_content};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryField {
    Title,
    Username,
    Password,
    Url,
    Tags,
    Notes,
    Expires,
}
impl EntryField {
    pub const ALL: [Self; 7] = [
        Self::Title,
        Self::Username,
        Self::Password,
        Self::Url,
        Self::Tags,
        Self::Expires,
        Self::Notes,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "name",
            Self::Username => "username",
            Self::Password => "password",
            Self::Url => "url",
            Self::Tags => "ui.tags",
            Self::Notes => "notes",
            Self::Expires => "ui.expires",
        }
    }
    pub fn value(self, content: &EntryContent) -> &str {
        match self {
            Self::Title => &content.title,
            Self::Username => &content.username,
            Self::Password => &content.password,
            Self::Url => &content.url,
            Self::Tags => &content.tags,
            Self::Notes => &content.notes,
            Self::Expires => &content.expires,
        }
    }
    pub fn set(self, content: &mut EntryContent, value: String) {
        *match self {
            Self::Title => &mut content.title,
            Self::Username => &mut content.username,
            Self::Password => &mut content.password,
            Self::Url => &mut content.url,
            Self::Tags => &mut content.tags,
            Self::Notes => &mut content.notes,
            Self::Expires => &mut content.expires,
        } = value;
    }
}

#[derive(Clone)]
pub(crate) struct EditorStore {
    database: DatabaseId,
    group: GroupId,
    entry: Option<EntryId>,
    original: EntryContent,
    content: EntryContent,
    error: Option<FormError>,
}

impl EditorStore {
    pub fn existing(database: DatabaseId, entry: &Entry) -> Self {
        Self {
            database,
            group: entry.group,
            entry: Some(entry.id),
            original: entry.content.clone(),
            content: entry.content.clone(),
            error: None,
        }
    }
    pub fn new(database: DatabaseId, group: GroupId) -> Self {
        let content = EntryContent {
            icon: "key-round".into(),
            ..Default::default()
        };
        Self {
            database,
            group,
            entry: None,
            original: content.clone(),
            content,
            error: None,
        }
    }
    pub fn database(&self) -> DatabaseId {
        self.database
    }
    pub fn group(&self) -> GroupId {
        self.group
    }
    pub fn entry(&self) -> Option<EntryId> {
        self.entry
    }
    pub fn content(&self) -> &EntryContent {
        &self.content
    }
    pub fn dirty(&self) -> bool {
        self.content != self.original
    }
    pub fn error(&self) -> Option<FormError> {
        self.error
    }
    pub fn edit(&mut self, edit: impl FnOnce(&mut EntryContent)) {
        edit(&mut self.content);
        self.error = None;
    }
    pub fn validate(&mut self) -> Result<(), FormError> {
        let result = validate_content(&self.content);
        self.error = result.as_ref().err().copied();
        result
    }
}
