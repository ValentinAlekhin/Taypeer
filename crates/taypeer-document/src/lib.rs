//! Volatile Automerge document adapter; no durable storage or authentication.

use automerge::{
    ActorId, Automerge, ChangeHash, ObjId, ObjType, PatchLog, ROOT, ReadDoc, ScalarValue, Value,
    transaction::{Transactable, Transaction},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use taypeer_core::{
    Attribute, AttributeId, AttributeValue, DatabaseId, EntryField, EntryFields, EntryId,
    EntrySnapshot, FieldState, FieldValue, Group, GroupId, RevisionId, RevisionKind, SavedRevision,
    Timestamp, ValidationError, ValueVariant, validate_group_name,
};

/// A document failure without user content or third-party parser diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A local form violated a domain invariant.
    Validation(ValidationError),
    /// The requested object does not exist in this database.
    NotFound,
    /// An operation needs an explicit conflict resolution workflow.
    Conflict,
    /// A draft or replica belongs to another document, or its heads are unknown.
    InvalidContext,
    /// An existing identity was reused for a different operation.
    DuplicateId,
    /// Known document structure is malformed or has an unexpected type.
    InvalidDocument,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<ValidationError> for Error {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}
impl From<automerge::AutomergeError> for Error {
    fn from(_: automerge::AutomergeError) -> Self {
        Self::InvalidDocument
    }
}

/// An unconfirmed local form with its original causal context.
///
/// Cloning is intended for retry after a failed confirmation; debug output is redacted.
#[derive(Clone)]
pub struct EntryDraft {
    database_id: DatabaseId,
    entry_id: EntryId,
    group_id: GroupId,
    revision_id: RevisionId,
    base: Vec<ChangeHash>,
    original: Option<EntryFields>,
    fields: EntryFields,
}

impl EntryDraft {
    /// Identity reserved for the entry, including a not-yet-confirmed creation.
    pub fn entry_id(&self) -> &EntryId {
        &self.entry_id
    }
    /// Destination group; moving entries is outside this increment.
    pub fn group_id(&self) -> &GroupId {
        &self.group_id
    }
    /// Current unconfirmed form.
    pub fn fields(&self) -> &EntryFields {
        &self.fields
    }
    /// Changes the form without changing the document or adding history.
    pub fn fields_mut(&mut self) -> &mut EntryFields {
        &mut self.fields
    }
    /// Adds a local attribute with an independently generated stable identity.
    pub fn add_attribute(&mut self, name: String, value: String, protected: bool) -> AttributeId {
        let id = AttributeId::new(random_id());
        self.fields.attributes.insert(
            id.clone(),
            Attribute {
                id: id.clone(),
                name,
                value: AttributeValue { value, protected },
            },
        );
        id
    }
}

impl fmt::Debug for EntryDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EntryDraft { <redacted> }")
    }
}

/// One process-memory replica of an experimental logical document.
///
/// Confirmation means that memory changed. It never means durable saving or authentication.
pub struct Document {
    database_id: DatabaseId,
    name: String,
    doc: Automerge,
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Document { <redacted> }")
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredRevision {
    revision: SavedRevision,
    changed_operations: BTreeSet<String>,
    // Exact retry identity is checked without logging or hashing user content.
    submitted: Option<EntryFields>,
}

#[derive(Serialize, Deserialize)]
struct Presence {
    alive: bool,
    operation: String,
}

#[derive(Serialize, Deserialize)]
struct GroupPlacement {
    parent: Option<GroupId>,
    order: u64,
}

impl Document {
    /// Creates an empty database with no implicit root group or entry.
    pub fn new(name: impl Into<String>, now: Timestamp) -> Result<Self, Error> {
        let name = name.into();
        validate_group_name(&name)?;
        let database_id = DatabaseId::new(random_id());
        let mut doc = Automerge::new();
        let mut tx = doc.transaction();
        tx.put(ROOT, "database_id", database_id.as_str())?;
        tx.put(ROOT, "name", name.as_str())?;
        tx.put(ROOT, "created_at", now)?;
        tx.put_object(ROOT, "groups", ObjType::Map)?;
        tx.put_object(ROOT, "entries", ObjType::Map)?;
        tx.put_object(ROOT, "revisions", ObjType::Map)?;
        tx.commit();
        Ok(Self {
            database_id,
            name,
            doc,
        })
    }

