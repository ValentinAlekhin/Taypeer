//! Exact lifecycle transitions, placement and subtree cloning.

use super::*;

/// Relative sibling position, resolved inside the document rather than the CLI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SiblingPosition {
    /// Before the first sibling.
    First,
    /// After the last sibling.
    Last,
    /// Immediately before this sibling.
    Before(GroupId),
    /// Immediately after this sibling.
    After(GroupId),
}

impl Document {
    /// Confirm exactly the reviewed selection. Concurrent additions are never silently added.
    pub fn confirm_lifecycle(
        &mut self,
        prepared: &PreparedLifecycle,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<Vec<ObjectId>, Error> {
        if let Some(result) = self.action_receipt(operation, prepared)? {
            return Ok(result);
        }
        if prepared.database != self.database_id {
            return Err(Error::InvalidContext);
        }
        let basis = self.at_heads(&prepared.heads)?;
        let expected = basis.prepare_lifecycle(
            prepared.action,
            prepared.target.clone(),
            prepared.destination.as_ref().map(|p| p.id.clone()),
        )?;
        if &expected != prepared {
            return Err(Error::InvalidContext);
        }
        if let Some(destination) = &prepared.destination
            && self.destination(Some(destination.id.clone()))?.as_ref() != Some(destination)
        {
            return Err(Error::InvalidContext);
        }
        for address in prepared.affected.keys() {
            let (status, conflict) = self.object_status(address)?;
            if status == ObjectStatus::Purged {
                return Err(Error::NotFound);
            }
            if prepared.action == LifecycleAction::Purge
                && (status != ObjectStatus::Trashed || conflict)
            {
                return Err(Error::Conflict);
            }
        }
        let root = objects::single(&basis.doc, &prepared.target)?;
        let order = if prepared.action == LifecycleAction::Restore
            && matches!(root.object, ObjectId::Group(_))
        {
            Some(self.position(
                prepared.destination.as_ref(),
                &SiblingPosition::Last,
                Some(&root.object),
                operation,
            )?)
        } else {
            None
        };
        let mut candidate = self.clone();
        let hashes = parse_heads(&candidate.doc, &prepared.heads)?;
        candidate.prepare_write()?;
        let mut tx = candidate.doc.transaction_at(PatchLog::null(), &hashes);
        for (address, observed) in &prepared.affected {
            let node = objects::generation_object(&tx, address)?;
            match prepared.action {
                LifecycleAction::Trash => {
                    objects::put_life(&mut tx, &node, true, operation.clone(), observed.clone())?
                }
                LifecycleAction::Restore => {
                    objects::put_life(&mut tx, &node, false, operation.clone(), observed.clone())?;
                    if address == &root {
                        match &address.object {
                            ObjectId::Entry(_) => {
                                tx.put(
                                    &node,
                                    "group",
                                    encode(
                                        prepared
                                            .destination
                                            .as_ref()
                                            .ok_or(Error::InvalidContext)?,
                                    )?,
                                )?;
                            }
                            ObjectId::Group(_) => {
                                groups::put_placement(
                                    &mut tx,
                                    &node,
                                    &GroupPlacement {
                                        parent: prepared.destination.clone(),
                                        order: order.clone().ok_or(Error::InvalidContext)?,
                                    },
                                    now,
                                )?;
                            }
                        }
                    }
                    match address.object {
                        ObjectId::Group(_) => objects::record_event(&mut tx, address, None)?,
                        ObjectId::Entry(_) => confirm_revision(
                            &mut tx,
                            address,
                            RevisionKind::Restore,
                            now,
                            &prepared.heads,
                            RevisionId::new(random_id()),
                        )?,
                    }
                }
                LifecycleAction::Purge => {
                    let purges = object(&tx, &ROOT, "purges")?;
                    tx.put(
                        purges,
                        address.key()?,
                        encode(&PurgeRecord {
                            operation: operation.clone(),
                            address: address.clone(),
                            covered: observed.clone(),
                        })?,
                    )?;
                }
            }
        }
        let result: Vec<_> = prepared.affected.keys().map(|a| a.object.clone()).collect();
        put_receipt(&mut tx, operation, prepared, &result)?;
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(result)
    }

    pub(super) fn position(
        &self,
        parent: Option<&GroupRef>,
        position: &SiblingPosition,
        exclude: Option<&ObjectId>,
        operation: &OperationId,
    ) -> Result<OrderKey, Error> {
        let groups = self.groups()?;
        let siblings: Vec<_> = groups
            .iter()
            .filter(|g| {
                g.parent.as_ref() == parent.map(|p| &p.id)
                    && exclude != Some(&ObjectId::Group(g.id.clone()))
            })
            .collect();
        let index = match position {
            SiblingPosition::First => 0,
            SiblingPosition::Last => siblings.len(),
            SiblingPosition::Before(id) => siblings
                .iter()
                .position(|g| &g.id == id)
                .ok_or(Error::InvalidContext)?,
            SiblingPosition::After(id) => {
                siblings
                    .iter()
                    .position(|g| &g.id == id)
                    .ok_or(Error::InvalidContext)?
                    + 1
            }
        };
        let left = index
            .checked_sub(1)
            .map(|i| (&siblings[i].order, siblings[i].id.as_str()));
        let right = siblings.get(index).map(|g| (&g.order, g.id.as_str()));
        if let Some(ObjectId::Group(id)) = exclude
            && let Some(current) = groups
                .iter()
                .find(|g| &g.id == id && g.parent.as_ref() == parent.map(|p| &p.id))
            && left.is_none_or(|(key, id)| {
                key.compare(id, &current.order, current.id.as_str()).is_lt()
            })
            && right
                .is_none_or(|(key, id)| current.order.compare(current.id.as_str(), key, id).is_lt())
        {
            return Ok(current.order.clone());
        }
        OrderKey::between(left, right, operation.as_str()).map_err(|_| Error::InvalidContext)
    }

    /// Move or explicitly resolve a group's placement. A supplied review preserves unseen variants.
    pub fn move_group(
        &mut self,
        request: &GroupMove,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<Vec<ObjectId>, Error> {
        let intent = ("move_group", request);
        if let Some(result) = self.action_receipt(operation, &intent)? {
            return Ok(result);
        }
        let basis = request
            .review
            .as_ref()
            .map(|h| self.at_heads(h))
            .transpose()?
            .unwrap_or_else(|| self.clone());
        let address = objects::single(&basis.doc, &ObjectId::Group(request.group.clone()))?;
        if objects::purge(&self.doc, &address)?.is_some() {
            return Err(Error::NotFound);
        }
        let node = groups::read_group(&basis.doc, &address)?;
        if self.object_status(&address)?.0 == ObjectStatus::Trashed {
            return Err(Error::InvalidContext);
        }
        if objects::single(&self.doc, &address.object)? != address {
            return Err(Error::InvalidContext);
        }
        if request.review.is_none() && (node.placements.len() != 1 || node.names.len() != 1) {
            return Err(Error::Conflict);
        }
        let destination = self.destination(request.parent.clone())?;
        let subtree = basis.selected_subtree(&address)?;
        if destination
            .as_ref()
            .is_some_and(|p| subtree.contains(&ObjectAddress::from(p.clone())))
        {
            return Err(Error::InvalidContext);
        }
        if basis.destination(request.parent.clone())? != destination {
            return Err(Error::InvalidContext);
        }
        if let Some(name) = &request.name {
            validate_group_name(name)?;
        }
        let order = basis.position(
            destination.as_ref(),
            &request.position,
            Some(&address.object),
            operation,
        )?;
        let placement = GroupPlacement {
            parent: destination,
            order,
        };
        let heads = basis.heads();
        let hashes = parse_heads(&self.doc, &heads)?;
        let mut candidate = self.clone();
        candidate.prepare_write()?;
        let mut tx = candidate.doc.transaction_at(PatchLog::null(), &hashes);
        let target = objects::generation_object(&tx, &address)?;
        let changed_placement = node.placements != [placement.clone()] || request.review.is_some();
        let changed_name = request
            .name
            .as_ref()
            .is_some_and(|name| node.names != [name.clone()]);
        if changed_placement {
            groups::put_placement(&mut tx, &target, &placement, now)?;
        }
        if changed_name {
            groups::put_group_name(
                &mut tx,
                &target,
                request.name.as_deref().ok_or(Error::InvalidContext)?,
                now,
            )?;
        }
        if changed_placement || changed_name {
            objects::record_event(&mut tx, &address, None)?;
        }
        let result = vec![address.object];
        put_receipt(&mut tx, operation, &intent, &result)?;
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(result)
    }

    /// Move an entry or resolve a reviewed destination without overwriting unseen changes.
    pub fn move_entry(
        &mut self,
        id: &EntryId,
        group: GroupId,
        review: Option<Vec<String>>,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<Vec<ObjectId>, Error> {
        let intent = ("move_entry", id, &group, &review);
        if let Some(result) = self.action_receipt(operation, &intent)? {
            return Ok(result);
        }
        let basis = review
            .as_ref()
            .map(|h| self.at_heads(h))
            .transpose()?
            .unwrap_or_else(|| self.clone());
        let address = objects::single(&basis.doc, &ObjectId::Entry(id.clone()))?;
        let (status, _) = self.object_status(&address)?;
        if matches!(status, ObjectStatus::Purged | ObjectStatus::Trashed) {
            return Err(Error::InvalidContext);
        }
        if objects::single(&self.doc, &address.object)? != address {
            return Err(Error::InvalidContext);
        }
        let before = projection::read_entry_generation(&basis.doc, &address, None)?;
        if before.placements.len() != 1 && review.is_none() {
            return Err(Error::Conflict);
        }
        let destination = self
            .destination(Some(group.clone()))?
            .ok_or(Error::InvalidContext)?;
        if basis.destination(Some(group.clone()))?.as_ref() != Some(&destination) {
            return Err(Error::InvalidContext);
        }
        let heads = basis.heads();
        let hashes = parse_heads(&self.doc, &heads)?;
        let mut candidate = self.clone();
        candidate.prepare_write()?;
        let mut tx = candidate.doc.transaction_at(PatchLog::null(), &hashes);
        if before.placements != [destination.clone()] || review.is_some() {
            let node = objects::generation_object(&tx, &address)?;
            tx.put(node, "group", encode(&destination)?)?;
            confirm_revision(
                &mut tx,
                &address,
                if review.is_some() {
                    RevisionKind::Resolve
                } else {
                    RevisionKind::Save
                },
                now,
                &heads,
                RevisionId::new(random_id()),
            )?;
        }
        let result = vec![address.object];
        put_receipt(&mut tx, operation, &intent, &result)?;
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(result)
    }

    /// Clone a complete unambiguous active subtree, with new public identities throughout.
    pub fn clone_group(
        &mut self,
        id: &GroupId,
        parent: Option<GroupId>,
        name: Option<String>,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<Vec<ObjectId>, Error> {
        let intent = ("clone_group", id, &parent, &name);
        if let Some(result) = self.action_receipt(operation, &intent)? {
            return Ok(result);
        }
        self.require_group(id)?;
        let source = objects::single(&self.doc, &ObjectId::Group(id.clone()))?;
        let selected = self.selected_subtree(&source)?;
        let destination = self.destination(parent.clone())?;
        if let Some(name) = &name {
            validate_group_name(name)?;
        }
        let root_order = self.position(
            destination.as_ref(),
            &SiblingPosition::Last,
            None,
            operation,
        )?;
        let heads = self.heads();
        let mut mapping = BTreeMap::new();
        for address in &selected {
            let (status, conflict) = self.object_status(address)?;
            if status != ObjectStatus::Active || conflict {
                return Err(Error::Conflict);
            }
            let id = match address.object {
                ObjectId::Group(_) => ObjectId::Group(GroupId::new(random_id())),
                ObjectId::Entry(_) => ObjectId::Entry(EntryId::new(random_id())),
            };
            mapping.insert(
                address.clone(),
                ObjectAddress {
                    generation: GenerationId::new(id.as_str()),
                    object: id,
                },
            );
        }
        let mut candidate = self.clone();
        candidate.prepare_write()?;
        let mut tx = candidate.doc.transaction();
        // Initialize all shells before references or snapshots are read.
        for address in mapping.values() {
            objects::initialize(&mut tx, &address.object)?;
        }
        for address in &selected {
            if !matches!(address.object, ObjectId::Group(_)) {
                continue;
            }
            let original = groups::read_group(&self.doc, address)?;
            let target_address = &mapping[address];
            let target = objects::generation_object(&tx, target_address)?;
            let [placement] = original.placements.as_slice() else {
                return Err(Error::Conflict);
            };
            let parent = if address == &source {
                destination.clone()
            } else {
                placement
                    .parent
                    .clone()
                    .map(|p| {
                        mapping
                            .get(&ObjectAddress::from(p))
                            .ok_or(Error::InvalidContext)?
                            .group_ref()
                    })
                    .transpose()?
            };
            initialize_group(
                &mut tx,
                &target,
                if address == &source {
                    name.as_deref().unwrap_or(&original.names[0])
                } else {
                    &original.names[0]
                },
                GroupPlacement {
                    parent,
                    order: if address == &source {
                        root_order.clone()
                    } else {
                        placement.order.clone()
                    },
                },
                now,
            )?;
            tx.put(&target, "icon", encode(&original.icons[0])?)?;
            let original_object = objects::generation_object(&self.doc, address)?;
            let description =
                crate::metadata::optional_text(&self.doc, &original_object, "description")?;
            tx.put(&target, "description", encode(&description)?)?;
            objects::record_event(&mut tx, target_address, None)?;
        }
        for address in &selected {
            if !matches!(address.object, ObjectId::Entry(_)) {
                continue;
            }
            let original = projection::read_entry_generation(&self.doc, address, None)?;
            let mut fields = original.fields.ok_or(Error::Conflict)?;
            crate::binary::renew_attachment_ids(&mut fields);
            fields.attributes = fields
                .attributes
                .into_values()
                .map(|mut attr| {
                    attr.id = AttributeId::new(random_id());
                    (attr.id.clone(), attr)
                })
                .collect();
            let parent = mapping
                .get(&ObjectAddress::from(original.placements[0].clone()))
                .ok_or(Error::InvalidContext)?
                .group_ref()?;
            let revision = RevisionId::new(random_id());
            initialize_entry(&mut tx, &mapping[address], &fields, &parent, now, &revision)?;
            confirm_revision(
                &mut tx,
                &mapping[address],
                RevisionKind::Clone,
                now,
                &heads,
                revision,
            )?;
        }
        let mut result = vec![mapping[&source].object.clone()];
        result.extend(
            mapping
                .values()
                .filter(|a| a.object != result[0])
                .map(|a| a.object.clone())
                .collect::<Vec<_>>(),
        );
        put_receipt(&mut tx, operation, &intent, &result)?;
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(result)
    }
}

/// An atomic group destination and optional explicit name resolution.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupMove {
    /// Target identity.
    pub group: GroupId,
    /// Explicit parent, or top level.
    pub parent: Option<GroupId>,
    /// Position among destination siblings.
    pub position: SiblingPosition,
    /// Optional reviewed heads for conflict resolution.
    pub review: Option<Vec<String>>,
    /// Optional explicit name choice when resolving a group conflict.
    pub name: Option<String>,
}
impl std::fmt::Debug for GroupMove {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupMove([REDACTED])")
    }
}

pub(super) fn initialize_group(
    tx: &mut Transaction<'_>,
    node: &automerge::ObjId,
    name: &str,
    placement: GroupPlacement,
    now: Timestamp,
) -> Result<(), Error> {
    tx.put(node, "created_at", now)?;
    tx.put(node, "icon", encode(&IconRef::Default)?)?;
    tx.put_object(node, "name_times", ObjType::Map)?;
    tx.put_object(node, "placement_times", ObjType::Map)?;
    groups::put_group_name(tx, node, name, now)?;
    groups::put_placement(tx, node, &placement, now)
}

pub(super) fn initialize_entry(
    tx: &mut Transaction<'_>,
    address: &ObjectAddress,
    fields: &EntryFields,
    parent: &GroupRef,
    now: Timestamp,
    revision: &RevisionId,
) -> Result<(), Error> {
    fields.validate()?;
    let node = objects::generation_object(tx, address)?;
    tx.put(&node, "created_at", now)?;
    tx.put(&node, "group", encode(parent)?)?;
    tx.put_object(&node, "attributes", ObjType::Map)?;
    tx.put_object(&node, "attachments", ObjType::Map)?;
    apply_form(tx, &node, None, fields, revision)?;
    Ok(())
}
