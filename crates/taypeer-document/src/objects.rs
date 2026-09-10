//! Generation addressing and immutable mutation provenance; no presentation decisions.

use super::{
    Error,
    codec::{decode, encode, object, unique},
    random_id,
};
use automerge::{
    ObjId, ObjType, ROOT, ReadDoc,
    transaction::{Transactable, Transaction},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use taypeer_core::{EntryId, GenerationId, GroupId, GroupRef, OperationId, RevisionId};

/// Public object identity, independent of its state generation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ObjectId {
    /// A group.
    Group(GroupId),
    /// An entry.
    Entry(EntryId),
}
impl ObjectId {
    pub(super) fn root(&self) -> &'static str {
        match self {
            Self::Group(_) => "groups",
            Self::Entry(_) => "entries",
        }
    }
    /// Stable identity text, never an object name.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Group(id) => id.as_str(),
            Self::Entry(id) => id.as_str(),
        }
    }
}

/// A particular lifetime of an object. Recovery preserves identity but changes generation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectAddress {
    /// Public identity.
    pub object: ObjectId,
    /// State lifetime.
    pub generation: GenerationId,
}
impl ObjectAddress {
    pub(super) fn key(&self) -> Result<String, Error> {
        serde_json::to_string(self).map_err(|_| Error::InvalidDocument)
    }
    pub(super) fn group_ref(&self) -> Result<GroupRef, Error> {
        match &self.object {
            ObjectId::Group(id) => Ok(GroupRef {
                id: id.clone(),
                generation: self.generation.clone(),
            }),
            ObjectId::Entry(_) => Err(Error::InvalidContext),
        }
    }
}
impl From<GroupRef> for ObjectAddress {
    fn from(value: GroupRef) -> Self {
        Self {
            object: ObjectId::Group(value.id),
            generation: value.generation,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct LifeMark {
    pub trashed: bool,
    pub operation: OperationId,
    pub observed: BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct MutationEvent {
    pub address: ObjectAddress,
    pub revision: Option<RevisionId>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PurgeRecord {
    pub operation: OperationId,
    pub address: ObjectAddress,
    pub covered: BTreeSet<String>,
}

pub(super) fn shell(read: &impl ReadDoc, id: &ObjectId) -> Result<ObjId, Error> {
    let root = object(read, &ROOT, id.root())?;
    object(read, &root, id.as_str())
}

pub(super) fn current(read: &impl ReadDoc, id: &ObjectId) -> Result<Vec<ObjectAddress>, Error> {
    let shell = shell(read, id)?;
    let mut generations = BTreeSet::new();
    for (value, _) in read.get_all(shell, "current")? {
        generations.insert(GenerationId::new(
            value.to_str().ok_or(Error::InvalidDocument)?,
        ));
    }
    if generations.is_empty() {
        return Err(Error::InvalidDocument);
    }
    let mut result = Vec::new();
    for generation in generations {
        let address = ObjectAddress {
            object: id.clone(),
            generation,
        };
        generation_object(read, &address)?;
        result.push(address);
    }
    Ok(result)
}

pub(super) fn single(read: &impl ReadDoc, id: &ObjectId) -> Result<ObjectAddress, Error> {
    let mut current = current(read, id)?;
    if current.len() != 1 {
        return Err(Error::Conflict);
    }
    current.pop().ok_or(Error::InvalidDocument)
}

pub(super) fn generation_object(
    read: &impl ReadDoc,
    address: &ObjectAddress,
) -> Result<ObjId, Error> {
    let shell = shell(read, &address.object)?;
    if address.generation.as_str() == address.object.as_str() {
        return Ok(shell);
    }
    let generations = object(read, &shell, "generations")?;
    object(read, &generations, address.generation.as_str())
}

pub(super) fn entry_object(read: &impl ReadDoc, id: &EntryId) -> Result<ObjId, Error> {
    generation_object(read, &single(read, &ObjectId::Entry(id.clone()))?)
}

pub(super) fn all(read: &impl ReadDoc) -> Result<Vec<ObjectAddress>, Error> {
    let mut result = Vec::new();
    for root_name in ["groups", "entries"] {
        let root = object(read, &ROOT, root_name)?;
        for id in read.keys(root) {
            let identity = if root_name == "groups" {
                ObjectId::Group(GroupId::new(&id))
            } else {
                ObjectId::Entry(EntryId::new(&id))
            };
            let shell = shell(read, &identity)?;
            result.push(ObjectAddress {
                object: identity.clone(),
                generation: GenerationId::new(&id),
            });
            let generations = object(read, &shell, "generations")?;
            for generation in read.keys(generations) {
                result.push(ObjectAddress {
                    object: identity.clone(),
                    generation: GenerationId::new(generation),
                });
            }
            current(read, &identity)?;
        }
    }
    Ok(result)
}

pub(super) fn initialize(
    tx: &mut Transaction<'_>,
    id: &ObjectId,
) -> Result<(ObjectAddress, ObjId), Error> {
    let root = object(tx, &ROOT, id.root())?;
    if !tx.get_all(&root, id.as_str())?.is_empty() {
        return Err(Error::DuplicateId);
    }
    let node = tx.put_object(root, id.as_str(), ObjType::Map)?;
    tx.put(&node, "current", id.as_str())?;
    tx.put_object(&node, "generations", ObjType::Map)?;
    let address = ObjectAddress {
        object: id.clone(),
        generation: GenerationId::new(id.as_str()),
    };
    put_life(
        tx,
        &node,
        false,
        OperationId::new(random_id()),
        BTreeSet::new(),
    )?;
    Ok((address, node))
}

pub(super) fn put_life(
    tx: &mut Transaction<'_>,
    node: &ObjId,
    trashed: bool,
    operation: OperationId,
    observed: BTreeSet<String>,
) -> Result<(), Error> {
    tx.put(
        node,
        "life",
        encode(&LifeMark {
            trashed,
            operation,
            observed,
        })?,
    )?;
    Ok(())
}

pub(super) fn life(read: &impl ReadDoc, address: &ObjectAddress) -> Result<Vec<LifeMark>, Error> {
    let node = generation_object(read, address)?;
    let values = read.get_all(node, "life")?;
    if values.is_empty() {
        return Err(Error::InvalidDocument);
    }
    values
        .into_iter()
        .map(|(value, _)| decode(&value))
        .collect()
}

pub(super) fn events(
    read: &impl ReadDoc,
    address: &ObjectAddress,
) -> Result<BTreeSet<String>, Error> {
    let root = object(read, &ROOT, "events")?;
    let mut result = BTreeSet::new();
    for id in read.keys(&root) {
        let event: MutationEvent = decode(&unique(read, &root, &id)?)?;
        if &event.address == address {
            result.insert(id);
        }
    }
    Ok(result)
}

pub(super) fn record_event(
    tx: &mut Transaction<'_>,
    address: &ObjectAddress,
    revision: Option<RevisionId>,
) -> Result<(), Error> {
    let root = object(tx, &ROOT, "events")?;
    tx.put(
        root,
        random_id(),
        encode(&MutationEvent {
            address: address.clone(),
            revision,
        })?,
    )?;
    Ok(())
}

pub(super) fn purge(
    read: &impl ReadDoc,
    address: &ObjectAddress,
) -> Result<Option<PurgeRecord>, Error> {
    let root = object(read, &ROOT, "purges")?;
    let values = read.get_all(root, address.key()?)?;
    let mut result: Option<PurgeRecord> = None;
    for (value, _) in values {
        let record: PurgeRecord = decode(&value)?;
        if record.address != *address {
            return Err(Error::InvalidDocument);
        }
        if let Some(existing) = &mut result {
            existing.covered.extend(record.covered);
        } else {
            result = Some(record);
        }
    }
    Ok(result)
}