    /// Logical identity, shared by forks of this database.
    pub fn database_id(&self) -> &DatabaseId {
        &self.database_id
    }
    /// Exact database name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Appends a group under an existing parent, or at the top level.
    pub fn create_group(
        &mut self,
        name: String,
        parent: Option<GroupId>,
        now: Timestamp,
    ) -> Result<Group, Error> {
        validate_group_name(&name)?;
        let groups = self.groups()?;
        if parent
            .as_ref()
            .is_some_and(|id| !groups.iter().any(|group| &group.id == id))
        {
            return Err(Error::NotFound);
        }
        let order = groups
            .iter()
            .filter(|group| group.parent == parent)
            .map(|group| group.order)
            .max()
            .map_or(Some(0), |order| order.checked_add(1))
            .ok_or(Error::InvalidDocument)?;
        let group = Group {
            id: GroupId::new(random_id()),
            name,
            parent,
            order,
            created_at: now,
            modified_at: now,
        };
        let mut tx = self.doc.transaction();
        let root = object(&tx, &ROOT, "groups")?;
        if !tx.get_all(&root, group.id.as_str())?.is_empty() {
            return Err(Error::DuplicateId);
        }
        let object = tx.put_object(&root, group.id.as_str(), ObjType::Map)?;
        tx.put(
            &object,
            "placement",
            encode(&GroupPlacement {
                parent: group.parent.clone(),
                order: group.order,
            })?,
        )?;
        tx.put(&object, "created_at", now)?;
        tx.put_object(&object, "name_times", ObjType::Map)?;
        put_group_name(&mut tx, &object, &group.name, now)?;
        tx.commit();
        Ok(group)
    }

    /// Renames a group without changing its identity, parent, or sibling position.
    pub fn rename_group(
        &mut self,
        id: &GroupId,
        name: String,
        now: Timestamp,
    ) -> Result<(), Error> {
        validate_group_name(&name)?;
        let group = self
            .groups()?
            .into_iter()
            .find(|group| &group.id == id)
            .ok_or(Error::NotFound)?;
        if group.name == name {
            return Ok(());
        }
        let mut tx = self.doc.transaction();
        let root = object(&tx, &ROOT, "groups")?;
        let group_object = object(&tx, &root, id.as_str())?;
        put_group_name(&mut tx, &group_object, &name, now)?;
        tx.commit();
        Ok(())
    }

    /// Lists groups in deterministic parent/order/ID order.
    pub fn groups(&self) -> Result<Vec<Group>, Error> {
        read_groups(&self.doc)
    }

    /// Opens an empty local form and captures the current document heads.
    pub fn begin_create_entry(&self, group: GroupId) -> Result<EntryDraft, Error> {
        self.require_group(&group)?;
        Ok(EntryDraft {
            database_id: self.database_id.clone(),
            entry_id: EntryId::new(random_id()),
            group_id: group,
            revision_id: RevisionId::new(random_id()),
            base: self.doc.get_heads(),
            original: None,
            fields: EntryFields::default(),
        })
    }

    /// Opens an unambiguous entry. Existing conflicts need a later explicit workflow.
    pub fn begin_edit_entry(&self, id: &EntryId) -> Result<EntryDraft, Error> {
        let snapshot = self.entry(id)?;
        let fields = snapshot.fields.ok_or(Error::Conflict)?;
        Ok(EntryDraft {
            database_id: self.database_id.clone(),
            entry_id: id.clone(),
            group_id: snapshot.group_id,
            revision_id: RevisionId::new(random_id()),
            base: self.doc.get_heads(),
            original: Some(fields.clone()),
            fields,
        })
    }

