//! Addressed binary references, lifetime-aware retention and operation receipts.

use super::*;
use automerge::{ObjId, transaction::Transaction};
use taypeer_core::OperationId;

/// Rebuildable references for quota accounting and physical retention.
#[derive(Clone, Debug, Default)]
pub struct BlobReferences {
    /// Unique logical aliases used by accessible attachment variants and revisions.
    pub attachments: BTreeSet<BlobId>,
    /// All required aliases, including icons and unprocessed late sources.
    pub retained: BTreeSet<BlobId>,
    /// References required for accessible data, excluding waiting late sources.
    pub required: BTreeSet<BlobId>,
    /// An unreadable pending source prevents deleting potentially needed contents.
    pub unknown: bool,
}
impl BlobReferences {
    fn snapshot(&mut self, snapshot: &EntrySnapshot, quota: bool) {
        for state in &snapshot.values {
            for variant in &state.variants {
                match &variant.value {
                    FieldValue::Blob(id) => {
                        self.retained.insert(id.clone());
                        if quota {
                            self.attachments.insert(id.clone());
                        }
                    }
                    FieldValue::Icon(icon) => {
                        if let Some(id) = icon.blob() {
                            self.retained.insert(id.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

impl Document {
    /// Compute references from accessible generations and history, never stale CRDT scalar bytes.
    pub fn blob_references(&self) -> Result<BlobReferences, Error> {
        let mut refs = BlobReferences::default();
        for address in objects::all(&self.doc)? {
            if objects::purge(&self.doc, &address)?.is_some() {
                continue;
            }
            match &address.object {
                ObjectId::Group(_) => {
                    for icon in groups::read_group(&self.doc, &address)?.icons {
                        if let Some(id) = icon.blob() {
                            refs.retained.insert(id.clone());
                        }
                    }
                }
                ObjectId::Entry(id) => {
                    refs.snapshot(
                        &projection::read_entry_generation(&self.doc, &address, None)?,
                        true,
                    );
                    for stored in stored_revisions(&self.doc, id)? {
                        if stored.revision.snapshot.generation == address.generation
                            && !self.revision_is_purged(&stored.revision.id)?
                        {
                            refs.snapshot(&stored.revision.snapshot, true);
                        }
                    }
                }
            }
        }
        refs.required = refs.retained.clone();
        for source in self.pending_sources()? {
            match self.preview_source(&source.id) {
                Ok(SourcePreview::Entry(snapshot)) => refs.snapshot(&snapshot, false),
                Ok(SourcePreview::Group(group)) => {
                    for icon in group.icons {
                        if let Some(id) = icon.blob() {
                            refs.retained.insert(id.clone());
                        }
                    }
                }
                Err(_) => refs.unknown = true,
            }
        }
        Ok(refs)
    }

    /// Return a previously committed binary command result before repeating external I/O.
    /// Intent is the adapter's exact serialized request, stored only inside the encrypted document.
    pub fn binary_receipt(
        &self,
        operation: &OperationId,
        intent: &serde_json::Value,
    ) -> Result<Option<Vec<ObjectId>>, Error> {
        let old = object(&self.doc, &ROOT, "operations")?;
        if !self.doc.get_all(old, operation.as_str())?.is_empty() {
            return Ok(self
                .receipt(
                    operation,
                    &operations::Intent::Binary {
                        request: intent.clone(),
                    },
                )?
                .map(|id| vec![ObjectId::Entry(id)]));
        }
        self.action_receipt(operation, &("binary", intent))
    }

    /// Commit an idempotent storage operation marker without adding user history.
    pub fn record_binary_operation(
        &mut self,
        operation: &OperationId,
        intent: &serde_json::Value,
    ) -> Result<(), Error> {
        if self.binary_receipt(operation, intent)?.is_none() {
            let mut tx = self.doc.transaction();
            lifecycle::put_receipt(&mut tx, operation, &("binary", intent), &[])?;
            tx.commit();
        }
        Ok(())
    }
    /// Confirm a binary-bearing draft and its retry receipt in the same candidate document.
    pub fn save_binary_entry(
        &mut self,
        draft: EntryDraft,
        operation: &OperationId,
        intent: &serde_json::Value,
        now: Timestamp,
    ) -> Result<EntryId, Error> {
        if let Some(result) = self.binary_receipt(operation, intent)? {
            return match result.as_slice() {
                [ObjectId::Entry(id)] => Ok(id.clone()),
                _ => Err(Error::InvalidContext),
            };
        }
        let mut candidate = self.clone();
        let id = candidate.confirm_entry(
            draft,
            now,
            None,
            Some((
                operation,
                &operations::Intent::Binary {
                    request: intent.clone(),
                },
            )),
        )?;
        *self = candidate;
        Ok(id)
    }

    /// Set or explicitly resolve a group icon at reviewed heads, preserving unseen alternatives.
    pub fn set_group_icon(
        &mut self,
        group: &GroupId,
        icon: IconRef,
        review: Option<&[String]>,
        operation: &OperationId,
        intent: &serde_json::Value,
        now: Timestamp,
    ) -> Result<(), Error> {
        if self.binary_receipt(operation, intent)?.is_some() {
            return Ok(());
        }
        self.require_group(group)?;
        let base = review
            .map(|heads| lifecycle::parse_heads(&self.doc, heads))
            .transpose()?
            .unwrap_or_else(|| self.doc.get_heads());
        let basis = self.doc.fork_at(&base)?;
        let address = objects::single(&basis, &ObjectId::Group(group.clone()))?;
        if objects::single(&self.doc, &ObjectId::Group(group.clone()))? != address {
            return Err(Error::InvalidContext);
        }
        let mut candidate = self.clone();
        let mut tx = candidate.doc.transaction_at(PatchLog::null(), &base);
        let node = objects::generation_object(&tx, &address)?;
        tx.put(&node, "icon", encode(&icon)?)?;
        // Record the content event so concurrent trash/purge retains this change as a late source.
        tx.put(&node, "icon_modified_at", now)?;
        objects::record_event(&mut tx, &address, None)?;
        lifecycle::put_receipt(
            &mut tx,
            operation,
            &("binary", intent),
            &[ObjectId::Group(group.clone())],
        )?;
        tx.commit();
        *self = candidate;
        Ok(())
    }
}

pub(super) fn apply_attachments(
    tx: &mut Transaction<'_>,
    entry: &ObjId,
    original: Option<&EntryFields>,
    fields: &EntryFields,
    revision: &RevisionId,
    changed: &mut BTreeSet<String>,
) -> Result<(), Error> {
    let attachments = object(tx, entry, "attachments")?;
    for (id, attachment) in &fields.attachments {
        let old = original.and_then(|fields| fields.attachments.get(id));
        if old == Some(attachment) {
            continue;
        }
        if old.is_none() {
            if !tx.get_all(&attachments, id.as_str())?.is_empty() {
                return Err(Error::DuplicateId);
            }
            tx.put_object(&attachments, id.as_str(), ObjType::Map)?;
        }
        if old.is_none_or(|old| old.name != attachment.name) {
            put_field(
                tx,
                entry,
                &EntryField::AttachmentName(id.clone()),
                FieldValue::Text(Some(attachment.name.clone())),
                revision,
                changed,
            )?;
        }
        if old.is_none_or(|old| old.blob != attachment.blob) {
            put_field(
                tx,
                entry,
                &EntryField::AttachmentBlob(id.clone()),
                FieldValue::Blob(attachment.blob.clone()),
                revision,
                changed,
            )?;
        }
        put_field(
            tx,
            entry,
            &EntryField::AttachmentPresence(id.clone()),
            FieldValue::Presence(true),
            revision,
            changed,
        )?;
    }
    if let Some(original) = original {
        for id in original
            .attachments
            .keys()
            .filter(|id| !fields.attachments.contains_key(*id))
        {
            put_field(
                tx,
                entry,
                &EntryField::AttachmentPresence(id.clone()),
                FieldValue::Presence(false),
                revision,
                changed,
            )?;
        }
    }
    Ok(())
}

pub(super) fn renew_attachment_ids(fields: &mut EntryFields) {
    fields.attachments = std::mem::take(&mut fields.attachments)
        .into_values()
        .map(|mut attachment| {
            attachment.id = AttachmentId::new(random_id());
            (attachment.id.clone(), attachment)
        })
        .collect();
}

pub(super) fn validate_attachment_owners(
    read: &Automerge,
    owner: &EntryId,
    fields: &EntryFields,
) -> Result<(), Error> {
    if fields.attachments.is_empty() {
        return Ok(());
    }
    for address in objects::all(read)? {
        if matches!(&address.object, ObjectId::Entry(id) if id != owner) {
            let node = objects::generation_object(read, &address)?;
            let attachments = object(read, &node, "attachments")?;
            for id in fields.attachments.keys() {
                if !read.get_all(&attachments, id.as_str())?.is_empty() {
                    return Err(Error::DuplicateId);
                }
            }
        }
    }
    Ok(())
}

pub(super) fn attachment_resolutions(
    fields: &EntryFields,
    before: &EntrySnapshot,
    result: &mut Vec<(EntryField, FieldValue)>,
) {
    for state in &before.values {
        if let EntryField::AttachmentPresence(id) = &state.field {
            result.push((
                state.field.clone(),
                FieldValue::Presence(fields.attachments.contains_key(id)),
            ));
        }
    }
    for attachment in fields.attachments.values() {
        if !before
            .values
            .iter()
            .any(|state| state.field == EntryField::AttachmentPresence(attachment.id.clone()))
        {
            result.push((
                EntryField::AttachmentPresence(attachment.id.clone()),
                FieldValue::Presence(true),
            ));
        }
        result.push((
            EntryField::AttachmentName(attachment.id.clone()),
            FieldValue::Text(Some(attachment.name.clone())),
        ));
        result.push((
            EntryField::AttachmentBlob(attachment.id.clone()),
            FieldValue::Blob(attachment.blob.clone()),
        ));
    }
}

pub(super) fn restored_attachments(
    read: &Automerge,
    entry: &EntryId,
    fields: &[Resolution],
) -> Result<BTreeSet<AttachmentId>, Error> {
    let node = objects::entry_object(read, entry)?;
    let attachments = object(read, &node, "attachments")?;
    let mut new = BTreeSet::new();
    for resolution in fields {
        if let (EntryField::AttachmentPresence(id), FieldValue::Presence(true)) =
            (&resolution.field, &resolution.value)
            && read.get_all(&attachments, id.as_str())?.is_empty()
        {
            new.insert(id.clone());
        }
    }
    for address in objects::all(read)? {
        if matches!(&address.object, ObjectId::Entry(id) if id != entry) {
            let node = objects::generation_object(read, &address)?;
            let attachments = object(read, &node, "attachments")?;
            for id in &new {
                if !read.get_all(&attachments, id.as_str())?.is_empty() {
                    return Err(Error::DuplicateId);
                }
            }
        }
    }
    Ok(new)
}
