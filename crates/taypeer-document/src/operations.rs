//! Idempotent confirmation, historical restoration and observed-context resolution.

use super::*;
use taypeer_core::OperationId;

/// The exact document context displayed during conflict review.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictContext {
    /// Database identity prevents cross-database use.
    pub database: DatabaseId,
    /// Entry being reviewed.
    pub entry: EntryId,
    /// Causal heads, not display timestamps.
    pub heads: Vec<String>,
}

/// An explicit replacement for one field at an observed conflict context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// Field to resolve.
    pub field: EntryField,
    /// Whole chosen value; protected value and protection are atomic.
    pub value: FieldValue,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum Intent {
    Clone {
        source: EntryId,
        group: GroupId,
        title: Option<String>,
    },
    Restore {
        entry: EntryId,
        revision: RevisionId,
        group: GroupId,
    },
    Resolve {
        entry: EntryId,
        heads: Vec<String>,
        fields: Vec<Resolution>,
    },
    PurgeHistory {
        entry: EntryId,
        revisions: BTreeSet<RevisionId>,
    },
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    intent: Intent,
    result: EntryId,
}

impl Document {
    /// Capture the context for a later explicit conflict resolution.
    pub fn conflict_context(&self, entry: &EntryId) -> Result<ConflictContext, Error> {
        self.entry(entry)?;
        Ok(ConflictContext {
            database: self.database_id.clone(),
            entry: entry.clone(),
            heads: self
                .doc
                .get_heads()
                .iter()
                .map(ToString::to_string)
                .collect(),
        })
    }

    fn receipt(&self, operation: &OperationId, intent: &Intent) -> Result<Option<EntryId>, Error> {
        if operation.as_str().is_empty() {
            return Err(Error::InvalidContext);
        }
        let newer = object(&self.doc, &ROOT, "lifecycle_receipts")?;
        if !self.doc.get_all(newer, operation.as_str())?.is_empty() {
            return Err(Error::DuplicateId);
        }
        let root = object(&self.doc, &ROOT, "operations")?;
        let Some(value) = unique_optional(&self.doc, &root, operation.as_str())? else {
            return Ok(None);
        };
        let receipt: Receipt = decode(&value)?;
        if &receipt.intent != intent {
            return Err(Error::DuplicateId);
        }
        Ok(Some(receipt.result))
    }