    /// Confirms addressed form changes and one revision atomically in process memory.
    ///
    /// Replaying the same draft is idempotent. A failed confirmation leaves this document intact.
    pub fn save_entry(&mut self, draft: EntryDraft, now: Timestamp) -> Result<EntryId, Error> {
        if draft.database_id != self.database_id {
            return Err(Error::InvalidContext);
        }
        self.require_group(&draft.group_id)?;
        draft.fields.validate()?;
        if draft
            .base
            .iter()
            .any(|head| self.doc.get_change_by_hash(head).is_none())
        {
            return Err(Error::InvalidContext);
        }
        let revisions = object(&self.doc, &ROOT, "revisions")?;
        if let Some(value) = unique_optional(&self.doc, &revisions, draft.revision_id.as_str())? {
            let stored: StoredRevision = decode(&value)?;
            return if stored.revision.entry_id == draft.entry_id
                && stored.submitted.as_ref() == Some(&draft.fields)
                && stored.revision.base
                    == draft
                        .base
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
            {
                Ok(draft.entry_id)
            } else {
                Err(Error::DuplicateId)
            };
        }
        if draft.original.as_ref() == Some(&draft.fields) {
            return Ok(draft.entry_id);
        }
        let new_attributes: BTreeSet<_> = draft
            .fields
            .attributes
            .keys()
            .filter(|id| {
                draft
                    .original
                    .as_ref()
                    .is_none_or(|original| !original.attributes.contains_key(*id))
            })
            .collect();
        if !new_attributes.is_empty() {
            let entries = object(&self.doc, &ROOT, "entries")?;
            for key in self.doc.keys(&entries) {
                let entry = object(&self.doc, &entries, &key)?;
                let attributes = object(&self.doc, &entry, "attributes")?;
                for id in &new_attributes {
                    if !self.doc.get_all(&attributes, id.as_str())?.is_empty() {
                        return Err(Error::DuplicateId);
                    }
                }
            }
        }

        let mut candidate = self.doc.clone();
        let mut tx = candidate.transaction_at(PatchLog::null(), &draft.base);
        let entries = object(&tx, &ROOT, "entries")?;
        let entry = if draft.original.is_none() {
            // Check the current state as well as the isolated base to prevent overwrite on reuse.
            let current_entries = object(&self.doc, &ROOT, "entries")?;
            if !self
                .doc
                .get_all(current_entries, draft.entry_id.as_str())?
                .is_empty()
            {
                return Err(Error::DuplicateId);
            }
            let entry = tx.put_object(entries, draft.entry_id.as_str(), ObjType::Map)?;
            tx.put(&entry, "group", draft.group_id.as_str())?;
            tx.put(&entry, "created_at", now)?;
            tx.put_object(&entry, "attributes", ObjType::Map)?;
            entry
        } else {
            object(&tx, &entries, draft.entry_id.as_str())?
        };
        let changed = apply_form(
            &mut tx,
            &entry,
            draft.original.as_ref(),
            &draft.fields,
            &draft.revision_id,
        )?;
        let snapshot = read_entry(&tx, &draft.entry_id, Some((&changed, now)))?;
        let revision = SavedRevision {
            id: draft.revision_id.clone(),
            entry_id: draft.entry_id.clone(),
            saved_at: now,
            kind: if draft.original.is_none() {
                RevisionKind::Create
            } else {
                RevisionKind::Save
            },
            base: draft.base.iter().map(ToString::to_string).collect(),
            snapshot,
        };
        let stored = StoredRevision {
            revision,
            changed_operations: changed,
            submitted: Some(draft.fields),
        };
        let revisions = object(&tx, &ROOT, "revisions")?;
        tx.put(revisions, draft.revision_id.as_str(), encode(&stored)?)?;
        tx.commit();
        // Conflicts are valid results, but malformed known structure is not.
        read_entry(&candidate, &draft.entry_id, None)?;
        self.doc = candidate;
        Ok(draft.entry_id)
    }

