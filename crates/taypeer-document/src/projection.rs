//! Logical entry snapshots; ambiguous values never become an implicit winner.

use super::{
    Error,
    codec::{object, unique},
    fields::read_field,
    stored_revisions,
};
use automerge::ReadDoc;
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::{
    Attachment, AttachmentId, Attribute, AttributeId, EntryField, EntryFields, EntryId,
    EntrySnapshot, FieldState, FieldValue, GroupRef, Timestamp, ValidationError,
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
    let mut placements: Vec<GroupRef> = Vec::new();
    for (value, _) in read.get_all(&entry, "group")? {
        let destination = super::decode(&value)?;
        if !placements.contains(&destination) {
            placements.push(destination);
        }
    }
    if placements.is_empty() {
        return Err(Error::InvalidDocument);
    }
    let group_id = if let [group] = placements.as_slice() {
        Some(group.id.clone())
    } else {
        None
    };
    let created_at = unique(read, &entry, "created_at")?
        .to_i64()
        .ok_or(Error::InvalidDocument)?;
    let values = read_values(read, &entry)?;
    let mut conflicts: Vec<_> = values
        .iter()
        .filter(|state| state.variants.len() > 1)
        .cloned()
        .collect();
    let modified_at = modification_time(read, id, &entry, &values, pending, created_at)?;
    let fields = unambiguous_fields(&values, &mut conflicts)?;
    Ok(EntrySnapshot {
        id: id.clone(),
        group_id,
        generation: address.generation.clone(),
        placements,
        fields,
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
        let removed = matches!(presence.variants.as_slice(), [variant] if variant.value == FieldValue::Presence(false));
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
        if !matches!(presence.variants.as_slice(), [v] if v.value == FieldValue::Presence(false)) {
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

fn unambiguous_fields(
    values: &[FieldState],
    conflicts: &mut Vec<FieldState>,
) -> Result<Option<EntryFields>, Error> {
    if !conflicts.is_empty() {
        return Ok(None);
    }
    let fields = project_fields(values)?;
    match fields.validate() {
        Ok(()) => Ok(Some(fields)),
        Err(ValidationError::DuplicateAttributeName) => {
            let mut names: BTreeMap<&str, Vec<&Attribute>> = BTreeMap::new();
            for attribute in fields.attributes.values() {
                names.entry(&attribute.name).or_default().push(attribute);
            }
            let duplicate_ids: BTreeSet<_> = names
                .values()
                .filter(|attrs| attrs.len() > 1)
                .flatten()
                .map(|attr| &attr.id)
                .collect();
            conflicts.extend(values.iter().filter(|state| matches!(&state.field, EntryField::AttributeName(id) if duplicate_ids.contains(id))).cloned());
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
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
        let [variant] = state.variants.as_slice() else {
            return Err(Error::Conflict);
        };
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
