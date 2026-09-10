//! Lossless group projection and an acyclic ordinary tree. Invalid edges stay inspectable.

use super::{
    Error,
    codec::{decode, object, unique},
    objects::{self, ObjectAddress, ObjectId},
};
use automerge::{
    ObjId, ReadDoc,
    transaction::{Transactable, Transaction},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::{Group, GroupPlacement, Timestamp, validate_group_name};

/// Product availability of a state generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectStatus {
    /// Current, unambiguously placed state.
    Active,
    /// Retained in the trash.
    Trashed,
    /// Closed permanently; only explicit pending-source workflows may read late data.
    Purged,
    /// Retained for placement/generation conflict review.
    Unplaced,
}

/// A group with every retained name and atomic placement alternative.
#[derive(Clone, Serialize, Deserialize)]
pub struct GroupNode {
    /// Exact lifetime.
    pub address: ObjectAddress,
    /// Distinct names. Multiple names require explicit resolution.
    pub names: Vec<String>,
    /// Complete placement alternatives.
    pub placements: Vec<GroupPlacement>,
    /// Product availability.
    pub status: ObjectStatus,
    /// Whether lifecycle, placement, name or generation needs review.
    pub conflicted: bool,
    /// Whether a parent edge must be excluded from the ordinary tree.
    pub placement_conflict: bool,
    /// Whether this generation is selected by the object's current register.
    pub current: bool,
    /// Original creation time.
    pub created_at: Timestamp,
    /// Time of currently visible content operations.
    pub modified_at: Timestamp,
}
impl std::fmt::Debug for GroupNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupNode([REDACTED])")
    }
}

pub(super) fn own_status(
    read: &impl ReadDoc,
    address: &ObjectAddress,
) -> Result<(ObjectStatus, bool), Error> {
    if objects::purge(read, address)?.is_some() {
        return Ok((ObjectStatus::Purged, false));
    }
    let marks = objects::life(read, address)?;
    let events = objects::events(read, address)?;
    let trashed: Vec<_> = marks.iter().filter(|m| m.trashed).collect();
    if trashed.is_empty() {
        return Ok((ObjectStatus::Active, false));
    }
    let conflict =
        marks.iter().any(|m| !m.trashed) || trashed.iter().any(|m| !events.is_subset(&m.observed));
    Ok((ObjectStatus::Trashed, conflict))
}

pub(super) fn read_group(read: &impl ReadDoc, address: &ObjectAddress) -> Result<GroupNode, Error> {
    let obj = objects::generation_object(read, address)?;
    let mut names = BTreeSet::new();
    let mut placements = Vec::new();
    let mut times = Vec::new();
    for key in ["name", "placement"] {
        let time_map = object(read, &obj, &format!("{key}_times"))?;
        for (value, operation) in read.get_all(&obj, key)? {
            if key == "name" {
                let name = value.to_str().ok_or(Error::InvalidDocument)?;
                validate_group_name(name)?;
                names.insert(name.to_owned());
            } else {
                let placement: GroupPlacement = decode(&value)?;
                if !placements.contains(&placement) {
                    placements.push(placement);
                }
            }
            times.push(
                unique(read, &time_map, &operation.to_string())?
                    .to_i64()
                    .ok_or(Error::InvalidDocument)?,
            );
        }
    }
    if names.is_empty() || placements.is_empty() {
        return Err(Error::InvalidDocument);
    }
    let created_at = unique(read, &obj, "created_at")?
        .to_i64()
        .ok_or(Error::InvalidDocument)?;
    let (status, conflict) = own_status(read, address)?;
    let current = objects::current(read, &address.object)?;
    Ok(GroupNode {
        address: address.clone(),
        names: names.into_iter().collect(),
        placement_conflict: placements.len() != 1 || current.len() != 1,
        placements,
        status,
        conflicted: conflict || current.len() != 1,
        current: current.contains(address),
        created_at,
        modified_at: times.into_iter().max().unwrap_or(created_at),
    })
}