    /// Applies an addressed edit to unambiguous scalar fields while retaining other conflicts.
    ///
    /// This Rust-only command supports domain tests; ordinary UI forms use drafts. Attribute
    /// commands remain on the draft API, which validates their complete local form.
    pub fn edit_fields(
        &mut self,
        id: &EntryId,
        updates: BTreeMap<EntryField, FieldValue>,
        now: Timestamp,
    ) -> Result<EntryId, Error> {
        let before = self.entry(id)?;
        if updates.is_empty() {
            return Ok(id.clone());
        }
        for (field, value) in &updates {
            if matches!(
                field,
                EntryField::AttributeName(_)
                    | EntryField::AttributeValue(_)
                    | EntryField::AttributePresence(_)
            ) {
                return Err(Error::InvalidDocument);
            }
            if before.conflicts.iter().any(|state| &state.field == field) {
                return Err(Error::Conflict);
            }
            validate_field(field, value)?;
        }
        let updates: BTreeMap<_, _> = updates
            .into_iter()
            .filter(|(field, value)| {
                !before.values.iter().any(|state| {
                    &state.field == field
                        && matches!(state.variants.as_slice(), [variant] if &variant.value == value)
                })
            })
            .collect();
        if updates.is_empty() {
            return Ok(id.clone());
        }
        let revision_id = RevisionId::new(random_id());
        let base = self.doc.get_heads();
        let mut candidate = self.doc.clone();
        let mut tx = candidate.transaction();
        let entries = object(&tx, &ROOT, "entries")?;
        let entry = object(&tx, &entries, id.as_str())?;
        let mut changed = BTreeSet::new();
        for (field, value) in updates {
            put_field(&mut tx, &entry, &field, value, &revision_id, &mut changed)?;
        }
        let snapshot = read_entry(&tx, id, Some((&changed, now)))?;
        let stored = StoredRevision {
            revision: SavedRevision {
                id: revision_id.clone(),
                entry_id: id.clone(),
                saved_at: now,
                kind: RevisionKind::Save,
                base: base.iter().map(ToString::to_string).collect(),
                snapshot,
            },
            changed_operations: changed,
            submitted: None,
        };
        let revisions = object(&tx, &ROOT, "revisions")?;
        tx.put(revisions, revision_id.as_str(), encode(&stored)?)?;
        tx.commit();
        read_entry(&candidate, id, None)?;
        self.doc = candidate;
        Ok(id.clone())
    }

    /// Returns every entry without projecting an arbitrary conflict winner.
    pub fn entries(&self) -> Result<Vec<EntrySnapshot>, Error> {
        let root = object(&self.doc, &ROOT, "entries")?;
        self.doc
            .keys(root)
            .map(|key| self.entry(&EntryId::new(key)))
            .collect()
    }

    /// Reads one complete logical entry view.
    pub fn entry(&self, id: &EntryId) -> Result<EntrySnapshot, Error> {
        read_entry(&self.doc, id, None)
    }

    /// Reads immutable confirmations, sorted by display time and stable revision ID.
    pub fn history(&self, id: &EntryId) -> Result<Vec<SavedRevision>, Error> {
        self.entry(id)?;
        let mut revisions: Vec<_> = stored_revisions(&self.doc, id)?
            .into_iter()
            .map(|stored| stored.revision)
            .collect();
        revisions.sort_by(|a, b| (a.saved_at, &a.id).cmp(&(b.saved_at, &b.id)));
        Ok(revisions)
    }

    /// Creates an independent in-memory replica with a fresh Automerge actor.
    ///
    /// This is a test boundary and grants no network admission or credentials.
    pub fn fork(&self) -> Self {
        Self {
            database_id: self.database_id.clone(),
            name: self.name.clone(),
            doc: self.doc.fork(),
        }
    }

    /// Merges another test replica without creating a saved revision.
    ///
    /// The source remains unchanged. Delivery of an already-known change is idempotent.
    pub fn merge(&mut self, other: &Self) -> Result<(), Error> {
        if other.database_id != self.database_id {
            return Err(Error::InvalidContext);
        }
        let mut candidate = self.doc.clone();
        let mut incoming = other.doc.clone();
        candidate.merge(&mut incoming)?;
        let root = object(&candidate, &ROOT, "entries")?;
        let mut attribute_ids = BTreeSet::new();
        for id in candidate.keys(&root) {
            let entry = object(&candidate, &root, &id)?;
            let attributes = object(&candidate, &entry, "attributes")?;
            // Identity outlives ordinary deletion. Inspect retained objects, not only
            // the active attributes projected into the editable entry form.
            for attribute_id in candidate.keys(attributes) {
                if !attribute_ids.insert(attribute_id) {
                    return Err(Error::DuplicateId);
                }
            }
            read_entry(&candidate, &EntryId::new(id), None)?;
        }
        // Group conflicts remain explicit errors at the group projection boundary.
        self.doc = candidate;
        Ok(())
    }

    /// Resolves a retained field operation to its source change, after confirmation.
    pub fn source_change(&self, operation: &str) -> Option<String> {
        let op = self.doc.import_obj(operation).ok()?;
        self.doc.hash_for_opid(&op).map(|hash| hash.to_string())
    }

