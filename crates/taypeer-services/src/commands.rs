//! Service-owned candidates and exact, secret-free retry receipts.
use crate::*;
use serde::{Serialize, de::DeserializeOwned};
use taypeer_core::OperationId;

pub(super) fn fingerprint(request: &impl Serialize) -> Result<serde_json::Value, ServiceError> {
    let digest = taypeer_trust::Digest::object(b"taypeer.command.v1", request)
        .map_err(ServiceError::Trust)?;
    serde_json::to_value(digest).map_err(|_| ServiceError::InvalidInput)
}
fn receipt_error(error: taypeer_document::Error) -> ServiceError {
    match error {
        taypeer_document::Error::DuplicateId => ServiceError::OperationConflict,
        other => other.into(),
    }
}
impl DatabaseState {
    pub(super) fn command_result<T: DeserializeOwned>(
        &self,
        operation: &OperationId,
        kind: &str,
        fingerprint: Option<&serde_json::Value>,
    ) -> Result<Option<T>, ServiceError> {
        let Some((old, result)) = self
            .document()
            .command_receipt(operation, kind)
            .map_err(receipt_error)?
        else {
            return Ok(None);
        };
        if fingerprint.is_some_and(|new| new != &old) {
            return Err(ServiceError::OperationConflict);
        }
        serde_json::from_value(result)
            .map(Some)
            .map_err(|_| ServiceError::InvalidDocument)
    }
    pub(super) fn command<T: Serialize + DeserializeOwned>(
        &mut self,
        operation: &OperationId,
        kind: &str,
        fingerprint: serde_json::Value,
        change: impl FnOnce(
            &mut Document,
            &taypeer_document::CommandReceipt<'_>,
        ) -> Result<T, ServiceError>,
    ) -> Result<T, ServiceError> {
        if let Some(result) = self.command_result(operation, kind, Some(&fingerprint))? {
            return Ok(result);
        }
        let mut candidate = self.document().clone();
        let receipt = taypeer_document::CommandReceipt {
            operation,
            kind,
            fingerprint: &fingerprint,
        };
        let result = change(&mut candidate, &receipt)?;
        self.commit(candidate)?;
        Ok(result)
    }
}
impl DatabaseService {
    /// Create an entry in one service-owned transaction, retaining no implicit editor.
    pub fn create_entry(
        &mut self,
        session: &SessionToken,
        group: GroupId,
        patch: EntryPatch,
        operation: &OperationId,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let fingerprint = fingerprint(&(&group, &patch))?;
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if let Some(id) = state.command_result(operation, "create_entry", Some(&fingerprint))? {
            return Ok(stamped(session, id));
        }
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let id = state.command(operation, "create_entry", fingerprint, |doc, receipt| {
            let mut draft = DraftState::new(doc.begin_create_entry(group)?, DraftKind::New);
            let mut fields = draft.view().fields;
            patch.apply(&mut fields)?;
            draft.update(fields)?;
            draft.save_command(doc, now, receipt)
        })?;
        Ok(stamped(session, id))
    }
    /// Apply an addressed patch exactly once without publishing an intermediate editor.
    pub fn update_entry(
        &mut self,
        session: &SessionToken,
        id: &EntryId,
        patch: EntryPatch,
        operation: &OperationId,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let fingerprint = fingerprint(&(id, &patch))?;
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if let Some(id) = state.command_result(operation, "update_entry", Some(&fingerprint))? {
            return Ok(stamped(session, id));
        }
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let id = state.command(operation, "update_entry", fingerprint, |doc, receipt| {
            let mut draft = DraftState::new(doc.begin_edit_entry(id)?, DraftKind::Existing);
            let mut fields = draft.view().fields;
            patch.apply(&mut fields)?;
            draft.update(fields)?;
            draft.save_command(doc, now, receipt)
        })?;
        Ok(stamped(session, id))
    }
}
