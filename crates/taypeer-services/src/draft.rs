//! Owns the local editor, its presentation order and interruption lifecycle.

use super::{DraftView, EditableAttribute, EditableEntry, PendingDraftSummary, ServiceError};
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::{Attribute, AttributeId, AttributeValue, EntryFields, EntryId};
use taypeer_document::{Document, EntryDraft};

#[derive(Clone, Copy)]
pub(super) enum DraftKind {
    New,
    Existing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DraftStatus {
    Active,
    Interrupted,
}

pub(super) struct DraftState {
    document: EntryDraft,
    baseline: EntryFields,
    kind: DraftKind,
    status: DraftStatus,
    attribute_order: Vec<AttributeId>,
    expiry_input: Option<String>,
}

impl DraftState {
    pub(super) fn new(document: EntryDraft, kind: DraftKind) -> Self {
        Self {
            baseline: document.fields().clone(),
            attribute_order: document.fields().attributes.keys().cloned().collect(),
            document,
            kind,
            status: DraftStatus::Active,
            expiry_input: None,
        }
    }

    pub(super) fn entry_id(&self) -> Option<&EntryId> {
        match self.kind {
            DraftKind::New => None,
            DraftKind::Existing => Some(self.document.entry_id()),
        }
    }

    pub(super) fn needs_restore(&self) -> bool {
        self.status == DraftStatus::Interrupted
    }

    fn require_active(&self) -> Result<(), ServiceError> {
        if self.needs_restore() {
            return Err(ServiceError::DraftNeedsRestore);
        }
        Ok(())
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.document.fields() != &self.baseline || self.expiry_input.is_some()
    }

    pub(super) fn interrupt(&mut self) {
        self.status = DraftStatus::Interrupted;
    }

    pub(super) fn restore(&mut self) -> Result<DraftView, ServiceError> {
        if !self.needs_restore() {
            return Err(ServiceError::InvalidContext);
        }
        self.status = DraftStatus::Active;
        Ok(self.view())
    }

    pub(super) fn pending_summary(&self) -> PendingDraftSummary {
        PendingDraftSummary {
            entry_id: self.entry_id().cloned(),
            group_id: self.document.group_id().clone(),
        }
    }

    pub(super) fn update(&mut self, fields: EditableEntry) -> Result<DraftView, ServiceError> {
        self.require_active()?;
        // Work on a clone so invalid identities or duplicate attributes leave the editor intact.
        let mut candidate = self.document.clone();
        let known: BTreeSet<_> = candidate.fields().attributes.keys().cloned().collect();
        let mut attributes = BTreeMap::new();
        let mut attribute_order = Vec::new();
        for attribute in fields.attributes {
            let id = match attribute.id {
                Some(id) if known.contains(&id) => id,
                Some(_) => return Err(ServiceError::InvalidContext),
                None => candidate.add_attribute(
                    attribute.name.clone(),
                    attribute.value.clone(),
                    attribute.protected,
                ),
            };
            attribute_order.push(id.clone());
            if attributes
                .insert(
                    id.clone(),
                    Attribute {
                        id,
                        name: attribute.name,
                        value: AttributeValue {
                            value: attribute.value,
                            protected: attribute.protected,
                        },
                    },
                )
                .is_some()
            {
                return Err(ServiceError::InvalidInput);
            }
        }
        *candidate.fields_mut() = EntryFields {
            title: fields.title,
            username: fields.username,
            password: fields.password,
            url: fields.url,
            notes: fields.notes,
            tags: fields.tags.into_iter().collect(),
            expires_at: fields.expires_at,
            attributes,
        };
        self.document = candidate;
        self.attribute_order = attribute_order;
        Ok(self.view())
    }

    pub(super) fn set_expiry_input(
        &mut self,
        input: Option<String>,
    ) -> Result<DraftView, ServiceError> {
        self.require_active()?;
        self.expiry_input = input;
        Ok(self.view())
    }

    pub(super) fn save(&self, document: &mut Document, now: i64) -> Result<EntryId, ServiceError> {
        self.require_active()?;
        if self.expiry_input.is_some() {
            return Err(ServiceError::InvalidInput);
        }
        if self.entry_id().is_some() && !self.is_dirty() {
            return Ok(self.document.entry_id().clone());
        }
        // The editor retains the retry context when document validation fails.
        Ok(document.save_entry(self.document.clone(), now)?)
    }

    pub(super) fn view(&self) -> DraftView {
        let mut fields = editable(self.document.fields());
        let order: BTreeMap<_, _> = self
            .attribute_order
            .iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        fields.attributes.sort_by_key(|attribute| {
            attribute
                .id
                .as_ref()
                .and_then(|id| order.get(id))
                .copied()
                .unwrap_or(usize::MAX)
        });
        DraftView {
            entry_id: self.entry_id().cloned(),
            group_id: self.document.group_id().clone(),
            dirty: self.is_dirty(),
            fields,
            expiry_input: self.expiry_input.clone(),
        }
    }
}

fn editable(fields: &EntryFields) -> EditableEntry {
    EditableEntry {
        title: fields.title.clone(),
        username: fields.username.clone(),
        password: fields.password.clone(),
        url: fields.url.clone(),
        notes: fields.notes.clone(),
        tags: fields.tags.iter().cloned().collect(),
        expires_at: fields.expires_at,
        attributes: fields
            .attributes
            .values()
            .map(|attribute| EditableAttribute {
                id: Some(attribute.id.clone()),
                name: attribute.name.clone(),
                value: attribute.value.value.clone(),
                protected: attribute.value.protected,
            })
            .collect(),
    }
}