    /// Clone the current unambiguous entry with fresh entry and attribute identities.
    /// Retrying the same operation returns the original clone; a different intent is rejected.
    pub fn clone_entry(
        &mut self,
        source: &EntryId,
        group: GroupId,
        title: Option<String>,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<EntryId, Error> {
        let intent = Intent::Clone {
            source: source.clone(),
            group: group.clone(),
            title: title.clone(),
        };
        if let Some(result) = self.receipt(operation, &intent)? {
            return Ok(result);
        }
        let mut fields = self.entry(source)?.fields.ok_or(Error::Conflict)?;
        if let Some(title) = title {
            fields.title = title;
        }
        fields.attributes = fields
            .attributes
            .into_values()
            .map(|mut attribute| {
                attribute.id = AttributeId::new(random_id());
                (attribute.id.clone(), attribute)
            })
            .collect();
        let mut candidate = self.clone();
        let mut draft = candidate.begin_create_entry(group)?;
        draft.fields = fields;
        let id = candidate.confirm_entry(
            draft,
            now,
            Some(RevisionKind::Clone),
            Some((operation, &intent)),
        )?;
        *self = candidate;
        Ok(id)
    }

    /// Restore an accessible historical revision as a new confirmation, retaining its identity.
    /// The destination is explicit and must be a current group. Ambiguous sources require review.
    pub fn restore_revision(
        &mut self,
        entry: &EntryId,
        revision: &RevisionId,
        group: GroupId,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<EntryId, Error> {
        let intent = Intent::Restore {
            entry: entry.clone(),
            revision: revision.clone(),
            group: group.clone(),
        };
        if let Some(result) = self.receipt(operation, &intent)? {
            return Ok(result);
        }
        self.require_group(&group)?;
        let source = self
            .history(entry)?
            .into_iter()
            .find(|r| &r.id == revision)
            .ok_or(Error::NotFound)?;
        let fields = source.snapshot.fields.ok_or(Error::Conflict)?;
        let mut candidate = self.clone();
        let before = candidate.entry(entry)?;
        // Restoration must create a revision even if its values equal the current state.
        let resolutions = all_resolutions(&fields, &before);
        let context = candidate.conflict_context(entry)?;
        candidate.confirm_resolution(
            &context,
            &resolutions,
            RevisionKind::Restore,
            now,
            operation,
            &intent,
        )?;
        *self = candidate;
        Ok(entry.clone())
    }

    /// Resolve only fields observed at `context`; later unseen values remain conflicting.
    pub fn resolve_fields(
        &mut self,
        context: &ConflictContext,
        fields: Vec<Resolution>,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<EntryId, Error> {
        if context.database != self.database_id {
            return Err(Error::InvalidContext);
        }
        let intent = Intent::Resolve {
            entry: context.entry.clone(),
            heads: context.heads.clone(),
            fields: fields.clone(),
        };
        if let Some(result) = self.receipt(operation, &intent)? {
            return Ok(result);
        }
        if fields.is_empty() {
            return Err(Error::InvalidContext);
        }
        let mut candidate = self.clone();
        candidate.confirm_resolution(
            context,
            &fields,
            RevisionKind::Resolve,
            now,
            operation,
            &intent,
        )?;
        *self = candidate;
        Ok(context.entry.clone())
    }

    fn confirm_resolution(
        &mut self,
        context: &ConflictContext,
        fields: &[Resolution],
        kind: RevisionKind,
        now: Timestamp,
        operation: &OperationId,
        intent: &Intent,
    ) -> Result<(), Error> {
        if context.database != self.database_id {
            return Err(Error::InvalidContext);
        }
        let base: Vec<ChangeHash> = context
            .heads
            .iter()
            .map(|head| head.parse().map_err(|_| Error::InvalidContext))
            .collect::<Result<_, _>>()?;
        if base
            .iter()
            .any(|head| self.doc.get_change_by_hash(head).is_none())
        {
            return Err(Error::InvalidContext);
        }
        let mut unique_fields = BTreeSet::new();
        if base.is_empty() {
            return Err(Error::InvalidContext);
        }
        let basis = self.doc.fork_at(&base)?;
        let address = objects::single(&basis, &ObjectId::Entry(context.entry.clone()))?;
        if self.require_active_entry(&context.entry)? != address {
            return Err(Error::InvalidContext);
        }
        for resolution in fields {
            if !unique_fields.insert(&resolution.field) {
                return Err(Error::InvalidContext);
            }
            validate_field(&resolution.field, &resolution.value)?;
        }
        let revision = RevisionId::new(random_id());
        let new_attributes = if matches!(intent, Intent::Restore { .. }) {
            self.restored_attributes(&context.entry, fields)?
        } else {
            BTreeSet::new()
        };
        let mut tx = self.doc.transaction_at(PatchLog::null(), &base);
        let target = objects::entry_object(&tx, &context.entry)?;
        let attributes = object(&tx, &target, "attributes")?;
        for id in new_attributes {
            tx.put_object(&attributes, id.as_str(), ObjType::Map)?;
        }
        if let Intent::Restore { group, .. } = intent {
            let destination = objects::single(&tx, &ObjectId::Group(group.clone()))?.group_ref()?;
            tx.put(&target, "group", encode(&destination)?)?;
        }
        let mut changed = BTreeSet::new();
        for resolution in fields {
            put_field(
                &mut tx,
                &target,
                &resolution.field,
                resolution.value.clone(),
                &revision,
                &mut changed,
            )?;
        }
        let snapshot = read_entry(&tx, &context.entry, Some((&changed, now)))?;
        let stored = StoredRevision {
            revision: SavedRevision {
                id: revision.clone(),
                entry_id: context.entry.clone(),
                saved_at: now,
                kind,
                base: context.heads.clone(),
                snapshot,
            },
            changed_operations: changed,
            submitted: None,
        };
        let revisions = object(&tx, &ROOT, "revisions")?;
        tx.put(revisions, revision.as_str(), encode(&stored)?)?;
        let address = objects::single(&tx, &ObjectId::Entry(context.entry.clone()))?;
        objects::record_event(&mut tx, &address, Some(revision.clone()))?;
        write_receipt(&mut tx, operation, intent, &context.entry)?;
        tx.commit();
        read_entry(&self.doc, &context.entry, None)?;
        Ok(())
    }

    fn restored_attributes(
        &self,
        entry: &EntryId,
        fields: &[Resolution],
    ) -> Result<BTreeSet<AttributeId>, Error> {
        let target = objects::entry_object(&self.doc, entry)?;
        let attributes = object(&self.doc, &target, "attributes")?;
        let mut new = BTreeSet::new();
        for resolution in fields {
            if let (EntryField::AttributePresence(id), FieldValue::Presence(true)) =
                (&resolution.field, &resolution.value)
                && self.doc.get_all(&attributes, id.as_str())?.is_empty()
            {
                new.insert(id.clone());
            }
        }
        if new.is_empty() {
            return Ok(new);
        }
        // History can refer to another lifetime, but never borrow another entry's identity.
        for address in objects::all(&self.doc)? {
            if !matches!(&address.object, ObjectId::Entry(id) if id != entry) {
                continue;
            }
            let node = objects::generation_object(&self.doc, &address)?;
            let attributes = object(&self.doc, &node, "attributes")?;
            for id in &new {
                if !self.doc.get_all(&attributes, id.as_str())?.is_empty() {
                    return Err(Error::DuplicateId);
                }
            }
        }
        Ok(new)
    }

    pub(super) fn revision_is_purged(&self, revision: &RevisionId) -> Result<bool, Error> {
        let root = object(&self.doc, &ROOT, "purged_revisions")?;
        Ok(!self.doc.get_all(root, revision.as_str())?.is_empty())
    }

    /// Hide exactly the selected revisions. Current unresolved conflicts prohibit cleanup.
    /// Concurrent unseen revisions are not part of this operation and remain available.
    pub fn purge_history(
        &mut self,
        entry: &EntryId,
        revisions: BTreeSet<RevisionId>,
        operation: &OperationId,
    ) -> Result<(), Error> {
        let intent = Intent::PurgeHistory {
            entry: entry.clone(),
            revisions: revisions.clone(),
        };
        if self.receipt(operation, &intent)?.is_some() {
            return Ok(());
        }
        if self.entry(entry)?.has_conflicts() {
            return Err(Error::Conflict);
        }
        let existing: BTreeSet<_> = self.history(entry)?.into_iter().map(|r| r.id).collect();
        if revisions.is_empty() || !revisions.is_subset(&existing) {
            return Err(Error::NotFound);
        }
        let mut candidate = self.clone();
        let mut tx = candidate.doc.transaction();
        let root = object(&tx, &ROOT, "purged_revisions")?;
        for revision in revisions {
            tx.put(&root, revision.as_str(), operation.as_str())?;
        }
        write_receipt(&mut tx, operation, &intent, entry)?;
        tx.commit();
        *self = candidate;
        Ok(())
    }
}

fn all_resolutions(fields: &EntryFields, before: &EntrySnapshot) -> Vec<Resolution> {
    let mut result = vec![
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
    ];
    for state in &before.values {
        if let EntryField::AttributePresence(id) = &state.field {
            result.push((
                state.field.clone(),
                FieldValue::Presence(fields.attributes.contains_key(id)),
            ));
        }
    }
    for attribute in fields.attributes.values() {
        if !before
            .values
            .iter()
            .any(|state| state.field == EntryField::AttributePresence(attribute.id.clone()))
        {
            result.push((
                EntryField::AttributePresence(attribute.id.clone()),
                FieldValue::Presence(true),
            ));
        }
        result.push((
            EntryField::AttributeName(attribute.id.clone()),
            FieldValue::Text(Some(attribute.name.clone())),
        ));
        result.push((
            EntryField::AttributeValue(attribute.id.clone()),
            FieldValue::Attribute(attribute.value.clone()),
        ));
    }
    result
        .into_iter()
        .map(|(field, value)| Resolution { field, value })
        .collect()
}

pub(super) fn write_receipt(
    tx: &mut automerge::transaction::Transaction<'_>,
    operation: &OperationId,
    intent: &Intent,
    result: &EntryId,
) -> Result<(), Error> {
    let root = object(tx, &ROOT, "operations")?;
    tx.put(
        root,
        operation.as_str(),
        encode(&Receipt {
            intent: intent.clone(),
            result: result.clone(),
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Document, GroupId, EntryId) {
        let mut doc = Document::new("PUBLIC operations", 1).unwrap();
        let group = doc.create_group("PUBLIC group".into(), None, 2).unwrap().id;
        let mut draft = doc.begin_create_entry(group.clone()).unwrap();
        draft.fields_mut().title = "PUBLIC title".into();
        draft.fields_mut().password = Some("PUBLIC original".into());
        draft.add_attribute("PUBLIC attribute".into(), "PUBLIC protected".into(), true);
        let entry = doc.save_entry(draft, 3).unwrap();
        (doc, group, entry)
    }

    #[test]
    fn cloning_and_restoration_have_new_versions_and_durable_retry_identity() {
        let (mut doc, group, entry) = fixture();
        let original = doc.history(&entry).unwrap()[0].clone();
        let operation = OperationId::new("PUBLIC clone");
        let clone = doc
            .clone_entry(&entry, group.clone(), None, &operation, 4)
            .unwrap();
        assert_ne!(clone, entry);
        assert_eq!(doc.history(&clone).unwrap()[0].kind, RevisionKind::Clone);
        let source_attributes = original
            .snapshot
            .fields
            .as_ref()
            .unwrap()
            .attributes
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let cloned_attributes = doc
            .entry(&clone)
            .unwrap()
            .fields
            .unwrap()
            .attributes
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert!(source_attributes.is_disjoint(&cloned_attributes));
        let mut reopened = Document::load(&doc.export()).unwrap();
        assert_eq!(
            reopened
                .clone_entry(&entry, group.clone(), None, &operation, 5)
                .unwrap(),
            clone
        );
        assert_eq!(reopened.entries().unwrap().len(), 2);
        assert_eq!(
            reopened
                .clone_entry(
                    &entry,
                    group.clone(),
                    Some("PUBLIC different".into()),
                    &operation,
                    6
                )
                .unwrap_err(),
            Error::DuplicateId
        );
        let mut draft = reopened.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().password = Some("PUBLIC updated".into());
        draft.fields_mut().attributes.clear();
        reopened.save_entry(draft, 7).unwrap();
        let group = reopened
            .create_group("PUBLIC destination".into(), None, 7)
            .unwrap()
            .id;
        let restore = OperationId::new("PUBLIC restore");
        reopened
            .restore_revision(&entry, &original.id, group.clone(), &restore, 8)
            .unwrap();
        assert_eq!(
            reopened.entry(&entry).unwrap().fields,
            original.snapshot.fields
        );
        assert_eq!(
            reopened.history(&entry).unwrap().last().unwrap().kind,
            RevisionKind::Restore
        );
        reopened
            .restore_revision(&entry, &original.id, group, &restore, 9)
            .unwrap();
        assert_eq!(reopened.history(&entry).unwrap().len(), 3);
        let restored = reopened.history(&entry).unwrap().pop().unwrap();
        assert_eq!(
            reopened.entry(&entry).unwrap().group_id,
            restored.snapshot.group_id
        );
        assert_ne!(original.snapshot.group_id, restored.snapshot.group_id);
    }

    #[test]
    fn resolution_does_not_consume_a_variant_that_arrived_after_review() {
        let (mut left, _, entry) = fixture();
        let mut right = left.fork();
        let mut late = left.fork();
        for (doc, text) in [
            (&mut left, "PUBLIC left"),
            (&mut right, "PUBLIC right"),
            (&mut late, "PUBLIC late"),
        ] {
            let mut draft = doc.begin_edit_entry(&entry).unwrap();
            draft.fields_mut().password = Some(text.into());
            doc.save_entry(draft, 10).unwrap();
        }
        left.merge(&right).unwrap();
        let context = left.conflict_context(&entry).unwrap();
        left.merge(&late).unwrap();
        left.resolve_fields(
            &context,
            vec![Resolution {
                field: EntryField::Password,
                value: FieldValue::Text(Some("PUBLIC chosen".into())),
            }],
            &OperationId::new("PUBLIC resolve"),
            11,
        )
        .unwrap();
        let entry_view = left.entry(&entry).unwrap();
        let values = &entry_view
            .conflicts
            .iter()
            .find(|s| s.field == EntryField::Password)
            .unwrap()
            .variants;
        assert_eq!(values.len(), 2);
        assert!(
            values
                .iter()
                .any(|v| v.value == FieldValue::Text(Some("PUBLIC late".into())))
        );
        assert!(
            values
                .iter()
                .any(|v| v.value == FieldValue::Text(Some("PUBLIC chosen".into())))
        );
        assert_eq!(left.history(&entry).unwrap().len(), 5);
        assert!(
            left.purge_history(
                &entry,
                left.history(&entry)
                    .unwrap()
                    .into_iter()
                    .map(|r| r.id)
                    .collect(),
                &OperationId::new("PUBLIC blocked purge")
            )
            .is_err()
        );
    }

    #[test]
    fn history_purge_survives_old_replica_delivery_and_keeps_unseen_revisions() {
        let (mut left, group, entry) = fixture();
        let old = left.fork();
        let mut right = left.fork();
        let removed = left.history(&entry).unwrap()[0].id.clone();
        let mut draft = right.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().notes = Some("PUBLIC unseen".into());
        right.save_entry(draft, 20).unwrap();
        let operation = OperationId::new("PUBLIC purge");
        left.purge_history(&entry, BTreeSet::from([removed.clone()]), &operation)
            .unwrap();
        left.merge(&old).unwrap();
        assert!(left.history(&entry).unwrap().is_empty());
        left.merge(&right).unwrap();
        assert_eq!(left.history(&entry).unwrap().len(), 1);
        assert_ne!(left.history(&entry).unwrap()[0].id, removed);
        left.purge_history(&entry, BTreeSet::from([removed.clone()]), &operation)
            .unwrap();
        assert_eq!(
            left.restore_revision(
                &entry,
                &removed,
                group,
                &OperationId::new("PUBLIC rejected restore"),
                30
            )
            .unwrap_err(),
            Error::NotFound
        );
    }
}
