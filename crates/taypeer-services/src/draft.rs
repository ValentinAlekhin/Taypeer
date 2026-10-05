//! Owns the local editor, its presentation order and interruption lifecycle.

use super::{DraftView, EditableAttribute, EditableEntry, PendingDraftSummary, ServiceError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::{Attribute, AttributeId, AttributeValue, EntryFields, EntryId};
use taypeer_document::{Document, EntryDraft};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) enum DraftKind {
    New,
    Existing,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DraftStatus {
    Active,
    Interrupted,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct DraftState {
    id: taypeer_core::DraftId,
    revision: super::DraftRevision,
    #[serde(default)]
    pub(super) attempt: Option<(taypeer_core::OperationId, serde_json::Value)>,
    document: EntryDraft,
    baseline: EntryFields,
    kind: DraftKind,
    status: DraftStatus,
    attribute_order: Vec<AttributeId>,
    expiry_input: Option<String>,
    binary_receipts: BTreeMap<taypeer_core::OperationId, serde_json::Value>,
}

impl DraftState {
    pub(super) fn new(document: EntryDraft, kind: DraftKind) -> Result<Self, ServiceError> {
        Ok(Self {
            id: super::editing::new_draft_id()?,
            revision: super::DraftRevision::default(),
            attempt: None,
            baseline: document.fields().clone(),
            attribute_order: document.fields().attributes.keys().cloned().collect(),
            document,
            kind,
            status: DraftStatus::Active,
            expiry_input: None,
            binary_receipts: BTreeMap::new(),
        })
    }

    pub(super) fn identity(&self) -> super::DraftIdentity {
        super::DraftIdentity {
            draft: self.id.clone(),
            target: match self.kind {
                DraftKind::New => super::DraftTarget::NewEntry {
                    entry: self.document.entry_id().clone(),
                    group: self.document.group_id().cloned(),
                },
                DraftKind::Existing => super::DraftTarget::Entry(self.document.entry_id().clone()),
            },
            revision: self.revision,
        }
    }

    pub(super) fn resume(&mut self) {
        self.status = DraftStatus::Active;
    }

    fn changed(&mut self) -> Result<(), ServiceError> {
        self.revision.0 = self
            .revision
            .0
            .checked_add(1)
            .ok_or(ServiceError::InvalidContext)?;
        Ok(())
    }

    pub(super) fn continue_saved(&mut self, document: &Document) -> Result<(), ServiceError> {
        let confirmed = self.clone();
        self.continue_from(&confirmed, document)
    }

    pub(super) fn continue_from(
        &mut self,
        confirmed: &Self,
        document: &Document,
    ) -> Result<(), ServiceError> {
        self.document =
            document.continue_entry_draft(&confirmed.document, self.document.fields().clone())?;
        self.baseline = confirmed.document.fields().clone();
        self.kind = DraftKind::Existing;
        self.attempt = None;
        self.binary_receipts.clear();
        Ok(())
    }

    pub(super) fn can_save(&self) -> bool {
        self.expiry_input.is_none() && self.document.fields().validate().is_ok()
    }

    pub(super) fn has_binary_operation(&self, operation: &taypeer_core::OperationId) -> bool {
        self.binary_receipts.contains_key(operation)
    }
    pub(super) fn fingerprint(&self) -> Result<serde_json::Value, ServiceError> {
        super::commands::fingerprint(&(&self.document, &self.expiry_input))
    }
    pub(super) fn confirmed(&self, doc: &Document) -> Result<bool, ServiceError> {
        let Some((operation, fingerprint)) = &self.attempt else {
            return Ok(false);
        };
        let receipt = match doc.command_receipt(operation, "save_draft") {
            Ok(receipt) => receipt,
            // A failed attempt did not reserve the document's operation namespace.
            // A different committed command must not hide this unsaved sidecar.
            Err(taypeer_document::Error::DuplicateId) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        Ok(receipt.is_some_and(|(saved, _)| saved == *fingerprint)
            && self.fingerprint()? == *fingerprint)
    }
    pub(super) fn document(&self) -> Result<&EntryDraft, ServiceError> {
        self.require_active()?;
        Ok(&self.document)
    }
    pub(super) fn document_mut(&mut self) -> Result<&mut EntryDraft, ServiceError> {
        self.require_active()?;
        Ok(&mut self.document)
    }
    pub(super) fn binary_receipt(
        &self,
        operation: &taypeer_core::OperationId,
        intent: &serde_json::Value,
    ) -> Result<bool, ServiceError> {
        self.require_active()?;
        match self.binary_receipts.get(operation) {
            Some(old) if old == intent => Ok(true),
            Some(_) => Err(ServiceError::InvalidInput),
            None => Ok(false),
        }
    }
    pub(super) fn record_binary(
        &mut self,
        operation: taypeer_core::OperationId,
        intent: serde_json::Value,
    ) -> Result<(), ServiceError> {
        self.binary_receipts.insert(operation, intent);
        self.changed()
    }
    pub(super) fn adds_binary_content(&self) -> bool {
        self.document
            .fields()
            .attachments
            .iter()
            .any(|(id, attachment)| {
                self.baseline
                    .attachments
                    .get(id)
                    .is_none_or(|old| old.blob != attachment.blob)
            })
    }
    pub(super) fn binary_references(&self) -> BTreeSet<taypeer_core::BlobId> {
        let mut refs = BTreeSet::new();
        for fields in [&self.baseline, self.document.fields()] {
            refs.extend(fields.attachments.values().map(|a| a.blob.clone()));
            refs.extend(fields.appearance.icon.blob().cloned());
        }
        refs
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
            group_id: self.document.group_id().cloned(),
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
            attachments: candidate.fields().attachments.clone(),
            appearance: candidate.fields().appearance.clone(),
        };
        if candidate.fields() != self.document.fields() {
            self.changed()?;
        }
        self.document = candidate;
        self.attribute_order = attribute_order;
        Ok(self.view())
    }

    pub(super) fn set_expiry_input(
        &mut self,
        input: Option<String>,
    ) -> Result<DraftView, ServiceError> {
        self.require_active()?;
        if self.expiry_input != input {
            self.changed()?;
            self.expiry_input = input;
        }
        Ok(self.view())
    }

    pub(super) fn save_command(
        &self,
        document: &mut Document,
        now: i64,
        receipt: &taypeer_document::CommandReceipt<'_>,
    ) -> Result<EntryId, ServiceError> {
        self.require_active()?;
        if self.expiry_input.is_some() {
            return Err(ServiceError::InvalidInput);
        }
        Ok(document.save_entry_command(self.document.clone(), now, receipt)?)
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
            identity: self.identity(),
            entry_id: self.entry_id().cloned(),
            group_id: self.document.group_id().cloned(),
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
