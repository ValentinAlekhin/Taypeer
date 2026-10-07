//! Explicit immutable publication, separate from ongoing widget input and patch acknowledgements.
use super::*;
use taypeer_core::{DraftId, OperationId};
use taypeer_services::{DraftIdentity, DraftSaveOutcome};

pub(super) struct SaveAttempt {
    identity: Option<DraftIdentity>,
    input_revision: u64,
    operation: OperationId,
    ticket: Option<Ticket<crate::backend::CapturedSnapshot>>,
}
impl SaveAttempt {
    pub(super) fn saving(&self) -> bool {
        self.ticket.is_some()
    }
}
pub(crate) enum SaveEvent {
    Confirmed(DraftSaveOutcome),
    Failed(taypeer_runtime::RuntimeError),
}
impl EditorStore {
    pub fn draft_id(&self) -> &DraftId {
        &self.identity.draft
    }
    pub fn durably_current(&self) -> bool {
        self.durable_revision == Some(self.revision)
    }
    pub fn dirty(&self) -> bool {
        self.durable_revision != Some(self.revision) || self.saving()
    }
    pub fn saving(&self) -> bool {
        self.snapshot.as_ref().is_some_and(SaveAttempt::saving)
    }
    /// Queue one snapshot at this click's input position, without waiting for patch replies.
    pub fn start_snapshot(&mut self) -> bool {
        if self.saving() || self.error.is_some() || !self.editable() {
            return false;
        }
        if self.snapshot.is_none() {
            let Ok(operation) = taypeer_services::new_operation_id() else {
                self.save_error = Some(FormError::Backend);
                return false;
            };
            self.snapshot = Some(SaveAttempt {
                identity: None,
                input_revision: self.revision,
                operation,
                ticket: None,
            });
        }
        let attempt = self
            .snapshot
            .as_mut()
            .expect("publication attempt was created");
        attempt.ticket = Some(match &attempt.identity {
            Some(identity) => self
                .connection
                .retry_snapshot(identity.clone(), attempt.operation.clone()),
            None => self
                .connection
                .capture_snapshot(self.identity.draft.clone(), attempt.operation.clone()),
        });
        self.save_error = None;
        true
    }
    pub(super) fn poll_snapshot(&mut self) -> bool {
        let Some(result) = self
            .snapshot
            .as_ref()
            .and_then(|attempt| attempt.ticket.as_ref())
            .and_then(Ticket::try_take)
        else {
            return false;
        };
        let attempt = self
            .snapshot
            .as_mut()
            .expect("ticket belongs to its publication attempt");
        attempt.ticket = None;
        let result = result.and_then(|captured| {
            attempt.identity = Some(captured.identity);
            captured.outcome
        });
        match result {
            Ok(outcome) => {
                let (identity, operation) = match &outcome {
                    DraftSaveOutcome::Saved {
                        identity,
                        operation,
                    }
                    | DraftSaveOutcome::Unchanged {
                        identity,
                        operation,
                    } => (identity, operation),
                };
                if Some(identity) != attempt.identity.as_ref() || operation != &attempt.operation {
                    self.save_error = Some(FormError::Backend);
                    self.save_event =
                        Some(SaveEvent::Failed(taypeer_runtime::RuntimeError::Protocol));
                    return true;
                }
                if attempt.input_revision == self.revision {
                    self.durable_revision = Some(self.revision);
                }
                self.save_event = Some(SaveEvent::Confirmed(outcome));
                self.snapshot = None;
                self.save_error = None;
                self.needs_projection = true;
            }
            Err(error) => {
                self.save_error = Some(FormError::Runtime(error));
                self.save_event = Some(SaveEvent::Failed(error));
                if matches!(
                    error,
                    taypeer_runtime::RuntimeError::Service(
                        taypeer_services::ServiceError::InvalidInput
                    )
                ) || attempt.input_revision != self.revision
                {
                    self.snapshot = None;
                }
            }
        }
        true
    }
    pub fn take_save_event(&mut self) -> Option<SaveEvent> {
        self.save_event.take()
    }
}
