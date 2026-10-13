//! Addressed descriptive metadata saves retain their original causal context.
use super::*;
use taypeer_core::{DatabaseMetadataPatch, FieldUpdate, GroupMetadataPatch, OperationId};

impl Document {
    /// Selected parent in an exact form basis, without reading a later placement.
    pub fn group_parent_at(
        &self,
        id: &GroupId,
        heads: &[String],
    ) -> Result<Option<GroupId>, Error> {
        let basis = self.at_heads(heads)?;
        basis.require_group(id)?;
        let address = objects::single(&basis.doc, &ObjectId::Group(id.clone()))?;
        Ok(groups::read_group(&basis.doc, &address)?
            .placement
            .parent
            .map(|parent| parent.id))
    }

    /// Validate an explicit group-form destination against its original and current generations.
    /// A new group has no source yet; an existing group cannot become its own descendant.
    pub fn validate_group_parent(
        &self,
        group: Option<&GroupId>,
        parent: Option<&GroupId>,
        heads: &[String],
    ) -> Result<(), Error> {
        let basis = self.at_heads(heads)?;
        match group {
            Some(group) => {
                self.require_group(group)?;
                let address = objects::single(&basis.doc, &ObjectId::Group(group.clone()))?;
                self.reviewed_group_destination(&basis, &address, parent.cloned())?;
            }
            None => {
                if self.destination(parent.cloned())? != basis.destination(parent.cloned())? {
                    return Err(Error::InvalidContext);
                }
            }
        }
        Ok(())
    }

