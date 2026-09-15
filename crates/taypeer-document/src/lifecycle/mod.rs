//! Lifecycle scenarios, exact selection and durable operation identities.

mod commands;
mod recovery;
#[cfg(test)]
mod tests;
pub use commands::{GroupMove, SiblingPosition};
pub use recovery::{RecoveryMode, RecoveryRequest, SourcePreview};

use super::objects::{self, PurgeRecord};
use super::*;
use automerge::transaction::Transaction;
use taypeer_core::OperationId;

/// A lifecycle transition prepared against an explicit document context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    /// Retain selected objects in the trash.
    Trash,
    /// Return selected objects to an explicitly chosen destination.
    Restore,
    /// Close selected trashed generations permanently.
    Purge,
}

/// A reviewable exact selection. Confirmation validates it against its original heads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedLifecycle {
    /// Owning database.
    pub database: DatabaseId,
    /// Original causal context.
    pub heads: Vec<String>,
    /// Requested transition.
    pub action: LifecycleAction,
    /// Selected root object.
    pub target: ObjectId,
    /// Explicit restore destination; None means top-level for groups only.
    pub destination: Option<GroupRef>,
    /// Exact generation addresses and the content events reviewed for each.
    #[serde(with = "selection_serde")]
    pub affected: BTreeMap<ObjectAddress, BTreeSet<String>>,
}

/// One retained object and its product availability. No field values are included.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectState {
    /// Exact lifetime.
    pub address: ObjectAddress,
    /// Product visibility.
    pub status: ObjectStatus,
    /// An explicit review is needed before destructive cleanup.
    pub conflicted: bool,
}

/// A retained late mutation of a closed generation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingSource {
    /// Stable content event identity, also used in processing receipts.
    pub id: String,
    /// Closed target generation.
    pub address: ObjectAddress,
    /// Original Automerge change hash.
    pub change: String,
    /// Original CRDT actor, not a network admission claim.
    pub actor: String,
    /// Source revision, when the event confirms an entry.
    pub revision: Option<RevisionId>,
}

#[derive(Serialize, Deserialize)]
struct ActionReceipt {
    intent: serde_json::Value,
    result: Vec<ObjectId>,
}

impl Document {
    /// Lossless current/trashed group projection. Purged content is excluded.
    pub fn tree(&self) -> Result<Vec<GroupNode>, Error> {
        Ok(groups::tree(&self.doc)?
            .into_iter()
            .filter(|n| n.status != ObjectStatus::Purged)
            .collect())
    }

    /// Read retained object availability, including placement and generation conflicts.
    pub fn object_states(&self) -> Result<Vec<ObjectState>, Error> {
        let mut result = Vec::new();
        for address in objects::all(&self.doc)? {
            let (status, conflicted) = self.object_status(&address)?;
            if status != ObjectStatus::Purged {
                result.push(ObjectState {
                    address,
                    status,
                    conflicted,
                });
            }
        }
        Ok(result)
    }

    /// Read a retained entry generation explicitly (for trash and conflict review).
    /// Closed generations cannot be inspected through this API.
    pub fn inspect_entry(&self, address: &ObjectAddress) -> Result<EntrySnapshot, Error> {
        if objects::purge(&self.doc, address)?.is_some() {
            return Err(Error::NotFound);
        }
        projection::read_entry_generation(&self.doc, address, None)
    }

    pub(super) fn object_status(
        &self,
        address: &ObjectAddress,
    ) -> Result<(ObjectStatus, bool), Error> {
        let tree = groups::tree(&self.doc)?;
        if matches!(address.object, ObjectId::Group(_)) {
            let node = tree
                .iter()
                .find(|n| &n.address == address)
                .ok_or(Error::NotFound)?;
            return Ok((node.status, node.conflicted));
        }
        let (mut status, mut conflict) = groups::own_status(&self.doc, address)?;
        if status == ObjectStatus::Purged {
            return Ok((status, conflict));
        }
        let snapshot = projection::read_entry_generation(&self.doc, address, None)?;
        conflict |= snapshot.has_conflicts();
        let current = objects::current(&self.doc, &address.object)?;
        if current.len() != 1 || !current.contains(address) {
            conflict = true;
            if status == ObjectStatus::Active {
                status = ObjectStatus::Unplaced;
            }
        }
        if snapshot.placements.len() != 1 && status == ObjectStatus::Active {
            status = ObjectStatus::Unplaced;
        }
        for placement in &snapshot.placements {
            let parent = ObjectAddress::from(placement.clone());
            match tree.iter().find(|n| n.address == parent) {
                Some(parent) if parent.status == ObjectStatus::Trashed => {
                    if status != ObjectStatus::Trashed {
                        status = ObjectStatus::Trashed;
                        conflict = true;
                    }
                }
                Some(parent)
                    if parent.status == ObjectStatus::Active
                        && parent.current
                        && !parent.placement_conflict => {}
                _ => {
                    conflict = true;
                    if status == ObjectStatus::Active {
                        status = ObjectStatus::Unplaced;
                    }
                }
            }
        }
        Ok((status, conflict))
    }

