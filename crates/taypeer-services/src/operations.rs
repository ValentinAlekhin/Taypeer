//! Conflict review and idempotent history commands with durable confirmation.

use super::*;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use taypeer_core::{EntryField, FieldValue, OperationId};

/// Allocate an opaque idempotency key using the operating system random source.
pub fn new_operation_id() -> Result<OperationId, ServiceError> {
    let mut bytes = [0; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| ServiceError::Storage(StorageError::Random))?;
    Ok(OperationId::new(
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    ))
}

/// A field variant with protected contents removed until explicit reveal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictVariantView {
    /// Original operations, used to select an exact current variant.
    pub origins: Vec<String>,
    /// Whole value only when the field is unprotected.
    pub value: Option<FieldValue>,
}

/// Conflicting field and its masked alternatives.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictFieldView {
    /// Field address.
    pub field: EntryField,
    /// Whether explicit reveal is needed for any of its alternatives.
    pub protected: bool,
    /// All alternatives; none is selected implicitly.
    pub variants: Vec<ConflictVariantView>,
}

/// Review context and masked conflicts. New incoming alternatives invalidate no draft.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictView {
    /// Causal context to submit with an explicit resolution.
    pub context: ConflictContext,
    /// Conflicting values, excluding protected contents.
    pub fields: Vec<ConflictFieldView>,
}

impl DatabaseService {
    /// Clone a current entry, publishing new identities only after durable storage.
    pub fn clone_entry(
        &mut self,
        session: &SessionToken,
        entry: &EntryId,
        group: GroupId,
        title: Option<String>,
        operation: &OperationId,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let now = (self.clock)();
        let result = self
            .checked_mut(session)?
            .change(|doc| Ok(doc.clone_entry(entry, group, title, operation, now)?))?;
        Ok(stamped(session, result))
    }

    /// Restore a saved revision as a new current version; active drafts must be handled first.
    pub fn restore_revision(
        &mut self,
        session: &SessionToken,
        entry: &EntryId,
        revision: &RevisionId,
        group: GroupId,
        operation: &OperationId,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let result = state
            .change(|doc| Ok(doc.restore_revision(entry, revision, group, operation, now)?))?;
        Ok(stamped(session, result))
    }

    /// Remove exactly selected history rows, keeping current conflict alternatives intact.
    pub fn purge_history(
        &mut self,
        session: &SessionToken,
        entry: &EntryId,
        revisions: BTreeSet<RevisionId>,
        operation: &OperationId,
    ) -> Result<SessionValue<()>, ServiceError> {
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        state.change(|doc| Ok(doc.purge_history(entry, revisions, operation)?))?;
        Ok(stamped(session, ()))
    }

    /// Inspect conflicts while concealing passwords and atomic protected attributes.
    pub fn conflicts(
        &self,
        session: &SessionToken,
        entry: &EntryId,
    ) -> Result<SessionValue<ConflictView>, ServiceError> {
        let document = self.checked(session)?.document();
        let fields = document.entry(entry)?.conflicts.into_iter().map(|state| {
            let protected = state.field == EntryField::Password || state.variants.iter().any(|variant| matches!(&variant.value, FieldValue::Attribute(value) if value.protected));
            ConflictFieldView {
                field: state.field, protected,
                variants: state.variants.into_iter().map(|variant| ConflictVariantView {
                    origins: variant.origins, value: (!protected).then_some(variant.value),
                }).collect(),
            }
        }).collect();
        Ok(stamped(
            session,
            ConflictView {
                context: document.conflict_context(entry)?,
                fields,
            },
        ))
    }

    /// Explicitly reveal one currently retained conflicting variant by exact origins.
    pub fn reveal_conflict_variant(
        &self,
        session: &SessionToken,
        entry: &EntryId,
        field: &EntryField,
        origins: &[String],
    ) -> Result<SessionValue<FieldValue>, ServiceError> {
        let snapshot = self.checked(session)?.document().entry(entry)?;
        let value = snapshot
            .conflicts
            .into_iter()
            .find(|state| &state.field == field)
            .and_then(|state| {
                state
                    .variants
                    .into_iter()
                    .find(|variant| variant.origins == origins)
            })
            .ok_or(ServiceError::NotFound)?
            .value;
        Ok(stamped(session, value))
    }

    /// Confirm chosen values at the reviewed causal context, preserving unseen concurrent edits.
    pub fn resolve_conflicts(
        &mut self,
        session: &SessionToken,
        context: &ConflictContext,
        fields: Vec<Resolution>,
        operation: &OperationId,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let id = state.change(|doc| Ok(doc.resolve_fields(context, fields, operation, now)?))?;
        Ok(stamped(session, id))
    }
}
