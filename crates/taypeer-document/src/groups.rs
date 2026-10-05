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
    /// Original names retained for historical inspection.
    pub names: Vec<String>,
    /// All stored icon alternatives, including acquisition provenance.
    pub icons: Vec<taypeer_core::IconRef>,
    /// Complete placement alternatives.
    pub placements: Vec<GroupPlacement>,
    /// Selected exact name; alternatives do not obstruct ordinary use.
    pub name: String,
    /// Selected icon.
    pub icon: taypeer_core::IconRef,
    /// Selected acyclic placement, projected from the original parent register.
    pub placement: GroupPlacement,
    /// Product availability.
    pub status: ObjectStatus,
    /// Whether original content alternatives remain available for historical inspection.
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
    if !marks.iter().any(|mark| mark.trashed) {
        return Ok((ObjectStatus::Active, false));
    }
    // A causally later restore supersedes the trash mark. Concurrent content never does.
    Ok((ObjectStatus::Trashed, false))
}

pub(super) fn read_group(read: &impl ReadDoc, address: &ObjectAddress) -> Result<GroupNode, Error> {
    let obj = objects::generation_object(read, address)?;
    let mut names = BTreeSet::new();
    let mut placements = Vec::new();
    let mut selected_name = None;
    let mut selected_placement = None;
    let mut times = Vec::new();
    for key in ["name", "placement"] {
        let time_map = object(read, &obj, &format!("{key}_times"))?;
        for (value, operation) in read.get_all(&obj, key)? {
            if key == "name" {
                let name = value.to_str().ok_or(Error::InvalidDocument)?;
                validate_group_name(name)?;
                names.insert(name.to_owned());
                if selected_name
                    .as_ref()
                    .is_none_or(|(_, rank)| &operation > rank)
                {
                    selected_name = Some((name.to_owned(), operation.clone()));
                }
            } else {
                let placement: GroupPlacement = decode(&value)?;
                if !placements.contains(&placement) {
                    placements.push(placement.clone());
                }
                if selected_placement
                    .as_ref()
                    .is_none_or(|(_, rank)| &operation > rank)
                {
                    selected_placement = Some((placement, operation.clone()));
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
    for key in ["icon_modified_at", "description_modified_at"] {
        for (value, _) in read.get_all(&obj, key)? {
            times.push(value.to_i64().ok_or(Error::InvalidDocument)?);
        }
    }
    let created_at = unique(read, &obj, "created_at")?
        .to_i64()
        .ok_or(Error::InvalidDocument)?;
    let mut icons = Vec::new();
    let mut selected_icon = None;
    for (value, operation) in read.get_all(&obj, "icon")? {
        let icon: taypeer_core::IconRef = decode(&value)?;
        if !icons.contains(&icon) {
            icons.push(icon.clone());
        }
        if selected_icon
            .as_ref()
            .is_none_or(|(_, rank)| &operation > rank)
        {
            selected_icon = Some((icon, operation));
        }
    }
    if icons.is_empty() {
        return Err(Error::InvalidDocument);
    }
    let (status, conflict) = own_status(read, address)?;
    let current = objects::single(read, &address.object)?;
    let conflicted = conflict || names.len() > 1 || placements.len() > 1 || icons.len() > 1;
    Ok(GroupNode {
        address: address.clone(),
        names: names.into_iter().collect(),
        name: selected_name.ok_or(Error::InvalidDocument)?.0,
        placement: selected_placement.ok_or(Error::InvalidDocument)?.0,
        icon: selected_icon.ok_or(Error::InvalidDocument)?.0,
        placement_conflict: false,
        placements,
        status,
        conflicted,
        icons,
        current: &current == address,
        created_at,
        modified_at: times.into_iter().max().unwrap_or(created_at),
    })
}

pub(super) fn tree(read: &impl ReadDoc) -> Result<Vec<GroupNode>, Error> {
    let mut nodes = BTreeMap::new();
    let mut ranks = BTreeMap::new();
    for address in objects::all(read)? {
        if matches!(address.object, ObjectId::Group(_)) {
            let object = objects::generation_object(read, &address)?;
            let rank = read
                .get_all(object, "placement")?
                .into_iter()
                .map(|(_, operation)| operation)
                .max()
                .ok_or(Error::InvalidDocument)?;
            ranks.insert(address.clone(), rank);
            nodes.insert(address.clone(), read_group(read, &address)?);
        }
    }
    // Break one selected edge per cycle; rank rather than traversal order chooses the root.
    let mut roots = BTreeSet::new();
    for start in nodes.keys() {
        let mut path = Vec::new();
        let mut positions = BTreeMap::new();
        let mut cursor = Some(start.clone());
        while let Some(address) = cursor {
            if let Some(&position) = positions.get(&address) {
                let root = path[position..]
                    .iter()
                    .min_by(|left, right| {
                        ranks[*left]
                            .cmp(&ranks[*right])
                            .then_with(|| left.cmp(right))
                    })
                    .ok_or(Error::InvalidDocument)?;
                roots.insert(root.clone());
                break;
            }
            let Some(node) = nodes.get(&address) else {
                break;
            };
            if !node.current {
                break;
            }
            positions.insert(address.clone(), path.len());
            path.push(address);
            cursor = node.placement.parent.clone().map(ObjectAddress::from);
        }
    }
    for address in roots {
        if let Some(node) = nodes.get_mut(&address) {
            node.placement.parent = None;
        }
    }
    // Truly absent parents are orphans. A closed generation is retained but never adopted.
    let known: BTreeSet<_> = nodes.keys().cloned().collect();
    for node in nodes.values_mut() {
        if node
            .placement
            .parent
            .as_ref()
            .is_some_and(|parent| !known.contains(&ObjectAddress::from(parent.clone())))
        {
            node.placement.parent = None;
        }
        if !node.current && node.status == ObjectStatus::Active {
            node.status = ObjectStatus::Unplaced;
        }
    }
    loop {
        let previous: BTreeMap<_, _> = nodes
            .iter()
            .map(|(address, node)| (address.clone(), (node.status, node.current)))
            .collect();
        let mut changed = false;
        for node in nodes.values_mut() {
            if node.status == ObjectStatus::Purged {
                continue;
            }
            if node
                .placements
                .iter()
                .filter_map(|placement| placement.parent.as_ref())
                .any(|parent| {
                    previous
                        .get(&ObjectAddress::from(parent.clone()))
                        .is_some_and(|(status, _)| *status == ObjectStatus::Trashed)
                })
                && node.status != ObjectStatus::Trashed
            {
                node.status = ObjectStatus::Trashed;
                changed = true;
            }
            if let Some(parent) = &node.placement.parent {
                match previous.get(&ObjectAddress::from(parent.clone())) {
                    Some((ObjectStatus::Trashed, _)) => {
                        if node.status != ObjectStatus::Trashed {
                            node.status = ObjectStatus::Trashed;
                            changed = true;
                        }
                    }
                    Some((status, current))
                        if node.status == ObjectStatus::Active
                            && (*status == ObjectStatus::Purged
                                || !current
                                || *status == ObjectStatus::Unplaced) =>
                    {
                        node.status = ObjectStatus::Unplaced;
                        node.placement_conflict = true;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        if !changed {
            break;
        }
    }
    Ok(nodes.into_values().collect())
}

pub(super) fn selected_group(node: &GroupNode) -> Result<Group, Error> {
    let ObjectId::Group(id) = &node.address.object else {
        return Err(Error::InvalidDocument);
    };
    Ok(Group {
        id: id.clone(),
        generation: node.address.generation.clone(),
        name: node.name.clone(),
        icon: node.icon.clone(),
        parent: node
            .placement
            .parent
            .as_ref()
            .map(|parent| parent.id.clone()),
        order: node.placement.order.clone(),
        created_at: node.created_at,
        modified_at: node.modified_at,
    })
}

pub(super) fn read_groups(read: &impl ReadDoc) -> Result<Vec<Group>, Error> {
    let mut result = tree(read)?
        .iter()
        .filter(|node| node.status == ObjectStatus::Active && node.current)
        .map(selected_group)
        .collect::<Result<Vec<_>, _>>()?;
    result.sort_by(|left, right| {
        left.parent.cmp(&right.parent).then_with(|| {
            left.order
                .compare(left.id.as_str(), &right.order, right.id.as_str())
        })
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
