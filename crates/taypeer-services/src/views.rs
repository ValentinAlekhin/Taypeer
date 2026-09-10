//! Presentation values and explicit secret access returned by the service.

use super::ServiceError;
use serde::{Deserialize, Serialize};
use std::fmt;
use taypeer_core::{AttributeId, DatabaseId, EntryId, GroupId, RevisionId};
use taypeer_core::{EntrySnapshot, Group};

/// Non-secret catalog data available while a database is locked.
#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseSummary {
    /// Stable database identifier.
    pub id: DatabaseId,
    /// Public demonstration database label.
    pub name: String,
    /// Whether a new unlock is needed for document access.
    pub locked: bool,
}

/// A group visible within an unlocked database.
#[derive(Clone, Serialize, Deserialize)]
pub struct GroupSummary {
    /// Stable group identifier.
    pub id: GroupId,
    /// Group label.
    pub name: String,
    /// Parent group, or no parent for the visible top level.
    pub parent: Option<GroupId>,
}

/// A table/search row; password and protected attributes are never included.
#[derive(Clone, Serialize, Deserialize)]
pub struct EntrySummary {
    /// Stable entry identifier.
    pub id: EntryId,
    /// Owning group, absent when placement needs resolution.
    pub group_id: Option<GroupId>,
    /// Entry label; empty if unresolved conflicts prevent a unique view.
    pub title: String,
    /// Ordinary username, if present.
    pub username: Option<String>,
    /// Ordinary URL, if present.
    pub url: Option<String>,
    /// Whether the row requires conflict resolution before editing.
    pub has_conflicts: bool,
}

/// A search row with the database and group needed to navigate to its source.
#[derive(Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Public database label.
    pub database_name: String,
    /// Source group label.
    pub group_name: String,
    /// Matching entry without secret values.
    pub entry: EntrySummary,
}

/// Editable form retaining the distinction between absent and explicitly empty fields.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableEntry {
    /// Required entry label.
    pub title: String,
    /// Optional ordinary username.
    pub username: Option<String>,
    /// Optional secret; omitted from Debug output.
    pub password: Option<String>,
    /// Optional ordinary URL.
    pub url: Option<String>,
    /// Optional notes.
    pub notes: Option<String>,
    /// Tags; saved as a set by the domain model.
    pub tags: Vec<String>,
    /// Optional expiration timestamp in UTC milliseconds since the Unix epoch.
    pub expires_at: Option<i64>,
    /// Attributes with stable identifiers and atomic value/protection pairs.
    pub attributes: Vec<EditableAttribute>,
}

impl fmt::Debug for EditableEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EditableEntry { fields: [REDACTED] }")
    }
}

/// One editable attribute; new attributes receive an identifier on draft update.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableAttribute {
    /// Existing identifier, or None only for a new attribute.
    pub id: Option<AttributeId>,
    /// Attribute label.
    pub name: String,
    /// Attribute value; never included in Debug output.
    pub value: String,
    /// Whether list/search/read-only views must hide the value.
    pub protected: bool,
}

impl fmt::Debug for EditableAttribute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EditableAttribute { fields: [REDACTED] }")
    }
}

/// The single local form currently associated with a database.
#[derive(Clone, Serialize, Deserialize)]
pub struct DraftView {
    /// Existing entry identifier; None denotes a not-yet-saved entry.
    pub entry_id: Option<EntryId>,
    /// Group in which the entry is being edited or created.
    pub group_id: GroupId,
    /// Form fields, including explicitly edited secrets.
    pub fields: EditableEntry,
    /// Whether the form differs from its starting state.
    pub dirty: bool,
    /// Unparsed expiration input retained for restoring an interrupted invalid form.
    /// None means the platform has supplied a valid optional timestamp.
    pub expiry_input: Option<String>,
}

/// Non-secret identity of an interrupted editor awaiting explicit restoration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingDraftSummary {
    /// Existing entry, or None for a not-yet-saved entry.
    pub entry_id: Option<EntryId>,
    /// Group containing the interrupted form.
    pub group_id: GroupId,
}

/// An attribute in a read-only view; protected values remain absent.
#[derive(Clone, Serialize, Deserialize)]
pub struct AttributeView {
    /// Stable attribute identifier.
    pub id: AttributeId,
    /// Attribute label.
    pub name: String,
    /// Value only when the attribute is not protected.
    pub value: Option<String>,
    /// Whether explicit reveal is required.
    pub protected: bool,
}

