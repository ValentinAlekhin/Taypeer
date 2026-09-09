//! Volatile Automerge document adapter; no durable storage or authentication.

use automerge::{
    ActorId, Automerge, ChangeHash, ObjType, PatchLog, ROOT, ReadDoc, transaction::Transactable,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use taypeer_core::{
    Attribute, AttributeId, AttributeValue, DatabaseId, EntryField, EntryFields, EntryId,
    EntrySnapshot, FieldValue, Group, GroupId, RevisionId, RevisionKind, SavedRevision, Timestamp,
    ValidationError, validate_group_name,
};

mod codec;
mod fields;
mod groups;
mod projection;

use codec::{decode, encode, object, unique, unique_optional};
use fields::{apply_form, put_field, validate_field};
use groups::{GroupPlacement, put_group_name, read_groups};
use projection::read_entry;

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

#[cfg(test)]
mod tests;