    pub(super) fn require_active_entry(&self, id: &EntryId) -> Result<ObjectAddress, Error> {
        let address = objects::single(&self.doc, &ObjectId::Entry(id.clone()))?;
        if self.object_status(&address)?.0 != ObjectStatus::Active {
            return Err(Error::Conflict);
        }
        Ok(address)
    }

    fn destination(&self, id: Option<GroupId>) -> Result<Option<GroupRef>, Error> {
        id.map(|id| {
            self.require_group(&id)?;
            objects::single(&self.doc, &ObjectId::Group(id))?.group_ref()
        })
        .transpose()
    }

    fn at_heads(&self, heads: &[String]) -> Result<Self, Error> {
        let hashes = parse_heads(&self.doc, heads)?;
        let doc = self.doc.fork_at(&hashes)?;
        Ok(Self {
            database_id: self.database_id.clone(),
            name: self.name.clone(),
            doc,
            writer: self.writer,
        })
    }

    /// Causal context for a later explicit placement or generation resolution.
    pub fn review_heads(&self) -> Vec<String> {
        self.heads()
    }

    /// Causal heads for an encrypted checkpoint or an editor's exact base.
    pub fn heads(&self) -> Vec<String> {
        self.doc
            .get_heads()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn selected_subtree(&self, root: &ObjectAddress) -> Result<BTreeSet<ObjectAddress>, Error> {
        objects::generation_object(&self.doc, root)?;
        let mut selected = BTreeSet::from([root.clone()]);
        if matches!(root.object, ObjectId::Entry(_)) {
            return Ok(selected);
        }
        let groups = groups::tree(&self.doc)?;
        loop {
            let before = selected.len();
            for node in &groups {
                if node.status != ObjectStatus::Purged
                    && node.placements.iter().any(|p| {
                        p.parent
                            .as_ref()
                            .is_some_and(|p| selected.contains(&ObjectAddress::from(p.clone())))
                    })
                {
                    selected.insert(node.address.clone());
                }
            }
            if selected.len() == before {
                break;
            }
        }
        for address in objects::all(&self.doc)? {
            if matches!(address.object, ObjectId::Entry(_))
                && objects::purge(&self.doc, &address)?.is_none()
            {
                let entry = projection::read_entry_generation(&self.doc, &address, None)?;
                if entry
                    .placements
                    .iter()
                    .any(|p| selected.contains(&ObjectAddress::from(p.clone())))
                {
                    selected.insert(address);
                }
            }
        }
        Ok(selected)
    }

    /// Prepare a deterministic exact subtree selection; no state is changed.
    pub fn prepare_lifecycle(
        &self,
        action: LifecycleAction,
        target: ObjectId,
        destination: Option<GroupId>,
    ) -> Result<PreparedLifecycle, Error> {
        let address = objects::single(&self.doc, &target)?;
        let destination = if action == LifecycleAction::Restore {
            self.destination(destination)?
        } else {
            if destination.is_some() {
                return Err(Error::InvalidContext);
            }
            None
        };
        if action == LifecycleAction::Restore
            && matches!(target, ObjectId::Entry(_))
            && destination.is_none()
        {
            return Err(Error::InvalidContext);
        }
        let mut affected = BTreeMap::new();
        for address in self.selected_subtree(&address)? {
            let (status, conflict) = self.object_status(&address)?;
            if status == ObjectStatus::Purged {
                return Err(Error::NotFound);
            }
            if matches!(action, LifecycleAction::Restore | LifecycleAction::Purge)
                && status != ObjectStatus::Trashed
            {
                return Err(Error::InvalidContext);
            }
            if action == LifecycleAction::Purge && conflict {
                return Err(Error::Conflict);
            }
            affected.insert(address.clone(), objects::events(&self.doc, &address)?);
        }
        if destination
            .as_ref()
            .is_some_and(|p| affected.contains_key(&ObjectAddress::from(p.clone())))
        {
            return Err(Error::InvalidContext);
        }
        Ok(PreparedLifecycle {
            database: self.database_id.clone(),
            heads: self.heads(),
            action,
            target,
            destination,
            affected,
        })
    }

    pub(super) fn action_receipt(
        &self,
        operation: &OperationId,
        intent: &impl Serialize,
    ) -> Result<Option<Vec<ObjectId>>, Error> {
        if operation.as_str().is_empty() {
            return Err(Error::InvalidContext);
        }
        let old = object(&self.doc, &ROOT, "operations")?;
        if !self.doc.get_all(old, operation.as_str())?.is_empty() {
            return Err(Error::DuplicateId);
        }
        let root = object(&self.doc, &ROOT, "lifecycle_receipts")?;
        let Some(value) = unique_optional(&self.doc, &root, operation.as_str())? else {
            return Ok(None);
        };
        let receipt: ActionReceipt = decode(&value)?;
        let intent = serde_json::to_value(intent).map_err(|_| Error::InvalidDocument)?;
        if receipt.intent != intent {
            return Err(Error::DuplicateId);
        }
        Ok(Some(receipt.result))
    }

    pub(super) fn validate_structure(&self) -> Result<(), Error> {
        groups::tree(&self.doc)?;
        let mut owners = BTreeMap::new();
        let mut attachment_owners = BTreeMap::new();
        for address in objects::all(&self.doc)? {
            objects::life(&self.doc, &address)?;
            objects::purge(&self.doc, &address)?;
            if let ObjectId::Entry(id) = &address.object {
                projection::read_entry_generation(&self.doc, &address, None)?;
                let entry = objects::generation_object(&self.doc, &address)?;
                let attributes = object(&self.doc, &entry, "attributes")?;
                for attribute in self.doc.keys(attributes) {
                    if owners
                        .insert(attribute, id.clone())
                        .is_some_and(|owner| &owner != id)
                    {
                        return Err(Error::DuplicateId);
                    }
                }
                let attachments = object(&self.doc, &entry, "attachments")?;
                for attachment in self.doc.keys(attachments) {
                    if attachment_owners
                        .insert(attachment, id.clone())
                        .is_some_and(|owner| &owner != id)
                    {
                        return Err(Error::DuplicateId);
                    }
                }
                stored_revisions(&self.doc, id)?;
            }
        }
        for root_name in ["events", "purges", "recoveries", "lifecycle_receipts"] {
            object(&self.doc, &ROOT, root_name)?;
        }
        Ok(())
    }
}

pub(super) fn parse_heads(doc: &Automerge, heads: &[String]) -> Result<Vec<ChangeHash>, Error> {
    let hashes: Vec<_> = heads
        .iter()
        .map(|h| h.parse().map_err(|_| Error::InvalidContext))
        .collect::<Result<_, _>>()?;
    if hashes.is_empty() || hashes.iter().any(|h| doc.get_change_by_hash(h).is_none()) {
        return Err(Error::InvalidContext);
    }
    Ok(hashes)
}

pub(super) fn put_receipt(
    tx: &mut Transaction<'_>,
    operation: &OperationId,
    intent: &impl Serialize,
    result: &[ObjectId],
) -> Result<(), Error> {
    let root = object(tx, &ROOT, "lifecycle_receipts")?;
    tx.put(
        root,
        operation.as_str(),
        encode(&ActionReceipt {
            intent: serde_json::to_value(intent).map_err(|_| Error::InvalidDocument)?,
            result: result.to_vec(),
        })?,
    )?;
    Ok(())
}

fn confirm_revision(
    tx: &mut Transaction<'_>,
    address: &ObjectAddress,
    kind: RevisionKind,
    now: Timestamp,
    base: &[String],
    revision_id: RevisionId,
) -> Result<(), Error> {
    let ObjectId::Entry(id) = &address.object else {
        return Ok(());
    };
    let target = objects::generation_object(tx, address)?;
    let mut changed = BTreeSet::new();
    for (_, operation) in tx.get_all(&target, "group")? {
        if tx.hash_for_opid(&operation).is_none() {
            changed.insert(operation.to_string());
        }
    }
    let preliminary = projection::read_entry_generation(tx, address, None)?;
    changed.extend(fields::pending_operations(
        tx,
        &target,
        &preliminary.values,
    )?);
    let snapshot = projection::read_entry_generation(tx, address, Some((&changed, now)))?;
    let stored = StoredRevision {
        revision: SavedRevision {
            id: revision_id.clone(),
            entry_id: id.clone(),
            saved_at: now,
            kind,
            base: base.to_vec(),
            snapshot,
        },
        changed_operations: changed,
        submitted: None,
    };
    let root = object(tx, &ROOT, "revisions")?;
    tx.put(root, revision_id.as_str(), encode(&stored)?)?;
    objects::record_event(tx, address, Some(revision_id))
}

mod selection_serde {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &BTreeMap<ObjectAddress, BTreeSet<String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<ObjectAddress, BTreeSet<String>>, D::Error> {
        let pairs = Vec::<(ObjectAddress, BTreeSet<String>)>::deserialize(deserializer)?;
        let mut result = BTreeMap::new();
        for (address, events) in pairs {
            if result.insert(address, events).is_some() {
                return Err(serde::de::Error::custom("duplicate selection"));
            }
        }
        Ok(result)
    }
}