    fn require_group(&self, id: &GroupId) -> Result<(), Error> {
        if self.groups()?.iter().any(|group| &group.id == id) {
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }
}

fn random_id() -> String {
    ActorId::random().to_hex_string()
}

fn encode<T: Serialize>(value: &T) -> Result<ScalarValue, Error> {
    serde_json::to_vec(value)
        .map(ScalarValue::Bytes)
        .map_err(|_| Error::InvalidDocument)
}

fn decode<T: DeserializeOwned>(value: &Value<'_>) -> Result<T, Error> {
    let Value::Scalar(scalar) = value else {
        return Err(Error::InvalidDocument);
    };
    let ScalarValue::Bytes(bytes) = scalar.as_ref() else {
        return Err(Error::InvalidDocument);
    };
    serde_json::from_slice(bytes).map_err(|_| Error::InvalidDocument)
}

fn unique_optional<R: ReadDoc>(
    read: &R,
    obj: &ObjId,
    key: &str,
) -> Result<Option<Value<'static>>, Error> {
    let values = read.get_all(obj, key)?;
    match values.len() {
        0 => Ok(None),
        1 => Ok(values
            .into_iter()
            .next()
            .map(|(value, _)| value.into_owned())),
        _ => Err(Error::Conflict),
    }
}

fn unique<R: ReadDoc>(read: &R, obj: &ObjId, key: &str) -> Result<Value<'static>, Error> {
    unique_optional(read, obj, key)?.ok_or(Error::NotFound)
}

fn object<R: ReadDoc>(read: &R, obj: &ObjId, key: &str) -> Result<ObjId, Error> {
    let values = read.get_all(obj, key)?;
    if values.len() > 1 {
        return Err(Error::DuplicateId);
    }
    let (value, id) = values.into_iter().next().ok_or(Error::NotFound)?;
    if value != Value::Object(ObjType::Map) {
        return Err(Error::InvalidDocument);
    }
    Ok(id)
}

fn read_groups<R: ReadDoc>(read: &R) -> Result<Vec<Group>, Error> {
    let root = object(read, &ROOT, "groups")?;
    let mut result = Vec::new();
    for key in read.keys(&root) {
        let obj = object(read, &root, &key)?;
        let placement: GroupPlacement = decode(&unique(read, &obj, "placement")?)?;
        let created_at = unique(read, &obj, "created_at")?
            .to_i64()
            .ok_or(Error::InvalidDocument)?;
        let mut names = BTreeSet::new();
        let name_times = object(read, &obj, "name_times")?;
        let mut times = Vec::new();
        for (value, operation) in read.get_all(&obj, "name")? {
            let name = value.to_str().ok_or(Error::InvalidDocument)?;
            validate_group_name(name)?;
            names.insert(name.to_owned());
            times.push(
                unique(read, &name_times, &operation.to_string())?
                    .to_i64()
                    .ok_or(Error::InvalidDocument)?,
            );
        }
        if names.len() > 1 {
            return Err(Error::Conflict);
        }
        let name = names.into_iter().next().ok_or(Error::InvalidDocument)?;
        result.push(Group {
            id: GroupId::new(key),
            name,
            parent: placement.parent,
            order: placement.order,
            created_at,
            modified_at: times.into_iter().max().unwrap_or(created_at),
        });
    }
    result.sort_by(|a, b| (&a.parent, a.order, &a.id).cmp(&(&b.parent, b.order, &b.id)));
    Ok(result)
}

fn put_group_name(
    tx: &mut Transaction<'_>,
    group: &ObjId,
    name: &str,
    now: Timestamp,
) -> Result<(), Error> {
    tx.put(group, "name", name)?;
    let times = object(tx, group, "name_times")?;
    let operations: Vec<_> = tx
        .get_all(group, "name")?
        .into_iter()
        .map(|(_, operation)| operation)
        .collect();
    for operation in operations {
        if tx.hash_for_opid(&operation).is_none() {
            tx.put(&times, operation.to_string(), now)?;
        }
    }
    Ok(())
}

fn stored_revisions<R: ReadDoc>(read: &R, id: &EntryId) -> Result<Vec<StoredRevision>, Error> {
    let root = object(read, &ROOT, "revisions")?;
    let mut result = Vec::new();
    for key in read.keys(&root) {
        let stored: StoredRevision = decode(&unique(read, &root, &key)?)?;
        if stored.revision.id.as_str() != key {
            return Err(Error::InvalidDocument);
        }
        if &stored.revision.entry_id == id {
            result.push(stored);
        }
    }
    Ok(result)
}