    /// Active destinations with names from an exact form basis. Recovered lifetimes and the
    /// source's retained subtree are excluded; the top level is represented separately by None.
    pub fn group_parent_candidates(
        &self,
        group: Option<&GroupId>,
        heads: &[String],
    ) -> Result<Vec<Group>, Error> {
        let basis = self.at_heads(heads)?;
        let excluded = match group {
            Some(group) => {
                let address = objects::single(&basis.doc, &ObjectId::Group(group.clone()))?;
                basis.selected_subtree(&address)?
            }
            None => BTreeSet::new(),
        };
        let current: BTreeMap<_, _> = self
            .groups()?
            .into_iter()
            .map(|group| (group.id, group.generation))
            .collect();
        Ok(basis
            .groups()?
            .into_iter()
            .filter(|group| {
                current.get(&group.id) == Some(&group.generation)
                    && !excluded.contains(&ObjectAddress::from(GroupRef {
                        id: group.id.clone(),
                        generation: group.generation.clone(),
                    }))
            })
            .collect())
    }
    /// Selected optional description; original branch values remain in saved history.
    pub fn description(&self) -> Result<Option<String>, Error> {
        optional_text(&self.doc, &ROOT, "description")
    }
    /// Selected current display name.
    pub fn display_name(&self) -> Result<String, Error> {
        Ok(optional_text(&self.doc, &ROOT, "display_name")?.unwrap_or_else(|| self.name.clone()))
    }
    /// Update descriptive fields at the current context using the original creation timestamp.
    /// Timestamped callers should use `update_metadata_at`.
    pub fn update_metadata(
        &mut self,
        name: String,
        description: Option<String>,
    ) -> Result<(), Error> {
        self.update_metadata_command(name, description, None)
    }
    /// Confirm metadata and its retry receipt together at the current context.
    pub fn update_metadata_command(
        &mut self,
        name: String,
        description: Option<String>,
        receipt: Option<&CommandReceipt<'_>>,
    ) -> Result<(), Error> {
        let now = unique(&self.doc, &ROOT, "created_at")?
            .to_i64()
            .ok_or(Error::InvalidDocument)?;
        self.update_metadata_at_command(name, description, now, receipt)
    }
    /// Save a changed database form as one timestamped version.
    pub fn update_metadata_at(
        &mut self,
        name: String,
        description: Option<String>,
        now: Timestamp,
    ) -> Result<(), Error> {
        self.update_metadata_at_command(name, description, now, None)
    }
    /// Save a database form and its receipt atomically.
    pub fn update_metadata_at_command(
        &mut self,
        name: String,
        description: Option<String>,
        now: Timestamp,
        receipt: Option<&CommandReceipt<'_>>,
    ) -> Result<(), Error> {
        let patch = DatabaseMetadataPatch {
            name: if self.display_name()? == name {
                FieldUpdate::Keep
            } else {
                FieldUpdate::Set(name)
            },
            description: optional_patch(self.description()?, description),
        };
        self.patch_metadata_command(&patch, &self.heads(), now, receipt)
    }
    /// Apply only dirty fields at persisted reviewed heads and record one changed version.
    /// Later unseen fields survive; a receipt-only or unchanged command creates no history.
    pub fn patch_metadata_command(
        &mut self,
        patch: &DatabaseMetadataPatch,
        heads: &[String],
        now: Timestamp,
        receipt: Option<&CommandReceipt<'_>>,
    ) -> Result<(), Error> {
        self.confirm_database_metadata(patch, heads, now, receipt, RevisionKind::Save)
    }
    pub(super) fn confirm_database_metadata(
        &mut self,
        patch: &DatabaseMetadataPatch,
        heads: &[String],
        now: Timestamp,
        receipt: Option<&CommandReceipt<'_>>,
        kind: RevisionKind,
    ) -> Result<(), Error> {
        validate_name_update(&patch.name)?;
        let basis = self.at_heads(heads)?;
        let name = confirmed_name(&patch.name, &basis.display_name()?, kind);
        let description = confirmed_optional(&patch.description, basis.description()?, kind);
        if name.is_none() && description.is_none() && receipt.is_none() {
            return Ok(());
        }
        let hashes = lifecycle::parse_heads(&self.doc, heads)?;
        let mut candidate = self.clone();
        candidate.prepare_write()?;
        let mut tx = candidate.doc.transaction_at(PatchLog::null(), &hashes);
        if let Some(name) = &name {
            tx.put(ROOT, "display_name", name.as_str())?;
        }
        if let Some(description) = &description {
            tx.put(ROOT, "description", encode(description)?)?;
        }
        if name.is_some() || description.is_some() {
            history::record_database_revision(&mut tx, &self.database_id, kind, now, heads)?;
        }
        if let Some(receipt) = receipt {
            receipt.write(&mut tx, &())?;
        }
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(())
    }
    /// Selected optional group description on its precise generation.
    pub fn group_description(&self, id: &GroupId) -> Result<Option<String>, Error> {
        self.require_group(id)?;
        let address = objects::single(&self.doc, &ObjectId::Group(id.clone()))?;
        let object = objects::generation_object(&self.doc, &address)?;
        optional_text(&self.doc, &object, "description")
    }
    /// Save descriptive text without changing unrelated group fields.
    pub fn set_group_description(
        &mut self,
        id: &GroupId,
        description: Option<String>,
        now: Timestamp,
    ) -> Result<(), Error> {
        let patch = GroupMetadataPatch {
            description: optional_patch(self.group_description(id)?, description),
            ..Default::default()
        };
        self.update_group_metadata_command(id, &patch, &self.heads(), now, None)
            .map(|_| ())
    }
    /// Save addressed group fields at persisted reviewed heads as one atomic version.
    /// Unknown fields and unseen concurrent operations are preserved.
    pub fn update_group_metadata_command(
        &mut self,
        id: &GroupId,
        patch: &GroupMetadataPatch,
        heads: &[String],
        now: Timestamp,
        receipt: Option<&CommandReceipt<'_>>,
    ) -> Result<Group, Error> {
        self.confirm_group_metadata(id, patch, heads, now, receipt, RevisionKind::Save)
    }
    pub(super) fn confirm_group_metadata(
        &mut self,
        id: &GroupId,
        patch: &GroupMetadataPatch,
        heads: &[String],
        now: Timestamp,
        receipt: Option<&CommandReceipt<'_>>,
        kind: RevisionKind,
    ) -> Result<Group, Error> {
        validate_name_update(&patch.name)?;
        self.require_group(id)?;
        let basis = self.at_heads(heads)?;
        let address = objects::single(&basis.doc, &ObjectId::Group(id.clone()))?;
        if objects::single(&self.doc, &address.object)? != address {
            return Err(Error::InvalidContext);
        }
        let node = groups::read_group(&basis.doc, &address)?;
        let object = objects::generation_object(&basis.doc, &address)?;
        let name = confirmed_name(&patch.name, &node.name, kind);
        let description = confirmed_optional(
            &patch.description,
            optional_text(&basis.doc, &object, "description")?,
            kind,
        );
        let icon = match &patch.icon {
            FieldUpdate::Keep => None,
            FieldUpdate::Set(icon) if icon != &node.icon || kind == RevisionKind::Restore => {
                Some(icon.clone())
            }
            FieldUpdate::Clear
                if node.icon != IconRef::Default || kind == RevisionKind::Restore =>
            {
                Some(IconRef::Default)
            }
            FieldUpdate::Set(_) | FieldUpdate::Clear => None,
        };
        let parent = match &patch.parent {
            FieldUpdate::Keep => None,
            FieldUpdate::Set(parent)
                if node.placement.parent.as_ref().map(|parent| &parent.id) != Some(parent) =>
            {
                Some(Some(parent.clone()))
            }
            FieldUpdate::Clear if node.placement.parent.is_some() => Some(None),
            FieldUpdate::Set(_) | FieldUpdate::Clear => None,
        };
        let placement = parent
            .map(|parent| {
                let destination = self.reviewed_group_destination(&basis, &address, parent)?;
                let operation = receipt
                    .map(|receipt| receipt.operation.clone())
                    .unwrap_or_else(|| OperationId::new(id.as_str()));
                let order = basis.position(
                    destination.as_ref(),
                    &SiblingPosition::Last,
                    Some(&address.object),
                    &operation,
                )?;
                Ok::<_, Error>(GroupPlacement {
                    parent: destination,
                    order,
                })
            })
            .transpose()?;
        if name.is_none()
            && description.is_none()
            && icon.is_none()
            && placement.is_none()
            && receipt.is_none()
        {
            return groups::selected_group(&node);
        }
        let hashes = lifecycle::parse_heads(&self.doc, heads)?;
        let mut candidate = self.clone();
        candidate.prepare_write()?;
        let mut tx = candidate.doc.transaction_at(PatchLog::null(), &hashes);
        let object = objects::generation_object(&tx, &address)?;
        if let Some(name) = &name {
            groups::put_group_name(&mut tx, &object, name, now)?;
        }
        if let Some(description) = &description {
            tx.put(&object, "description", encode(description)?)?;
            tx.put(&object, "description_modified_at", now)?;
        }
        if let Some(icon) = &icon {
            tx.put(&object, "icon", encode(icon)?)?;
            tx.put(&object, "icon_modified_at", now)?;
        }
        if let Some(placement) = &placement {
            groups::put_placement(&mut tx, &object, placement, now)?;
        }
        if name.is_some() || description.is_some() || icon.is_some() || placement.is_some() {
            history::record_group_revision(&mut tx, &address, kind, now, heads)?;
            objects::record_event(&mut tx, &address, None)?;
        }
        let group = groups::selected_group(&groups::read_group(&tx, &address)?)?;
        if let Some(receipt) = receipt {
            receipt.write(&mut tx, &group)?;
        }
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(group)
    }
}
fn validate_name_update(update: &FieldUpdate<String>) -> Result<(), Error> {
    match update {
        FieldUpdate::Keep => Ok(()),
        FieldUpdate::Set(name) => validate_group_name(name).map_err(Into::into),
        FieldUpdate::Clear => Err(ValidationError::EmptyGroupName.into()),
    }
}
fn changed_name(update: &FieldUpdate<String>, original: &str) -> Option<String> {
    match update {
        FieldUpdate::Set(name) if name != original => Some(name.clone()),
        FieldUpdate::Keep | FieldUpdate::Clear | FieldUpdate::Set(_) => None,
    }
}
fn confirmed_name(
    update: &FieldUpdate<String>,
    original: &str,
    kind: RevisionKind,
) -> Option<String> {
    if kind == RevisionKind::Restore {
        match update {
            FieldUpdate::Set(value) => Some(value.clone()),
            FieldUpdate::Keep | FieldUpdate::Clear => None,
        }
    } else {
        changed_name(update, original)
    }
}
fn confirmed_optional(
    update: &FieldUpdate<String>,
    original: Option<String>,
    kind: RevisionKind,
) -> Option<Option<String>> {
    if kind == RevisionKind::Restore {
        match update {
            FieldUpdate::Set(value) => Some(Some(value.clone())),
            FieldUpdate::Clear => Some(None),
            FieldUpdate::Keep => None,
        }
    } else {
        changed_optional(update, original)
    }
}
fn optional_patch(original: Option<String>, supplied: Option<String>) -> FieldUpdate<String> {
    if original == supplied {
        return FieldUpdate::Keep;
    }
    supplied.map_or(FieldUpdate::Clear, FieldUpdate::Set)
}
fn changed_optional(
    update: &FieldUpdate<String>,
    original: Option<String>,
) -> Option<Option<String>> {
    match update {
        FieldUpdate::Keep => None,
        FieldUpdate::Set(value) if original.as_ref() != Some(value) => Some(Some(value.clone())),
        FieldUpdate::Clear if original.is_some() => Some(None),
        FieldUpdate::Set(_) | FieldUpdate::Clear => None,
    }
}
pub(super) fn optional_text(
    read: &impl ReadDoc,
    object: &automerge::ObjId,
    key: &str,
) -> Result<Option<String>, Error> {
    let Some((value, _)) = read
        .get_all(object, key)?
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1))
    else {
        return Ok(None);
    };
    if key == "display_name" {
        Ok(Some(
            value.to_str().ok_or(Error::InvalidDocument)?.to_owned(),
        ))
    } else {
        codec::decode(&value)
    }
}

