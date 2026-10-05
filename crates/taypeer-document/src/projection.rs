//! Deterministic editable entry projection with lossless original alternatives.

use super::{
    Error,
    codec::{object, unique},
    fields::{read_field, selected_variant},
    stored_revisions,
};
use automerge::ReadDoc;
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::{
    Attachment, AttachmentId, Attribute, AttributeId, EntryField, EntryFields, EntryId,
    EntrySnapshot, FieldState, FieldValue, GroupRef, Timestamp,
};

pub(super) fn read_entry<R: ReadDoc>(
    read: &R,
    id: &EntryId,
    pending: Option<(&BTreeSet<String>, Timestamp)>,
) -> Result<EntrySnapshot, Error> {
    let address = super::objects::single(read, &super::ObjectId::Entry(id.clone()))?;
    read_entry_generation(read, &address, pending)
}

pub(super) fn read_entry_generation<R: ReadDoc>(
    read: &R,
    address: &super::ObjectAddress,
    pending: Option<(&BTreeSet<String>, Timestamp)>,
) -> Result<EntrySnapshot, Error> {
    let super::ObjectId::Entry(id) = &address.object else {
        return Err(Error::InvalidContext);
    };
    let entry = super::objects::generation_object(read, address)?;
    let mut placement_alternatives: Vec<Option<GroupRef>> = Vec::new();
    let mut selected_placement = None;
    for (value, operation) in read.get_all(&entry, "group")? {
        let destination: Option<GroupRef> = super::decode(&value)?;
        if !placement_alternatives.contains(&destination) {
            placement_alternatives.push(destination.clone());
        }
        if selected_placement
            .as_ref()
            .is_none_or(|(_, rank)| &operation > rank)
        {
            selected_placement = Some((destination, operation));
        }
    }
    let placement = selected_placement.ok_or(Error::InvalidDocument)?.0;
    let group_id = placement.as_ref().map(|group| group.id.clone());
    let placements = placement_alternatives.iter().flatten().cloned().collect();
    placement_alternatives.sort();
    let created_at = unique(read, &entry, "created_at")?
        .to_i64()
        .ok_or(Error::InvalidDocument)?;
    let values = read_values(read, &entry)?;
    let conflicts: Vec<_> = values
        .iter()
        .filter(|state| state.variants.len() > 1)
        .cloned()
        .collect();
    let modified_at = modification_time(read, id, &entry, &values, pending, created_at)?;
    let fields = project_fields(&values)?;
    fields.validate()?;
    Ok(EntrySnapshot {
        id: id.clone(),
        group_id,
        generation: address.generation.clone(),
        placements,
        placement,
        placement_alternatives,
        fields: Some(fields),
        conflicts,
        values,
        created_at,
        modified_at,
    })
}

fn read_values<R: ReadDoc>(read: &R, entry: &automerge::ObjId) -> Result<Vec<FieldState>, Error> {
    let mut values = Vec::new();
    for field in [
        EntryField::Title,
        EntryField::Username,
        EntryField::Password,
        EntryField::Url,
        EntryField::Notes,
        EntryField::Tags,
        EntryField::ExpiresAt,
        EntryField::Icon,
        EntryField::Foreground,
        EntryField::Background,
    ] {
        values.push(read_field(read, entry, field)?);
    }
    let attributes = object(read, entry, "attributes")?;
    for key in read.keys(attributes) {
        let attr = AttributeId::new(key);
        let presence = read_field(read, entry, EntryField::AttributePresence(attr.clone()))?;
        let removed = selected_variant(&presence)?.value == FieldValue::Presence(false);
        if !removed {
            for field in [
                EntryField::AttributeName(attr.clone()),
                EntryField::AttributeValue(attr),
            ] {
                values.push(read_field(read, entry, field)?);
            }
        }
        values.push(presence);
    }
    let attachments = object(read, entry, "attachments")?;
    for key in read.keys(attachments) {
        let id = AttachmentId::new(key);
        let presence = read_field(read, entry, EntryField::AttachmentPresence(id.clone()))?;
        if selected_variant(&presence)?.value != FieldValue::Presence(false) {
            values.push(read_field(
                read,
                entry,
                EntryField::AttachmentName(id.clone()),
            )?);
            values.push(read_field(read, entry, EntryField::AttachmentBlob(id))?);
        }
        values.push(presence);
    }
    Ok(values)
}

