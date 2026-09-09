//! Group placement and rename projection, retaining explicit name conflicts.

use super::{
    Error,
    codec::{decode, object, unique},
};
use automerge::{
    ObjId, ROOT, ReadDoc,
    transaction::{Transactable, Transaction},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use taypeer_core::{Group, GroupId, Timestamp, validate_group_name};

#[derive(Serialize, Deserialize)]
pub(super) struct GroupPlacement {
    pub(super) parent: Option<GroupId>,
    pub(super) order: u64,
}

pub(super) fn read_groups<R: ReadDoc>(read: &R) -> Result<Vec<Group>, Error> {
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

pub(super) fn put_group_name(
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