fn field_key(field: &EntryField) -> &str {
    match field {
        EntryField::Title => "title",
        EntryField::Username => "username",
        EntryField::Password => "password",
        EntryField::Url => "url",
        EntryField::Notes => "notes",
        EntryField::Tags => "tags",
        EntryField::ExpiresAt => "expires_at",
        EntryField::AttributeName(_) => "name",
        EntryField::AttributeValue(_) => "value",
        EntryField::AttributePresence(_) => "presence",
    }
}

fn field_object<R: ReadDoc>(read: &R, entry: &ObjId, field: &EntryField) -> Result<ObjId, Error> {
    match field {
        EntryField::AttributeName(id)
        | EntryField::AttributeValue(id)
        | EntryField::AttributePresence(id) => {
            let attrs = object(read, entry, "attributes")?;
            object(read, &attrs, id.as_str())
        }
        _ => Ok(entry.clone()),
    }
}

fn validate_field(field: &EntryField, value: &FieldValue) -> Result<(), Error> {
    let valid = match (field, value) {
        (EntryField::Title, FieldValue::Text(Some(value))) => {
            if value.is_empty() {
                return Err(ValidationError::EmptyTitle.into());
            }
            true
        }
        (
            EntryField::Username | EntryField::Password | EntryField::Url | EntryField::Notes,
            FieldValue::Text(_),
        ) => true,
        (EntryField::Tags, FieldValue::Tags(_))
        | (EntryField::ExpiresAt, FieldValue::Timestamp(_)) => true,
        (EntryField::AttributeName(_), FieldValue::Text(Some(value))) => {
            if value.is_empty() {
                return Err(ValidationError::EmptyAttributeName.into());
            }
            true
        }
        (EntryField::AttributeValue(_), FieldValue::Attribute(_))
        | (EntryField::AttributePresence(_), FieldValue::Presence(_)) => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidDocument)
    }
}

fn put_field(
    tx: &mut Transaction<'_>,
    entry: &ObjId,
    field: &EntryField,
    value: FieldValue,
    revision: &RevisionId,
    changed: &mut BTreeSet<String>,
) -> Result<(), Error> {
    validate_field(field, &value)?;
    let scalar = match value {
        FieldValue::Text(Some(value)) => ScalarValue::from(value),
        FieldValue::Text(None) | FieldValue::Timestamp(None) => ScalarValue::Null,
        FieldValue::Timestamp(Some(value)) => ScalarValue::Timestamp(value),
        FieldValue::Tags(value) => encode(&value)?,
        FieldValue::Attribute(value) => encode(&value)?,
        FieldValue::Presence(alive) => encode(&Presence {
            alive,
            operation: revision.as_str().into(),
        })?,
    };
    let obj = field_object(tx, entry, field)?;
    tx.put(&obj, field_key(field), scalar)?;
    for (_, operation) in tx.get_all(&obj, field_key(field))? {
        if tx.hash_for_opid(&operation).is_none() {
            changed.insert(operation.to_string());
        }
    }
    Ok(())
}

fn form_fields(fields: &EntryFields) -> Vec<(EntryField, FieldValue)> {
    vec![
        (
            EntryField::Title,
            FieldValue::Text(Some(fields.title.clone())),
        ),
        (
            EntryField::Username,
            FieldValue::Text(fields.username.clone()),
        ),
        (
            EntryField::Password,
            FieldValue::Text(fields.password.clone()),
        ),
        (EntryField::Url, FieldValue::Text(fields.url.clone())),
        (EntryField::Notes, FieldValue::Text(fields.notes.clone())),
        (EntryField::Tags, FieldValue::Tags(fields.tags.clone())),
        (
            EntryField::ExpiresAt,
            FieldValue::Timestamp(fields.expires_at),
        ),
    ]
}

