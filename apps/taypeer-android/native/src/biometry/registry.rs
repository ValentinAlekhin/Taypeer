//! Private durable wrapper registry. It never proves OS or worker authentication.
//! The fixed Native broker owns one serialized registry per registered copy.
mod persistence;
mod types;

use persistence::{FilePublisher, Publisher, load};
use std::path::{Path, PathBuf};
use taypeer_trust::Digest;
pub(super) use types::{Binding, Generation, WrapperRecord};
use types::{PendingSetup, Record, SetupAction, SetupReceipt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{self:?}")]
pub(super) enum RegistryError {
    Invalid,
    Io,
    CommitUncertain,
    Stale,
    NoPending,
    Conflict,
}

/// A consumed proof token minted only by the private broker after actual worker
/// proof ACK. It cannot be deserialized, cloned, or constructed from a UI flag.
pub(super) struct ConfirmedEnrollment {
    operation: Digest,
    candidate: WrapperRecord,
}
impl ConfirmedEnrollment {
    /// Consume the actual worker ACK, then check its permanent candidate binding.
    /// No UI flag, deserialized token or OS result can mint this authority.
    pub(super) fn consume_worker_proof(
        proof: taypeer_runtime::platform_worker::biometry::BiometricEnrollmentProof,
        expected: &taypeer_runtime::platform_worker::biometry::BiometricInputLease,
        candidate: WrapperRecord,
    ) -> Result<Self, RegistryError> {
        let actual = proof.into_lease();
        let binding = candidate.binding();
        if &actual != expected
            || actual.scope.namespace != binding.namespace()
            || actual.scope.managed.working_copy != binding.working_copy()
            || &actual.scope.managed.database != binding.database()
            || actual.scope.managed.root != binding.root()
            || actual.scope.managed.epoch != binding.epoch()
        {
            return Err(RegistryError::Stale);
        }
        Ok(Self {
            operation: actual.scope.attempt,
            candidate,
        })
    }
    #[cfg(test)]
    /// PUBLIC fixture construction is restricted to registry unit tests.
    pub(super) fn after_worker_proof(operation: Digest, candidate: WrapperRecord) -> Self {
        Self {
            operation,
            candidate,
        }
    }
}
impl std::fmt::Debug for ConfirmedEnrollment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfirmedEnrollment([redacted])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Durable registry metadata only: no variant proves OS-key availability, worker
/// authentication, current epoch eligibility, or an OPEN session.
pub(super) enum SetupState {
    PendingEnable {
        operation: Digest,
        candidate: WrapperRecord,
        active: Option<WrapperRecord>,
    },
    PendingDisable {
        operation: Digest,
        active: WrapperRecord,
    },
    Enabled(WrapperRecord),
    Disabled,
}
impl SetupState {
    fn from_record(record: &Record) -> Result<Self, RegistryError> {
        Ok(match &record.pending {
            Some(PendingSetup::Enable {
                operation,
                candidate,
                ..
            }) => Self::PendingEnable {
                operation: *operation,
                candidate: candidate.clone(),
                active: record.active.clone(),
            },
            Some(PendingSetup::Disable { operation, .. }) => {
                // Record validation requires this exact active generation.
                Self::PendingDisable {
                    operation: *operation,
                    active: record
                        .active
                        .as_ref()
                        .ok_or(RegistryError::Invalid)?
                        .clone(),
                }
            }
            None => match &record.active {
                Some(active) => Self::Enabled(active.clone()),
                None => Self::Disabled,
            },
        })
    }
}

struct Publication {
    operation: Option<Digest>,
    previous: Option<Vec<u8>>,
    exact: Vec<u8>,
    record: Record,
}
enum EnableCompletion {
    Current,
    EarlierEpochMetadata,
}

/// Pending publication owns exact wrapper/bytes across both definitive I/O failure
/// and uncertain rename. A different operation cannot overwrite it.
pub(super) struct Registry {
    binding: Binding,
    path: PathBuf,
    publisher: Box<dyn Publisher>,
    publication: Option<Publication>,
}
impl Registry {
    /// A fresh OS-created generation must not be an already active/latest key.
    /// Reuse refuses before wrapping, and is never treated as an orphan to delete.
    pub(super) fn require_fresh_generation(
        &self,
        generation: Generation,
    ) -> Result<(), RegistryError> {
        self.require_idle()?;
        if let Some(loaded) = load(&self.path, &self.binding)? {
            if loaded.record.pending.is_some() {
                return Err(RegistryError::Conflict);
            }
            if loaded
                .record
                .active
                .as_ref()
                .is_some_and(|active| active.generation() == generation)
                || loaded
                    .record
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.generation == generation)
            {
                return Err(RegistryError::Stale);
            }
        }
        Ok(())
    }
    /// Update the actual current epoch context for this same permanent copy.
    /// A retained publication's previous/exact bytes and original wrapper stay
    /// untouched, so its lost ACK can still be reconciled after rotation.
    pub(super) fn current_binding(&mut self, binding: Binding) -> Result<(), RegistryError> {
        if self.binding == binding {
            return Ok(());
        }
        if !self.binding.same_copy(&binding) {
            return Err(RegistryError::Stale);
        }
        self.binding = binding;
        Ok(())
    }
    pub(super) fn new(profile: &Path, binding: Binding) -> Result<Self, RegistryError> {
        if !binding.same_profile(profile)? {
            return Err(RegistryError::Invalid);
        }
        let profile = profile.canonicalize().map_err(|_| RegistryError::Io)?;
        let path = profile.join(format!("biometry-{}.json", binding.copy_digest()?));
        Ok(Self {
            binding,
            path,
            publisher: Box::new(FilePublisher),
            publication: None,
        })
    }
    /// Absence is distinct from invalid/unavailable. Loaded bytes are re-published
    /// with file and directory sync before an initial Enabled can be returned.
    pub(super) fn confirmed(&mut self) -> Result<Option<SetupState>, RegistryError> {
        if self.publication.is_some() {
            return Err(RegistryError::Conflict);
        }
        let Some(loaded) = load(&self.path, &self.binding)? else {
            return Ok(None);
        };
        let operation = loaded
            .record
            .pending
            .as_ref()
            .map(PendingSetup::operation)
            .or_else(|| {
                loaded
                    .record
                    .receipt
                    .as_ref()
                    .map(|receipt| receipt.operation)
            });
        self.publication = Some(Publication {
            operation,
            previous: Some(loaded.bytes.clone()),
            exact: loaded.bytes,
            record: loaded.record,
        });
        self.publish_owned().map(Some)
    }
    /// Consume one newly proven candidate. Retry never calls this fresh-key path.
    pub(super) fn begin_enable(
        &mut self,
        proof: ConfirmedEnrollment,
    ) -> Result<SetupState, RegistryError> {
        self.require_idle()?;
        if proof.candidate.binding() != &self.binding {
            return Err(RegistryError::Stale);
        }
        let loaded = load(&self.path, &self.binding)?;
        let previous = loaded.as_ref().map(|loaded| loaded.bytes.clone());
        let mut record = loaded
            .map(|loaded| loaded.record)
            .unwrap_or_else(Record::empty);
        if record.pending.is_some() {
            return Err(RegistryError::Conflict);
        }
        if record
            .receipt
            .as_ref()
            .is_some_and(|receipt| receipt.operation == proof.operation)
        {
            return Err(RegistryError::Stale);
        }
        record.pending = Some(PendingSetup::enable(proof.operation, proof.candidate));
        self.prepare_and_publish(previous, record, proof.operation)
    }
    /// Called only after the fixed broker has resolved the exact OS generation.
    pub(super) fn finish_enable(&mut self, operation: Digest) -> Result<SetupState, RegistryError> {
        self.complete_enable(operation, EnableCompletion::Current)
    }
    /// Dispose only an earlier proof-confirmed epoch's exact metadata phase.
    /// This does not prove current key usability or authorize a prompt/open.
    pub(super) fn finish_earlier_epoch_enable(
        &mut self,
        operation: Digest,
    ) -> Result<SetupState, RegistryError> {
        self.complete_enable(operation, EnableCompletion::EarlierEpochMetadata)
    }
    fn complete_enable(
        &mut self,
        operation: Digest,
        completion: EnableCompletion,
    ) -> Result<SetupState, RegistryError> {
        self.require_idle()?;
        let loaded = load(&self.path, &self.binding)?.ok_or(RegistryError::NoPending)?;
        let mut record = loaded.record;
        let candidate = match &record.pending {
            Some(PendingSetup::Enable {
                operation: pending,
                candidate,
                ..
            }) if *pending == operation => candidate.clone(),
            _ => {
                if matches!(completion, EnableCompletion::EarlierEpochMetadata) {
                    let active = record.active.as_ref().ok_or(RegistryError::Stale)?;
                    self.require_enable_completion(active, &completion)?;
                }
                return self.retry_completed(
                    operation,
                    SetupAction::Enable,
                    None,
                    record,
                    loaded.bytes,
                );
            }
        };
        self.require_enable_completion(&candidate, &completion)?;
        record.receipt = Some(SetupReceipt {
            operation,
            action: SetupAction::Enable,
            generation: candidate.generation(),
        });
        record.active = Some(candidate);
        record.pending = None;
        self.prepare_and_publish(Some(loaded.bytes), record, operation)
    }
    fn require_enable_completion(
        &self,
        candidate: &WrapperRecord,
        completion: &EnableCompletion,
    ) -> Result<(), RegistryError> {
        let allowed = match completion {
            EnableCompletion::Current => candidate.binding() == &self.binding,
            EnableCompletion::EarlierEpochMetadata => {
                candidate.binding().same_copy(&self.binding)
                    && candidate.binding().epoch() < self.binding.epoch()
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(RegistryError::Stale)
        }
    }
    /// Publish the exact selected generation before the fixed broker deletes its
    /// OS key. A stale retry cannot choose a later generation's alias.
    pub(super) fn begin_disable(
        &mut self,
        expected: Generation,
        operation: Digest,
    ) -> Result<SetupState, RegistryError> {
        self.require_idle()?;
        let loaded = load(&self.path, &self.binding)?.ok_or(RegistryError::NoPending)?;
        let mut record = loaded.record;
        if let Some(pending) = &record.pending {
            return match pending {
                PendingSetup::Disable {
                    operation: pending,
                    generation,
                } if *pending == operation && *generation == expected => {
                    self.prepare_and_publish(Some(loaded.bytes), record, operation)
                }
                _ => Err(RegistryError::Conflict),
            };
        }
        if record
            .receipt
            .as_ref()
            .is_some_and(|receipt| receipt.operation == operation)
        {
            return self.retry_completed(
                operation,
                SetupAction::Disable,
                Some(expected),
                record,
                loaded.bytes,
            );
        }
        if !record
            .active
            .as_ref()
            .is_some_and(|active| active.generation() == expected)
        {
            return Err(RegistryError::Stale);
        }
        record.pending = Some(PendingSetup::Disable {
            operation,
            generation: expected,
        });
        self.prepare_and_publish(Some(loaded.bytes), record, operation)
    }
    /// Complete only after exact OS deletion. A later reenrollment is never erased.
    pub(super) fn finish_disable(
        &mut self,
        expected: Generation,
        operation: Digest,
    ) -> Result<SetupState, RegistryError> {
        self.require_idle()?;
        let loaded = load(&self.path, &self.binding)?.ok_or(RegistryError::NoPending)?;
        let mut record = loaded.record;
        match &record.pending {
            Some(PendingSetup::Disable {
                operation: pending,
                generation,
            }) if *pending == operation && *generation == expected => {}
            _ => {
                return self.retry_completed(
                    operation,
                    SetupAction::Disable,
                    Some(expected),
                    record,
                    loaded.bytes,
                );
            }
        }
        record.active = None;
        record.pending = None;
        record.receipt = Some(SetupReceipt {
            operation,
            action: SetupAction::Disable,
            generation: expected,
        });
        self.prepare_and_publish(Some(loaded.bytes), record, operation)
    }
    /// Reconcile only a retained exact pending/latest operation. It never mints a
    /// key, rewraps bytes, or completes a missing OS/proof phase.
    pub(super) fn retry(&mut self, operation: Digest) -> Result<SetupState, RegistryError> {
        if let Some(publication) = &self.publication {
            if publication.operation != Some(operation) {
                return Err(RegistryError::Stale);
            }
            return self.publish_owned();
        }
        let loaded = load(&self.path, &self.binding)?.ok_or(RegistryError::NoPending)?;
        let matches = loaded
            .record
            .pending
            .as_ref()
            .is_some_and(|pending| pending.operation() == operation)
            || loaded
                .record
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.operation == operation);
        if !matches {
            return Err(RegistryError::Stale);
        }
        // The pending operation takes precedence: a receipt for the preceding
        // active wrapper must not bypass a later in-progress disable/reenroll.
        if loaded
            .record
            .pending
            .as_ref()
            .is_some_and(|pending| pending.operation() != operation)
        {
            return Err(RegistryError::Stale);
        }
        self.prepare_and_publish(Some(loaded.bytes), loaded.record, operation)
    }
    fn require_idle(&self) -> Result<(), RegistryError> {
        if self.publication.is_some() {
            Err(RegistryError::Conflict)
        } else {
            Ok(())
        }
    }
    fn retry_completed(
        &mut self,
        operation: Digest,
        action: SetupAction,
        expected: Option<Generation>,
        record: Record,
        bytes: Vec<u8>,
    ) -> Result<SetupState, RegistryError> {
        let receipt = record.receipt.as_ref().ok_or(RegistryError::NoPending)?;
        if record.pending.is_some()
            || receipt.operation != operation
            || receipt.action != action
            || expected.is_some_and(|generation| generation != receipt.generation)
        {
            return Err(RegistryError::Stale);
        }
        self.prepare_and_publish(Some(bytes), record, operation)
    }
    fn prepare_and_publish(
        &mut self,
        previous: Option<Vec<u8>>,
        record: Record,
        operation: Digest,
    ) -> Result<SetupState, RegistryError> {
        let exact = record.encode(&self.binding)?;
        self.publication = Some(Publication {
            operation: Some(operation),
            previous,
            exact,
            record,
        });
        self.publish_owned()
    }
    fn publish_owned(&mut self) -> Result<SetupState, RegistryError> {
        let publication = self.publication.as_ref().ok_or(RegistryError::NoPending)?;
        let actual = load(&self.path, &self.binding)?.map(|loaded| loaded.bytes);
        if actual.as_ref() != publication.previous.as_ref()
            && actual.as_ref() != Some(&publication.exact)
        {
            return Err(RegistryError::Stale);
        }
        // Equal bytes do not prove directory durability; every exact retry calls
        // the real publication boundary again before clearing its owner.
        self.publisher.publish(&self.path, &publication.exact)?;
        let state = SetupState::from_record(&publication.record)?;
        self.publication = None;
        Ok(state)
    }
}

#[cfg(test)]
mod tests;
