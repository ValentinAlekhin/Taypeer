//! Atomic field encoding, addressed form edits and conflict alternatives.

use super::{
    Error,
    codec::{decode, encode, object},
};
use automerge::{
    ObjId, ObjType, ReadDoc, ScalarValue, Value,
    transaction::{Transactable, Transaction},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::{
    EntryField, EntryFields, FieldState, FieldValue, RevisionId, ValidationError, ValueVariant,
};

#[derive(Serialize, Deserialize)]
struct Presence {
    alive: bool,
    operation: String,
}

fn field_key(field: &EntryField) -> &str {
    match field {
        EntryField::Title => "title",
        EntryField::Username => "username",
        EntryField::Password => "password",
        EntryField::Url => "url",
        EntryField::Notes => "notes",
        EntryField::Tags => "tags",
        EntryField::ExpiresAt => "expires_at",
        EntryField::AttributeName(_) => "name",
        EntryField::AttributeValue(_) => "value",
        EntryField::AttributePresence(_) | EntryField::AttachmentPresence(_) => "presence",
        EntryField::AttachmentName(_) => "name",
        EntryField::AttachmentBlob(_) => "blob",
        EntryField::Icon => "icon",
        EntryField::Foreground => "foreground",
        EntryField::Background => "background",
    }
}

fn field_object<R: ReadDoc>(read: &R, entry: &ObjId, field: &EntryField) -> Result<ObjId, Error> {
    match field {
        EntryField::AttributeName(id)
        | EntryField::AttributeValue(id)
        | EntryField::AttributePresence(id) => {
            let attrs = object(read, entry, "attributes")?;
            object(read, &attrs, id.as_str())
        }
        EntryField::AttachmentName(id)
        | EntryField::AttachmentBlob(id)
        | EntryField::AttachmentPresence(id) => {
            let attachments = object(read, entry, "attachments")?;
            object(read, &attachments, id.as_str())
        }
        _ => Ok(entry.clone()),
    }
}

pub(super) fn pending_operations(
    tx: &Transaction<'_>,
    entry: &ObjId,
    states: &[FieldState],
) -> Result<BTreeSet<String>, Error> {
    let mut changed = BTreeSet::new();
    for state in states {
        let target = field_object(tx, entry, &state.field)?;
        for (_, operation) in tx.get_all(target, field_key(&state.field))? {
            if tx.hash_for_opid(&operation).is_none() {
                changed.insert(operation.to_string());
            }
        }
    }
    Ok(changed)
}

pub(super) fn validate_field(field: &EntryField, value: &FieldValue) -> Result<(), Error> {
    let valid = match (field, value) {
        (EntryField::Title, FieldValue::Text(Some(value))) => {
            if value.is_empty() {
                return Err(ValidationError::EmptyTitle.into());
            }
            true
        }
        (
            EntryField::Username | EntryField::Password | EntryField::Url | EntryField::Notes,
            FieldValue::Text(_),
        ) => true,
        (EntryField::Tags, FieldValue::Tags(_))
        | (EntryField::ExpiresAt, FieldValue::Timestamp(_)) => true,
        (EntryField::AttributeName(_), FieldValue::Text(Some(value))) => {
            if value.is_empty() {
                return Err(ValidationError::EmptyAttributeName.into());
            }
            true
        }
        (EntryField::AttachmentName(_), FieldValue::Text(Some(name))) => !name.is_empty(),
        (EntryField::AttachmentBlob(_), FieldValue::Blob(_))
        | (EntryField::AttachmentPresence(_), FieldValue::Presence(_))
        | (EntryField::Icon, FieldValue::Icon(_))
        | (EntryField::Foreground | EntryField::Background, FieldValue::Color(_)) => true,
        (EntryField::AttributeValue(_), FieldValue::Attribute(_))
        | (EntryField::AttributePresence(_), FieldValue::Presence(_)) => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidDocument)
    }
}

pub(super) fn put_field(
    tx: &mut Transaction<'_>,
    entry: &ObjId,
    field: &EntryField,
    value: FieldValue,
    revision: &RevisionId,
    changed: &mut BTreeSet<String>,
) -> Result<(), Error> {
    validate_field(field, &value)?;
    let scalar = match value {
        FieldValue::Text(Some(value)) => ScalarValue::from(value),
        FieldValue::Text(None) | FieldValue::Timestamp(None) => ScalarValue::Null,
        FieldValue::Timestamp(Some(value)) => ScalarValue::Timestamp(value),
        FieldValue::Tags(value) => encode(&value)?,
        FieldValue::Attribute(value) => encode(&value)?,
        FieldValue::Blob(value) => encode(&value)?,
        FieldValue::Icon(value) => encode(&value)?,
        FieldValue::Color(value) => encode(&value)?,
        FieldValue::Presence(alive) => encode(&Presence {
            alive,
            operation: revision.as_str().into(),
        })?,
    };
    let obj = field_object(tx, entry, field)?;
    tx.put(&obj, field_key(field), scalar)?;
    for (_, operation) in tx.get_all(&obj, field_key(field))? {
        if tx.hash_for_opid(&operation).is_none() {
            changed.insert(operation.to_string());
        }
    }
    Ok(())
}

