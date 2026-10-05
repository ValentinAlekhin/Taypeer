//! One immutable save attempt, separate from ongoing widget input and patch acknowledgements.
use super::*;
use taypeer_core::OperationId;
use taypeer_services::{DraftIdentity, DraftSaveOutcome};

pub(super) struct SaveAttempt {
    identity: DraftIdentity,
    input_revision: u64,
    operation: OperationId,
    ticket: Option<Ticket<DraftSaveOutcome>>,
    retry_after: Instant,
}

pub(crate) enum SaveEvent {
    Confirmed(DraftSaveOutcome),
    Failed(taypeer_runtime::RuntimeError),
}

impl EditorStore {
    pub fn input_revision(&self) -> u64 {
        self.revision
    }
    pub fn durably_current(&self) -> bool {
        self.durable_revision == Some(self.revision) && !self.busy()
    }
    pub fn saving(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|attempt| attempt.ticket.is_some())
    }
    pub fn autosave_pending(&self) -> bool {
        self.durable_revision != Some(self.revision) || self.saving()
    }
    pub fn start_snapshot(&mut self, immediate: bool) -> bool {
        if self.busy()
            || self.saving()
            || self.error.is_some()
            || !self.connection.control.is_open()
            || self.durable_revision == Some(self.revision)
        {
            return false;
        }
        let now = Instant::now();
        if self
            .snapshot
            .as_ref()
            .is_some_and(|attempt| attempt.retry_after > now)
        {
            return false;
        }
        if !immediate
            && now.duration_since(self.last_input)
                < Duration::from_millis(taypeer_services::AUTOSAVE_DELAY_MILLIS)
        {
            return false;
        }
        if self.snapshot.is_none() {
            let Ok(operation) = taypeer_services::new_operation_id() else {
                self.save_error = Some(FormError::Backend);
                return false;
            };
            self.snapshot = Some(SaveAttempt {
                identity: self.identity.clone(),
                input_revision: self.revision,
                operation,
                ticket: None,
                retry_after: now,
            });
        }
        let Some(attempt) = self.snapshot.as_mut() else {
            return false;
        };
        attempt.ticket = Some(self.connection.command(Command::SaveDraftSnapshot {
            draft: attempt.identity.draft.clone(),
            revision: attempt.identity.revision,
            operation: attempt.operation.clone(),
        }));
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
        let Some(attempt) = self.snapshot.as_mut() else {
            return false;
        };
        attempt.ticket = None;
        match result {
            Ok(outcome) => {
                let identity = match &outcome {
                    DraftSaveOutcome::Saved { identity, .. }
                    | DraftSaveOutcome::LocalDraftSaved { identity, .. }
                    | DraftSaveOutcome::Unchanged { identity, .. } => identity,
                };
                let operation = match &outcome {
                    DraftSaveOutcome::Saved { operation, .. }
                    | DraftSaveOutcome::LocalDraftSaved { operation, .. }
                    | DraftSaveOutcome::Unchanged { operation, .. } => operation,
                };
                if identity != &attempt.identity || operation != &attempt.operation {
                    self.save_error = Some(FormError::Backend);
                    attempt.retry_after = Instant::now() + Duration::from_secs(1);
                    self.save_event =
                        Some(SaveEvent::Failed(taypeer_runtime::RuntimeError::Protocol));
                    return true;
                }
                if attempt.input_revision == self.revision {
                    self.durable_revision = Some(self.revision);
                    self.dirty = matches!(outcome, DraftSaveOutcome::LocalDraftSaved { .. });
                }
                self.save_event = Some(SaveEvent::Confirmed(outcome));
                self.snapshot = None;
                self.save_error = None;
                self.needs_projection = true;
            }
            Err(error) => {
                attempt.retry_after = Instant::now() + Duration::from_secs(1);
                self.save_error = Some(FormError::Runtime(error));
                self.save_event = Some(SaveEvent::Failed(error));
            }
        }
        true
    }
    pub fn take_save_event(&mut self) -> Option<SaveEvent> {
        self.save_event.take()
    }
    /// A local fallback acknowledges only the input that preceded that queued publication.
    pub fn confirm_local_fallback(&mut self, revision: u64) {
        if self.revision == revision {
            self.durable_revision = Some(revision);
        }
    }
}