pub(super) fn tree(read: &impl ReadDoc) -> Result<Vec<GroupNode>, Error> {
    let mut nodes = BTreeMap::new();
    for address in objects::all(read)? {
        if matches!(address.object, ObjectId::Group(_)) {
            nodes.insert(address.clone(), read_group(read, &address)?);
        }
    }
    let mut bad = BTreeSet::new();
    // A functional parent graph permits iterative cycle detection without a call-stack limit.
    for start in nodes.keys() {
        let mut path = Vec::new();
        let mut positions = BTreeMap::new();
        let mut cursor = Some(start.clone());
        while let Some(address) = cursor {
            if let Some(&position) = positions.get(&address) {
                bad.extend(path[position..].iter().cloned());
                break;
            }
            let Some(node) = nodes.get(&address) else {
                break;
            };
            if node.placements.len() != 1 {
                break;
            }
            positions.insert(address.clone(), path.len());
            path.push(address.clone());
            cursor = node.placements[0].parent.clone().map(ObjectAddress::from);
            if cursor.as_ref().is_some_and(|p| !nodes.contains_key(p)) {
                bad.insert(address);
                break;
            }
        }
    }
    for address in &bad {
        if let Some(node) = nodes.get_mut(address) {
            node.placement_conflict = true;
        }
    }
    // Propagate unavailable ancestry monotonically; retained children remain addressable.
    loop {
        let previous = nodes.clone();
        let mut changed = false;
        for node in nodes.values_mut() {
            if node.status == ObjectStatus::Purged {
                continue;
            }
            for placement in &node.placements {
                if let Some(parent) = &placement.parent {
                    match previous.get(&ObjectAddress::from(parent.clone())) {
                        Some(parent) if parent.status == ObjectStatus::Trashed => {
                            if node.status != ObjectStatus::Trashed {
                                node.status = ObjectStatus::Trashed;
                                node.conflicted = true;
                                changed = true;
                            }
                        }
                        Some(parent)
                            if (parent.status == ObjectStatus::Purged
                                || !parent.current
                                || parent.placement_conflict
                                || parent.names.len() != 1)
                                && !node.placement_conflict =>
                        {
                            node.placement_conflict = true;
                            changed = true;
                        }
                        None if !node.placement_conflict => {
                            node.placement_conflict = true;
                            changed = true;
                        }
                        _ => {}
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    for node in nodes.values_mut() {
        node.conflicted |= node.placement_conflict || node.names.len() != 1;
        if node.status == ObjectStatus::Active && (node.placement_conflict || !node.current) {
            node.status = ObjectStatus::Unplaced;
        }
    }
    Ok(nodes.into_values().collect())
}

pub(super) fn read_groups(read: &impl ReadDoc) -> Result<Vec<Group>, Error> {
    let nodes = tree(read)?;
    let mut accepted: BTreeSet<_> = nodes
        .iter()
        .filter(|n| n.status == ObjectStatus::Active && n.names.len() == 1)
        .map(|n| n.address.clone())
        .collect();
    loop {
        let previous = accepted.clone();
        for node in &nodes {
            if let [placement] = node.placements.as_slice() {
                if placement
                    .parent
                    .as_ref()
                    .is_some_and(|p| !previous.contains(&ObjectAddress::from(p.clone())))
                {
                    accepted.remove(&node.address);
                }
            } else {
                accepted.remove(&node.address);
            }
        }
        if previous == accepted {
            break;
        }
    }
    let mut result = Vec::new();
    for node in nodes {
        if !accepted.contains(&node.address) {
            continue;
        }
        let ObjectId::Group(id) = node.address.object else {
            return Err(Error::InvalidDocument);
        };
        let [placement] = node.placements.as_slice() else {
            return Err(Error::InvalidDocument);
        };
        result.push(Group {
            id,
            generation: node.address.generation,
            name: node.names[0].clone(),
            parent: placement.parent.as_ref().map(|p| p.id.clone()),
            order: placement.order.clone(),
            created_at: node.created_at,
            modified_at: node.modified_at,
        });
    }
    result.sort_by(|a, b| {
        a.parent
            .cmp(&b.parent)
            .then_with(|| a.order.compare(a.id.as_str(), &b.order, b.id.as_str()))
    });
    Ok(result)
}

pub(super) fn put_group_name(
    tx: &mut Transaction<'_>,
    group: &ObjId,
    name: &str,
    now: Timestamp,
) -> Result<(), Error> {
    tx.put(group, "name", name)?;
    record_times(tx, group, "name", now)
}
pub(super) fn put_placement(
    tx: &mut Transaction<'_>,
    group: &ObjId,
    placement: &GroupPlacement,
    now: Timestamp,
) -> Result<(), Error> {
    tx.put(group, "placement", super::encode(placement)?)?;
    record_times(tx, group, "placement", now)
}
fn record_times(
    tx: &mut Transaction<'_>,
    group: &ObjId,
    key: &str,
    now: Timestamp,
) -> Result<(), Error> {
    let times = object(tx, group, &format!("{key}_times"))?;
    let operations: Vec<_> = tx
        .get_all(group, key)?
        .into_iter()
        .map(|(_, op)| op)
        .collect();
    for op in operations {
        if tx.hash_for_opid(&op).is_none() {
            tx.put(&times, op.to_string(), now)?;
        }
    }
    Ok(())
}
