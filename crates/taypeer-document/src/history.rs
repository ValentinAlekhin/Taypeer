//! Saved descriptive object states and exact, idempotent history cleanup.

use super::*;
use automerge::transaction::Transaction;
use taypeer_core::{
    DatabaseSnapshot, GroupSnapshot, OperationId, SavedDatabaseRevision, SavedGroupRevision,
};

pub(super) fn record_group_revision(
    tx: &mut Transaction<'_>,
    address: &ObjectAddress,
    kind: RevisionKind,
    now: Timestamp,
    base: &[String],
) -> Result<(), Error> {
    let ObjectId::Group(id) = &address.object else {
        return Err(Error::InvalidContext);
    };
    let node = groups::read_group(tx, address)?;
    let node_object = objects::generation_object(tx, address)?;
    let revision = SavedGroupRevision {
        id: RevisionId::new(random_id()),
        group_id: id.clone(),
        saved_at: now,
        kind,
        base: base.to_vec(),
        snapshot: GroupSnapshot {
            group: groups::selected_group(&node)?,
            description: metadata::optional_text(tx, &node_object, "description")?,
        },
    };
    let root = object(tx, &ROOT, "group_revisions")?;
    tx.put(root, revision.id.as_str(), encode(&revision)?)?;
    Ok(())
}

pub(super) fn record_database_revision(
    tx: &mut Transaction<'_>,
    database: &DatabaseId,
    kind: RevisionKind,
    now: Timestamp,
    base: &[String],
) -> Result<(), Error> {
    let name = metadata::optional_text(tx, &ROOT, "display_name")?.unwrap_or(
        unique(tx, &ROOT, "name")?
            .to_str()
            .ok_or(Error::InvalidDocument)?
            .to_owned(),
    );
    let revision = SavedDatabaseRevision {
        id: RevisionId::new(random_id()),
        saved_at: now,
        kind,
        base: base.to_vec(),
        snapshot: DatabaseSnapshot {
            database_id: database.clone(),
            name,
            description: metadata::optional_text(tx, &ROOT, "description")?,
        },
    };
    let root = object(tx, &ROOT, "database_revisions")?;
    tx.put(root, revision.id.as_str(), encode(&revision)?)?;
    Ok(())
}

fn group_revisions(read: &impl ReadDoc) -> Result<Vec<SavedGroupRevision>, Error> {
    let root = object(read, &ROOT, "group_revisions")?;
    let mut result = Vec::new();
    for key in read.keys(&root) {
        let revision: SavedGroupRevision = decode(&unique(read, &root, &key)?)?;
        if revision.id.as_str() != key || revision.group_id != revision.snapshot.group.id {
            return Err(Error::InvalidDocument);
        }
        validate_group_name(&revision.snapshot.group.name)?;
        result.push(revision);
    }
    Ok(result)
}

fn database_revisions(read: &impl ReadDoc) -> Result<Vec<SavedDatabaseRevision>, Error> {
    let root = object(read, &ROOT, "database_revisions")?;
    let database = unique(read, &ROOT, "database_id")?
        .to_str()
        .ok_or(Error::InvalidDocument)?
        .to_owned();
    let mut result = Vec::new();
    for key in read.keys(&root) {
        let revision: SavedDatabaseRevision = decode(&unique(read, &root, &key)?)?;
        if revision.id.as_str() != key || revision.snapshot.database_id.as_str() != database {
            return Err(Error::InvalidDocument);
        }
        validate_group_name(&revision.snapshot.name)?;
        result.push(revision);
    }
    Ok(result)
}

pub(super) fn validate(read: &impl ReadDoc) -> Result<(), Error> {
    group_revisions(read)?;
    database_revisions(read)?;
    Ok(())
}

impl Document {
    /// Read immutable group confirmations, including retained trash generations.
    pub fn group_history(&self, id: &GroupId) -> Result<Vec<SavedGroupRevision>, Error> {
        objects::shell(&self.doc, &ObjectId::Group(id.clone()))?;
        let mut result = Vec::new();
        for revision in group_revisions(&self.doc)? {
            let address = ObjectAddress {
                object: ObjectId::Group(revision.group_id.clone()),
                generation: revision.snapshot.group.generation.clone(),
            };
            if &revision.group_id == id
                && !self.revision_is_purged(&revision.id)?
                && objects::purge(&self.doc, &address)?.is_none()
            {
                result.push(revision);
            }
        }
        result.sort_by(|left, right| (left.saved_at, &left.id).cmp(&(right.saved_at, &right.id)));
        Ok(result)
    }

    /// Read immutable database name and description confirmations.
    pub fn database_history(&self) -> Result<Vec<SavedDatabaseRevision>, Error> {
        let mut result = database_revisions(&self.doc)?
            .into_iter()
            .filter_map(|revision| match self.revision_is_purged(&revision.id) {
                Ok(false) => Some(Ok(revision)),
                Ok(true) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        result.sort_by(|left, right| (left.saved_at, &left.id).cmp(&(right.saved_at, &right.id)));
        Ok(result)
    }

    /// Hide exactly the visible selected group versions. Unseen versions remain accessible.
    pub fn purge_group_history(
        &mut self,
        group: &GroupId,
        revisions: BTreeSet<RevisionId>,
        operation: &OperationId,
    ) -> Result<(), Error> {
        let intent = ("purge_group_history", group, &revisions);
        if self.action_receipt(operation, &intent)?.is_some() {
            return Ok(());
        }
        let existing = self
            .group_history(group)?
            .into_iter()
            .map(|revision| revision.id)
            .collect();
        self.purge_object_revisions(revisions.clone(), existing, operation, &intent)
    }

    /// Hide exactly the visible selected database versions.
    pub fn purge_database_history(
        &mut self,
        revisions: BTreeSet<RevisionId>,
        operation: &OperationId,
    ) -> Result<(), Error> {
        let intent = ("purge_database_history", &revisions);
        if self.action_receipt(operation, &intent)?.is_some() {
            return Ok(());
        }
        let existing = self
            .database_history()?
            .into_iter()
            .map(|revision| revision.id)
            .collect();
        self.purge_object_revisions(revisions.clone(), existing, operation, &intent)
    }

    fn purge_object_revisions(
        &mut self,
        revisions: BTreeSet<RevisionId>,
        existing: BTreeSet<RevisionId>,
        operation: &OperationId,
        intent: &impl Serialize,
    ) -> Result<(), Error> {
        if revisions.is_empty() || !revisions.is_subset(&existing) {
            return Err(Error::NotFound);
        }
        self.prepare_write()?;
        let mut tx = self.doc.transaction();
        let root = object(&tx, &ROOT, "purged_revisions")?;
        for revision in revisions {
            tx.put(&root, revision.as_str(), operation.as_str())?;
        }
        lifecycle::put_receipt(&mut tx, operation, intent, &[])?;
        tx.commit();
        Ok(())
    }
}