#[cfg(test)]
mod parent_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use taypeer_core::OperationId;
    #[test]
    fn optional_metadata_roundtrips_and_survives_ordinary_edits() {
        let mut document = Document::new("PUBLIC original", 1000).unwrap();
        let group = document
            .create_group("PUBLIC group".into(), None, 1000)
            .unwrap()
            .id;
        assert_eq!(document.description().unwrap(), None);
        assert_eq!(document.group_description(&group).unwrap(), None);
        let mut tx = document.doc.transaction();
        tx.put(ROOT, "future_optional", "PUBLIC unknown preserved")
            .unwrap();
        tx.commit();
        document
            .update_metadata(
                "PUBLIC renamed".into(),
                Some("  PUBLIC description\nТекст  ".into()),
            )
            .unwrap();
        document
            .set_group_description(&group, Some("PUBLIC group description".into()), 2000)
            .unwrap();
        let mut reloaded = Document::load(&document.export()).unwrap();
        reloaded
            .rename_group(&group, "PUBLIC changed group".into(), 3000)
            .unwrap();
        assert_eq!(reloaded.name(), "PUBLIC original");
        assert_eq!(reloaded.display_name().unwrap(), "PUBLIC renamed");
        assert_eq!(
            reloaded.description().unwrap().as_deref(),
            Some("  PUBLIC description\nТекст  ")
        );
        assert_eq!(
            reloaded.group_description(&group).unwrap().as_deref(),
            Some("PUBLIC group description")
        );
        assert_eq!(
            reloaded
                .doc
                .get(ROOT, "future_optional")
                .unwrap()
                .unwrap()
                .0
                .to_str(),
            Some("PUBLIC unknown preserved")
        );
    }
    #[test]
    fn concurrent_metadata_is_selected_and_original_versions_remain() {
        let original = Document::new("PUBLIC original", 1000).unwrap();
        let mut left = original.fork();
        let mut right = original.fork();
        left.update_metadata_at("PUBLIC left".into(), None, 2000)
            .unwrap();
        right
            .update_metadata_at("PUBLIC right".into(), None, 3000)
            .unwrap();
        left.merge(&right).unwrap();
        right.merge(&left).unwrap();
        assert_eq!(left.display_name().unwrap(), right.display_name().unwrap());
        assert_eq!(left.database_history().unwrap().len(), 3);
        left.update_metadata_at("PUBLIC third".into(), None, 4000)
            .unwrap();
        assert_eq!(left.display_name().unwrap(), "PUBLIC third");
        assert_eq!(left.database_history().unwrap().len(), 4);
    }
    #[test]
    fn independent_metadata_edits_merge_without_rewriting_untouched_fields() {
        let original = Document::new("PUBLIC original", 1000).unwrap();
        let mut left = original.fork();
        let mut right = original.fork();
        left.update_metadata("PUBLIC renamed".into(), None).unwrap();
        right
            .update_metadata("PUBLIC original".into(), Some(String::new()))
            .unwrap();
        left.merge(&right).unwrap();
        assert_eq!(left.display_name().unwrap(), "PUBLIC renamed");
        assert_eq!(left.description().unwrap(), Some(String::new()));
    }
    #[test]
    fn cloned_group_keeps_its_description_and_identity_is_independent() {
        let mut document = Document::new("PUBLIC original", 1000).unwrap();
        let group = document
            .create_group("PUBLIC group".into(), None, 1000)
            .unwrap()
            .id;
        document
            .set_group_description(&group, Some("PUBLIC original description".into()), 2000)
            .unwrap();
        let objects = document
            .clone_group(
                &group,
                None,
                Some("PUBLIC clone".into()),
                &OperationId::new("PUBLIC clone"),
                3000,
            )
            .unwrap();
        let ObjectId::Group(clone) = &objects[0] else {
            panic!("expected group clone")
        };
        assert_ne!(clone, &group);
        document.set_group_description(&group, None, 4000).unwrap();
        let reloaded = Document::load(&document.export()).unwrap();
        assert_eq!(
            reloaded.group_description(clone).unwrap().as_deref(),
            Some("PUBLIC original description")
        );
        assert_eq!(reloaded.group_description(&group).unwrap(), None);
    }
}