pub(super) fn form_fields(fields: &EntryFields) -> Vec<(EntryField, FieldValue)> {
    vec![
        (
            EntryField::Icon,
            FieldValue::Icon(fields.appearance.icon.clone()),
        ),
        (
            EntryField::Foreground,
            FieldValue::Color(fields.appearance.foreground),
        ),
        (
            EntryField::Background,
            FieldValue::Color(fields.appearance.background),
        ),
        (
            EntryField::Title,
            FieldValue::Text(Some(fields.title.clone())),
        ),
        (
            EntryField::Username,
            FieldValue::Text(fields.username.clone()),
        ),
        (
            EntryField::Password,
            FieldValue::Text(fields.password.clone()),
        ),
        (EntryField::Url, FieldValue::Text(fields.url.clone())),
        (EntryField::Notes, FieldValue::Text(fields.notes.clone())),
        (EntryField::Tags, FieldValue::Tags(fields.tags.clone())),
        (
            EntryField::ExpiresAt,
            FieldValue::Timestamp(fields.expires_at),
        ),
    ]
}

pub(super) fn apply_form(
    tx: &mut Transaction<'_>,
    entry: &ObjId,
    original: Option<&EntryFields>,
    fields: &EntryFields,
    revision: &RevisionId,
) -> Result<BTreeSet<String>, Error> {
    let mut changed = BTreeSet::new();
    let old: BTreeMap<_, _> = original
        .map(form_fields)
        .unwrap_or_default()
        .into_iter()
        .collect();
    for (field, value) in form_fields(fields) {
        if old.get(&field) != Some(&value) {
            put_field(tx, entry, &field, value, revision, &mut changed)?;
        }
    }
    let attrs = object(tx, entry, "attributes")?;
    for (id, attr) in &fields.attributes {
        let old = original.and_then(|fields| fields.attributes.get(id));
        if old == Some(attr) {
            continue;
        }
        if old.is_none() {
            if !tx.get_all(&attrs, id.as_str())?.is_empty() {
                return Err(Error::DuplicateId);
            }
            tx.put_object(&attrs, id.as_str(), ObjType::Map)?;
        }
        if old.is_none_or(|old| old.name != attr.name) {
            put_field(
                tx,
                entry,
                &EntryField::AttributeName(id.clone()),
                FieldValue::Text(Some(attr.name.clone())),
                revision,
                &mut changed,
            )?;
        }
        if old.is_none_or(|old| old.value != attr.value) {
            put_field(
                tx,
                entry,
                &EntryField::AttributeValue(id.clone()),
                FieldValue::Attribute(attr.value.clone()),
                revision,
                &mut changed,
            )?;
        }
        // Every meaningful attribute edit witnesses presence. A unique operation marker avoids
        // same-value write elision, while presentation coalesces concurrent `true` values.
        put_field(
            tx,
            entry,
            &EntryField::AttributePresence(id.clone()),
            FieldValue::Presence(true),
            revision,
            &mut changed,
        )?;
    }
    if let Some(original) = original {
        for id in original
            .attributes
            .keys()
            .filter(|id| !fields.attributes.contains_key(*id))
        {
            put_field(
                tx,
                entry,
                &EntryField::AttributePresence(id.clone()),
                FieldValue::Presence(false),
                revision,
                &mut changed,
            )?;
        }
    }
    super::binary::apply_attachments(tx, entry, original, fields, revision, &mut changed)?;
    Ok(changed)
}

fn decode_field(field: &EntryField, value: &Value<'_>) -> Result<FieldValue, Error> {
    let decoded = match field {
        EntryField::Title
        | EntryField::Username
        | EntryField::Password
        | EntryField::Url
        | EntryField::Notes
        | EntryField::AttributeName(_)
        | EntryField::AttachmentName(_) => {
            if value == &Value::Scalar(std::borrow::Cow::Owned(ScalarValue::Null)) {
                FieldValue::Text(None)
            } else {
                FieldValue::Text(Some(value.to_str().ok_or(Error::InvalidDocument)?.into()))
            }
        }
        EntryField::Tags => FieldValue::Tags(decode(value)?),
        EntryField::ExpiresAt => {
            let Value::Scalar(scalar) = value else {
                return Err(Error::InvalidDocument);
            };
            FieldValue::Timestamp(match scalar.as_ref() {
                ScalarValue::Null => None,
                ScalarValue::Timestamp(time) => Some(*time),
                _ => return Err(Error::InvalidDocument),
            })
        }
        EntryField::AttributeValue(_) => FieldValue::Attribute(decode(value)?),
        EntryField::AttributePresence(_) | EntryField::AttachmentPresence(_) => {
            FieldValue::Presence(decode::<Presence>(value)?.alive)
        }
        EntryField::AttachmentBlob(_) => FieldValue::Blob(decode(value)?),
        EntryField::Icon => FieldValue::Icon(decode(value)?),
        EntryField::Foreground | EntryField::Background => FieldValue::Color(decode(value)?),
    };
    validate_field(field, &decoded)?;
    Ok(decoded)
}

pub(super) fn read_field<R: ReadDoc>(
    read: &R,
    entry: &ObjId,
    field: EntryField,
) -> Result<FieldState, Error> {
    let obj = field_object(read, entry, &field)?;
    let mut variants: Vec<ValueVariant> = Vec::new();
    for (value, operation) in read.get_all(obj, field_key(&field))? {
        let value = decode_field(&field, &value)?;
        if let Some(existing) = variants.iter_mut().find(|variant| variant.value == value) {
            existing.origins.push(operation.to_string());
        } else {
            variants.push(ValueVariant {
                value,
                origins: vec![operation.to_string()],
            });
        }
    }
    if variants.is_empty() {
        return Err(Error::InvalidDocument);
    }
    for variant in &mut variants {
        variant.origins.sort();
    }
    variants.sort_by(|a, b| a.origins.cmp(&b.origins));
    Ok(FieldState { field, variants })
}