fn apply_form(
    tx: &mut Transaction<'_>,
    entry: &ObjId,
    original: Option<&EntryFields>,
    fields: &EntryFields,
    revision: &RevisionId,
) -> Result<BTreeSet<String>, Error> {
    let mut changed = BTreeSet::new();
    let old: BTreeMap<_, _> = original
        .map(form_fields)
        .unwrap_or_default()
        .into_iter()
        .collect();
    for (field, value) in form_fields(fields) {
        if old.get(&field) != Some(&value) {
            put_field(tx, entry, &field, value, revision, &mut changed)?;
        }
    }
    let attrs = object(tx, entry, "attributes")?;
    for (id, attr) in &fields.attributes {
        let old = original.and_then(|fields| fields.attributes.get(id));
        if old == Some(attr) {
            continue;
        }
        if old.is_none() {
            if !tx.get_all(&attrs, id.as_str())?.is_empty() {
                return Err(Error::DuplicateId);
            }
            tx.put_object(&attrs, id.as_str(), ObjType::Map)?;
        }
        if old.is_none_or(|old| old.name != attr.name) {
            put_field(
                tx,
                entry,
                &EntryField::AttributeName(id.clone()),
                FieldValue::Text(Some(attr.name.clone())),
                revision,
                &mut changed,
            )?;
        }
        if old.is_none_or(|old| old.value != attr.value) {
            put_field(
                tx,
                entry,
                &EntryField::AttributeValue(id.clone()),
                FieldValue::Attribute(attr.value.clone()),
                revision,
                &mut changed,
            )?;
        }
        // Every meaningful attribute edit witnesses presence. A unique operation marker avoids
        // same-value write elision, while presentation coalesces concurrent `true` values.
        put_field(
            tx,
            entry,
            &EntryField::AttributePresence(id.clone()),
            FieldValue::Presence(true),
            revision,
            &mut changed,
        )?;
    }
    if let Some(original) = original {
        for id in original
            .attributes
            .keys()
            .filter(|id| !fields.attributes.contains_key(*id))
        {
            put_field(
                tx,
                entry,
                &EntryField::AttributePresence(id.clone()),
                FieldValue::Presence(false),
                revision,
                &mut changed,
            )?;
        }
    }
    Ok(changed)
}

fn decode_field(field: &EntryField, value: &Value<'_>) -> Result<FieldValue, Error> {
    let decoded = match field {
        EntryField::Title
        | EntryField::Username
        | EntryField::Password
        | EntryField::Url
        | EntryField::Notes
        | EntryField::AttributeName(_) => {
            if value == &Value::Scalar(std::borrow::Cow::Owned(ScalarValue::Null)) {
                FieldValue::Text(None)
            } else {
                FieldValue::Text(Some(value.to_str().ok_or(Error::InvalidDocument)?.into()))
            }
        }
        EntryField::Tags => FieldValue::Tags(decode(value)?),
        EntryField::ExpiresAt => {
            let Value::Scalar(scalar) = value else {
                return Err(Error::InvalidDocument);
            };
            FieldValue::Timestamp(match scalar.as_ref() {
                ScalarValue::Null => None,
                ScalarValue::Timestamp(time) => Some(*time),
                _ => return Err(Error::InvalidDocument),
            })
        }
        EntryField::AttributeValue(_) => FieldValue::Attribute(decode(value)?),
        EntryField::AttributePresence(_) => FieldValue::Presence(decode::<Presence>(value)?.alive),
    };
    validate_field(field, &decoded)?;
    Ok(decoded)
}

fn read_field<R: ReadDoc>(read: &R, entry: &ObjId, field: EntryField) -> Result<FieldState, Error> {
    let obj = field_object(read, entry, &field)?;
    let mut variants: Vec<ValueVariant> = Vec::new();
    for (value, operation) in read.get_all(obj, field_key(&field))? {
        let value = decode_field(&field, &value)?;
        if let Some(existing) = variants.iter_mut().find(|variant| variant.value == value) {
            existing.origins.push(operation.to_string());
        } else {
            variants.push(ValueVariant {
                value,
                origins: vec![operation.to_string()],
            });
        }
    }
    if variants.is_empty() {
        return Err(Error::InvalidDocument);
    }
    for variant in &mut variants {
        variant.origins.sort();
    }
    variants.sort_by(|a, b| a.origins.cmp(&b.origins));
    Ok(FieldState { field, variants })
}

