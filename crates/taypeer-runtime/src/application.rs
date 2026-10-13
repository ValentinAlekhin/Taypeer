//! Content-free observation of a live worker's automatic application results.
use crate::{RuntimeError, WorkerControl};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Outcome of the latest automatic application attempt, separate from receipt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationPhase {
    /// No application result has been published for this generation.
    #[default]
    Idle,
    /// The shared service durably applied all currently available changes.
    Applied,
    /// Ciphertext remains retained for dependencies or an explicit decision.
    Pending,
    /// Application failed; this never acknowledges a durable change.
    Failed,
}

/// A coherent, content-free snapshot of one worker generation's latest attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplicationSnapshot {
    /// Incremented for every published result, including identical reports.
    pub revision: u64,
    /// Latest report revision that durably changed data or the control baseline.
    /// Starts at zero in each worker generation and survives subsequent empty or
    /// failed attempts, so a consumer that misses polls still observes changes.
    pub applied_revision: u64,
    /// Application outcome; never a statement about remote receipt or presence.
    pub phase: ApplicationPhase,
    /// Newly accepted original changes in this attempt, after durable publication.
    pub applied: u64,
    /// Ciphertext packets still waiting after this attempt.
    pub pending: u64,
}

/// Observe safe application metadata independently of command I/O. This handle
/// cannot request worker commands and never stores documents, errors or source IDs.
#[derive(Clone)]
pub struct WorkerApplicationObserver {
    control: WorkerControl,
    snapshot: Arc<Mutex<ApplicationSnapshot>>,
}
impl WorkerApplicationObserver {
    pub(crate) fn new(control: WorkerControl) -> Self {
        Self {
            control,
            snapshot: Arc::new(Mutex::new(ApplicationSnapshot::default())),
        }
    }
    /// Poll without waiting for a busy command, Binder, filesystem or another
    /// poll. Revoked generations and a briefly contended snapshot return None;
    /// consumers must also check their captured generation before applying queries.
    pub fn snapshot(&self) -> Option<ApplicationSnapshot> {
        if !self.control.is_open() {
            return None;
        }
        let snapshot = *self.snapshot.try_lock().ok()?;
        self.control.is_open().then_some(snapshot)
    }
    pub(crate) fn publish(&self, revision: u64, result: &Result<Value, RuntimeError>) {
        let (mut snapshot, changed) = match result {
            Ok(value) => match taypeer_services::ApplyReport::deserialize(value) {
                Ok(report) => (
                    ApplicationSnapshot {
                        revision,
                        phase: if report.pending.is_empty() {
                            ApplicationPhase::Applied
                        } else {
                            ApplicationPhase::Pending
                        },
                        applied: report.applied as u64,
                        pending: report.pending.len() as u64,
                        applied_revision: 0,
                    },
                    // Older workers omit `changed`; positive original counts
                    // remain a durable-change acknowledgement in that format.
                    report.changed || report.applied > 0,
                ),
                Err(_) => (
                    ApplicationSnapshot {
                        revision,
                        phase: ApplicationPhase::Failed,
                        ..Default::default()
                    },
                    false,
                ),
            },
            Err(_) => (
                ApplicationSnapshot {
                    revision,
                    phase: ApplicationPhase::Failed,
                    ..Default::default()
                },
                false,
            ),
        };
        if let Ok(mut current) = self.snapshot.lock() {
            snapshot.applied_revision = if changed {
                revision
            } else {
                current.applied_revision
            };
            *current = snapshot;
        }
    }
}