fn modification_time<R: ReadDoc>(
    read: &R,
    id: &EntryId,
    entry: &automerge::ObjId,
    values: &[FieldState],
    pending: Option<(&BTreeSet<String>, Timestamp)>,
    created_at: Timestamp,
) -> Result<Timestamp, Error> {
    let mut origins: BTreeSet<_> = values
        .iter()
        .flat_map(|state| state.variants.iter())
        .flat_map(|variant| variant.origins.iter().cloned())
        .collect();
    origins.extend(
        read.get_all(entry, "group")?
            .into_iter()
            .map(|(_, op)| op.to_string()),
    );
    let mut operation_times = BTreeMap::new();
    for stored in stored_revisions(read, id)? {
        for operation in stored.changed_operations {
            operation_times.insert(operation, stored.revision.saved_at);
        }
    }
    if let Some((pending, now)) = pending {
        for operation in pending {
            operation_times.insert(operation.clone(), now);
        }
    }
    let modified_at = origins
        .iter()
        .filter_map(|operation| operation_times.get(operation))
        .copied()
        .max()
        .unwrap_or(created_at);
    Ok(modified_at)
}

fn project_fields(values: &[FieldState]) -> Result<EntryFields, Error> {
    let mut result = EntryFields::default();
    let mut names = BTreeMap::new();
    let mut attr_values = BTreeMap::new();
    let mut present = BTreeSet::new();
    let mut attachment_names = BTreeMap::new();
    let mut attachment_blobs = BTreeMap::new();
    let mut attachments = BTreeSet::new();
    for state in values {
        let variant = selected_variant(state)?;
        match (&state.field, &variant.value) {
            (EntryField::Title, FieldValue::Text(Some(value))) => result.title = value.clone(),
            (EntryField::Username, FieldValue::Text(value)) => result.username = value.clone(),
            (EntryField::Password, FieldValue::Text(value)) => result.password = value.clone(),
            (EntryField::Url, FieldValue::Text(value)) => result.url = value.clone(),
            (EntryField::Notes, FieldValue::Text(value)) => result.notes = value.clone(),
            (EntryField::Tags, FieldValue::Tags(value)) => result.tags = value.clone(),
            (EntryField::ExpiresAt, FieldValue::Timestamp(value)) => result.expires_at = *value,
            (EntryField::AttributeName(id), FieldValue::Text(Some(value))) => {
                names.insert(id.clone(), value.clone());
            }
            (EntryField::AttributeValue(id), FieldValue::Attribute(value)) => {
                attr_values.insert(id.clone(), value.clone());
            }
            (EntryField::AttributePresence(id), FieldValue::Presence(true)) => {
                present.insert(id.clone());
            }
            (EntryField::AttributePresence(_), FieldValue::Presence(false)) => {}
            (EntryField::Icon, FieldValue::Icon(value)) => result.appearance.icon = value.clone(),
            (EntryField::Foreground, FieldValue::Color(value)) => {
                result.appearance.foreground = *value
            }
            (EntryField::Background, FieldValue::Color(value)) => {
                result.appearance.background = *value
            }
            (EntryField::AttachmentName(id), FieldValue::Text(Some(value))) => {
                attachment_names.insert(id.clone(), value.clone());
            }
            (EntryField::AttachmentBlob(id), FieldValue::Blob(value)) => {
                attachment_blobs.insert(id.clone(), value.clone());
            }
            (EntryField::AttachmentPresence(id), FieldValue::Presence(true)) => {
                attachments.insert(id.clone());
            }
            (EntryField::AttachmentPresence(_), FieldValue::Presence(false)) => {}
            _ => return Err(Error::InvalidDocument),
        }
    }
    for id in present {
        let name = names.remove(&id).ok_or(Error::InvalidDocument)?;
        let value = attr_values.remove(&id).ok_or(Error::InvalidDocument)?;
        result
            .attributes
            .insert(id.clone(), Attribute { id, name, value });
    }
    for id in attachments {
        let name = attachment_names.remove(&id).ok_or(Error::InvalidDocument)?;
        let blob = attachment_blobs.remove(&id).ok_or(Error::InvalidDocument)?;
        result
            .attachments
            .insert(id.clone(), Attachment { id, name, blob });
    }
    Ok(result)
}

/// Saved states retain selected values only; earlier branch revisions retain alternatives.
pub(super) fn selected_snapshot(mut snapshot: EntrySnapshot) -> Result<EntrySnapshot, Error> {
    for state in &mut snapshot.values {
        state.variants = vec![selected_variant(state)?.clone()];
    }
    snapshot.conflicts.clear();
    snapshot.placement_alternatives = vec![snapshot.placement.clone()];
    snapshot.placements = snapshot.placement.iter().cloned().collect();
    Ok(snapshot)
}