fn read_entry<R: ReadDoc>(
    read: &R,
    id: &EntryId,
    pending: Option<(&BTreeSet<String>, Timestamp)>,
) -> Result<EntrySnapshot, Error> {
    let entries = object(read, &ROOT, "entries")?;
    let entry = object(read, &entries, id.as_str())?;
    let group_id = GroupId::new(
        unique(read, &entry, "group")?
            .to_str()
            .ok_or(Error::InvalidDocument)?,
    );
    let created_at = unique(read, &entry, "created_at")?
        .to_i64()
        .ok_or(Error::InvalidDocument)?;
    let mut values = Vec::new();
    for field in [
        EntryField::Title,
        EntryField::Username,
        EntryField::Password,
        EntryField::Url,
        EntryField::Notes,
        EntryField::Tags,
        EntryField::ExpiresAt,
    ] {
        values.push(read_field(read, &entry, field)?);
    }
    let attributes = object(read, &entry, "attributes")?;
    for key in read.keys(attributes) {
        let attr = AttributeId::new(key);
        let presence = read_field(read, &entry, EntryField::AttributePresence(attr.clone()))?;
        let removed = matches!(presence.variants.as_slice(), [variant] if variant.value == FieldValue::Presence(false));
        if !removed {
            for field in [
                EntryField::AttributeName(attr.clone()),
                EntryField::AttributeValue(attr),
            ] {
                values.push(read_field(read, &entry, field)?);
            }
        }
        values.push(presence);
    }
    let mut conflicts: Vec<_> = values
        .iter()
        .filter(|state| state.variants.len() > 1)
        .cloned()
        .collect();
    let origins: BTreeSet<_> = values
        .iter()
        .flat_map(|state| state.variants.iter())
        .flat_map(|variant| variant.origins.iter().cloned())
        .collect();
    let mut operation_times = BTreeMap::new();
    for stored in stored_revisions(read, id)? {
        for operation in stored.changed_operations {
            operation_times.insert(operation, stored.revision.saved_at);
        }
    }
    if let Some((pending, now)) = pending {
        for operation in pending {
            operation_times.insert(operation.clone(), now);
        }
    }
    let modified_at = origins
        .iter()
        .filter_map(|operation| operation_times.get(operation))
        .copied()
        .max()
        .unwrap_or(created_at);
    let fields = if conflicts.is_empty() {
        let fields = project_fields(&values)?;
        match fields.validate() {
            Ok(()) => Some(fields),
            Err(ValidationError::DuplicateAttributeName) => {
                let mut names: BTreeMap<&str, Vec<&Attribute>> = BTreeMap::new();
                for attribute in fields.attributes.values() {
                    names.entry(&attribute.name).or_default().push(attribute);
                }
                let duplicate_ids: BTreeSet<_> = names
                    .values()
                    .filter(|attrs| attrs.len() > 1)
                    .flatten()
                    .map(|attr| &attr.id)
                    .collect();
                conflicts.extend(values.iter().filter(|state| matches!(&state.field, EntryField::AttributeName(id) if duplicate_ids.contains(id))).cloned());
                None
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        None
    };
    Ok(EntrySnapshot {
        id: id.clone(),
        group_id,
        fields,
        conflicts,
        values,
        created_at,
        modified_at,
    })
}

fn project_fields(values: &[FieldState]) -> Result<EntryFields, Error> {
    let mut result = EntryFields::default();
    let mut names = BTreeMap::new();
    let mut attr_values = BTreeMap::new();
    let mut present = BTreeSet::new();
    for state in values {
        let [variant] = state.variants.as_slice() else {
            return Err(Error::Conflict);
        };
        match (&state.field, &variant.value) {
            (EntryField::Title, FieldValue::Text(Some(value))) => result.title = value.clone(),
            (EntryField::Username, FieldValue::Text(value)) => result.username = value.clone(),
            (EntryField::Password, FieldValue::Text(value)) => result.password = value.clone(),
            (EntryField::Url, FieldValue::Text(value)) => result.url = value.clone(),
            (EntryField::Notes, FieldValue::Text(value)) => result.notes = value.clone(),
            (EntryField::Tags, FieldValue::Tags(value)) => result.tags = value.clone(),
            (EntryField::ExpiresAt, FieldValue::Timestamp(value)) => result.expires_at = *value,
            (EntryField::AttributeName(id), FieldValue::Text(Some(value))) => {
                names.insert(id.clone(), value.clone());
            }
            (EntryField::AttributeValue(id), FieldValue::Attribute(value)) => {
                attr_values.insert(id.clone(), value.clone());
            }
            (EntryField::AttributePresence(id), FieldValue::Presence(true)) => {
                present.insert(id.clone());
            }
            (EntryField::AttributePresence(_), FieldValue::Presence(false)) => {}
            _ => return Err(Error::InvalidDocument),
        }
    }
    for id in present {
        let name = names.remove(&id).ok_or(Error::InvalidDocument)?;
        let value = attr_values.remove(&id).ok_or(Error::InvalidDocument)?;
        result
            .attributes
            .insert(id.clone(), Attribute { id, name, value });
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