/// Read-only entry details with masked password and protected attributes.
#[derive(Clone, Serialize, Deserialize)]
pub struct EntryView {
    /// Stable entry identifier.
    pub id: EntryId,
    /// Owning group, absent when placement needs resolution.
    pub group_id: Option<GroupId>,
    /// Entry label; empty for an ambiguous snapshot.
    pub title: String,
    /// Optional ordinary username.
    pub username: Option<String>,
    /// Optional ordinary URL.
    pub url: Option<String>,
    /// Optional ordinary notes.
    pub notes: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Optional expiration timestamp in UTC milliseconds since the Unix epoch.
    pub expires_at: Option<i64>,
    /// Attributes with protected values removed.
    pub attributes: Vec<AttributeView>,
    /// Whether a password field exists, including an explicitly empty one.
    pub has_password: bool,
    /// Whether conflict resolution is required before editing or revealing.
    pub has_conflicts: bool,
    /// Creation timestamp in UTC milliseconds since the Unix epoch.
    pub created_at: i64,
    /// Modification timestamp in UTC milliseconds since the Unix epoch.
    pub modified_at: i64,
}

/// A history row with no secret values.
#[derive(Clone, Serialize, Deserialize)]
pub struct RevisionSummary {
    /// Stable saved revision identifier.
    pub id: RevisionId,
    /// Saved entry title, or empty when ambiguous.
    pub title: String,
    /// Explicit-save timestamp in UTC milliseconds since the Unix epoch.
    pub saved_at: i64,
}

macro_rules! redacted_debug {
    ($($kind:ty),+ $(,)?) => {
        $(impl fmt::Debug for $kind {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($kind), " { contents: [REDACTED] }"))
            }
        })+
    };
}

redacted_debug!(
    DatabaseSummary,
    GroupSummary,
    EntrySummary,
    SearchResult,
    DraftView,
    AttributeView,
    EntryView,
    RevisionSummary,
);

/// A value returned only by explicit reveal; formatting never prints its contents.
#[derive(Clone)]
pub struct SecretValue(zeroize::Zeroizing<String>);

impl SecretValue {
    /// Borrow the revealed text for a current, explicitly requested UI presentation.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

pub(super) fn group_summary(group: Group) -> GroupSummary {
    GroupSummary {
        id: group.id,
        name: group.name,
        parent: group.parent,
    }
}

pub(super) fn matches_query(entry: &EntrySnapshot, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let Some(fields) = &entry.fields else {
        return false;
    };
    let contains = |value: &str| value.to_lowercase().contains(query);
    contains(&fields.title)
        || fields.username.as_deref().is_some_and(contains)
        || fields.url.as_deref().is_some_and(contains)
        || fields.notes.as_deref().is_some_and(contains)
        || fields.tags.iter().any(|tag| contains(tag))
        || fields
            .attributes
            .values()
            .any(|attribute| !attribute.value.protected && contains(&attribute.value.value))
}

pub(super) fn entry_summary(entry: EntrySnapshot) -> EntrySummary {
    let has_conflicts = entry.has_conflicts();
    let fields = entry.fields.unwrap_or_default();
    EntrySummary {
        id: entry.id,
        group_id: entry.group_id,
        title: fields.title,
        username: fields.username,
        url: fields.url,
        has_conflicts,
    }
}

pub(super) fn entry_view(entry: EntrySnapshot) -> EntryView {
    let has_conflicts = entry.has_conflicts();
    let fields = entry.fields.unwrap_or_default();
    EntryView {
        id: entry.id,
        group_id: entry.group_id,
        title: fields.title,
        username: fields.username,
        url: fields.url,
        notes: fields.notes,
        tags: fields.tags.into_iter().collect(),
        expires_at: fields.expires_at,
        attributes: fields
            .attributes
            .into_values()
            .map(|attribute| AttributeView {
                id: attribute.id,
                name: attribute.name,
                value: (!attribute.value.protected).then_some(attribute.value.value),
                protected: attribute.value.protected,
            })
            .collect(),
        has_password: fields.password.is_some(),
        has_conflicts,
        created_at: entry.created_at,
        modified_at: entry.modified_at,
    }
}

pub(super) fn password(entry: EntrySnapshot) -> Result<SecretValue, ServiceError> {
    entry
        .fields
        .ok_or(ServiceError::Conflict)?
        .password
        .map(|value| SecretValue(zeroize::Zeroizing::new(value)))
        .ok_or(ServiceError::NotFound)
}

pub(super) fn attribute_value(
    entry: EntrySnapshot,
    attribute: &AttributeId,
) -> Result<SecretValue, ServiceError> {
    entry
        .fields
        .ok_or(ServiceError::Conflict)?
        .attributes
        .remove(attribute)
        .map(|attribute| SecretValue(zeroize::Zeroizing::new(attribute.value.value)))
        .ok_or(ServiceError::NotFound)
}
