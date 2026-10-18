//! Current password provenance, one real OS round-trip, then durable local setup.
use super::{
    BiometricBindingView, BiometricInputSpec, BiometricPromptCall, BiometryFailure,
    BiometryStatusView,
    broker::{Reservation, random_digest},
    port::AttemptFence,
    refusal,
    registry::{Binding, ConfirmedEnrollment, SetupState, WrapperRecord},
    registry_error,
};
use crate::{AndroidError, DocumentSession};
use std::sync::{Arc, Mutex};
use taypeer_runtime::platform_worker::biometry::{
    BiometricAttemptScope, BiometricInputLease, BiometricInputPurpose,
};

struct Prepared {
    candidate: WrapperRecord,
    reservation: Arc<Reservation>,
}
enum EnrollmentState {
    Prepared(Box<Prepared>),
    Running,
    Finished,
}
/// Exact one-shot enrollment. UI cannot choose SPKI, scope, FD or proof boolean.
#[derive(uniffi::Object)]
pub struct BiometricEnrollment {
    worker: Arc<Mutex<taypeer_runtime::Worker>>,
    control: taypeer_runtime::WorkerControl,
    broker: Arc<super::Broker>,
    binding: Binding,
    call: Arc<BiometricPromptCall>,
    fence: Arc<AttemptFence>,
    state: Mutex<EnrollmentState>,
}
#[uniffi::export]
impl DocumentSession {
    /// Prepare only a current password-authenticated worker and exact OS key.
    /// All OS work is off-main; no worker mutex is held during callbacks.
    pub fn prepare_biometric_enrollment(&self) -> Result<Arc<BiometricEnrollment>, AndroidError> {
        let broker = Arc::clone(
            self.biometry
                .as_ref()
                .ok_or_else(|| refusal(BiometryFailure::Unavailable))?,
        );
        let revision = broker.revision();
        let managed = self.biometric_worker(|worker| worker.platform_biometric_scope())?;
        let binding = broker.binding(&managed)?;
        let reservation = broker.reserve(&binding)?;
        // Unknown/corrupt registry must refuse before creating another alias.
        let existing = reservation
            .slot()
            .registry
            .lock()
            .map_err(crate::document::failure)?
            .confirmed()
            .map_err(registry_error)?;
        if matches!(
            existing,
            Some(SetupState::PendingEnable { .. } | SetupState::PendingDisable { .. })
        ) {
            return Err(refusal(BiometryFailure::Busy));
        }
        broker.require_available()?;
        let lease = BiometricInputLease {
            scope: BiometricAttemptScope {
                namespace: binding.namespace(),
                managed,
                generation: self.generation(),
                attempt: random_digest()?,
            },
            input: random_digest()?,
            purpose: BiometricInputPurpose::EnrollmentProof,
        };
        let fence = AttemptFence::new(
            lease.clone(),
            Arc::clone(&self.process),
            Arc::clone(&broker.port),
            Arc::clone(&reservation),
            self.control.clone(),
        );
        broker.track(&fence, revision)?;
        let key = broker.port.create_key(
            BiometricBindingView::from_binding(&binding)?,
            lease.scope.attempt.to_string(),
        )?;
        let mut can_discard = false;
        let prepared = (|| {
            let (generation, spki) = broker.validate_key(&key, &binding, lease.scope.attempt)?;
            reservation
                .slot()
                .registry
                .lock()
                .map_err(crate::document::failure)?
                .require_fresh_generation(generation)
                .map_err(registry_error)?;
            can_discard = true;
            fence.bind_os(key.id.clone())?;
            fence.check()?;
            let wrapped = self.biometric_worker(|worker| {
                worker.platform_prepare_biometric_enrollment(lease.clone(), key.spki.clone())
            })?;
            if wrapped.spki != spki || wrapped.lease != lease {
                return Err(refusal(BiometryFailure::Stale));
            }
            fence.check()?;
            let ciphertext: [u8; 256] = wrapped
                .ciphertext
                .try_into()
                .map_err(|_| refusal(BiometryFailure::Stale))?;
            let candidate = WrapperRecord::new(binding.clone(), generation, spki, ciphertext)
                .map_err(registry_error)?;
            let call = BiometricPromptCall::new(Arc::clone(&fence), key.id.clone(), ciphertext);
            Ok(Arc::new(BiometricEnrollment {
                worker: Arc::clone(&self.worker),
                control: self.control.clone(),
                broker: Arc::clone(&broker),
                binding: binding.clone(),
                call,
                fence: Arc::clone(&fence),
                state: Mutex::new(EnrollmentState::Prepared(Box::new(Prepared {
                    candidate,
                    reservation,
                }))),
            }))
        })();
        if prepared.is_err() {
            fence.retire();
            // No worker proof/publication has begun, so this exact created key is
            // an orphan. Cleanup failure cannot substitute a successful prepare.
            broker.port.retire_lease(key.id.clone());
            if can_discard {
                let _ = broker
                    .port
                    .delete_key(key.binding, key.generation, key.alias);
            }
        }
        prepared
    }
}
#[uniffi::export]
impl BiometricEnrollment {
    /// Public one-shot identity; it never contains a key or grants authentication.
    pub fn identity(&self) -> BiometricInputSpec {
        self.call.scope()
    }
    /// Perform the OS round-trip without a worker lock, then consume the actual
    /// worker proof and publish exact bytes. Uncertain publication survives this
    /// object's lifetime and is retried through Host with this operation.
    pub fn confirm(&self) -> Result<BiometryStatusView, AndroidError> {
        self.fence.check()?;
        let prepared = {
            let mut state = self.state.lock().map_err(crate::document::failure)?;
            match std::mem::replace(&mut *state, EnrollmentState::Running) {
                EnrollmentState::Prepared(prepared) => prepared,
                previous => {
                    *state = previous;
                    return Err(refusal(BiometryFailure::Stale));
                }
            }
        };
        let mut publication_started = false;
        let result = (|| {
            self.call.run(self.broker.port.as_ref())?;
            let proof = with_open_worker(&self.worker, &self.control, |worker| {
                worker.platform_confirm_biometric_enrollment(self.fence.lease().clone())
            })?;
            self.fence.check()?;
            let proof = ConfirmedEnrollment::consume_worker_proof(
                proof,
                self.fence.lease(),
                prepared.candidate.clone(),
            )
            .map_err(registry_error)?;
            // Once the serialized exact owner starts, cancellation must retain
            // its candidate/alias for reconciliation, even when disk ACK is lost.
            publication_started = true;
            let state = {
                let mut registry = prepared
                    .reservation
                    .slot()
                    .registry
                    .lock()
                    .map_err(crate::document::failure)?;
                self.fence.check()?;
                prepared.reservation.check()?;
                registry.begin_enable(proof).map_err(registry_error)?
            };
            self.fence.check()?;
            let state = self.broker.finish_setup(&prepared.reservation, state)?;
            self.fence.check()?;
            self.broker.require_current(&self.binding)?;
            let view = self.broker.view(&self.binding, Some(state))?;
            self.fence.check()?;
            Ok(view)
        })();
        self.fence.retire();
        *self.state.lock().map_err(crate::document::failure)? = EnrollmentState::Finished;
        if result.is_err() && !publication_started {
            // Only this fresh unproven generation is eligible for deletion.
            let _ = self.broker.port.delete_key(
                BiometricBindingView::from_binding(prepared.candidate.binding())?,
                prepared.candidate.generation().to_string(),
                prepared.candidate.alias().to_owned(),
            );
        }
        result
    }
    /// Immediate terminal Native revocation. Any already-started publication is
    /// retained; it is never rolled back or represented as a false Disabled ACK.
    pub fn cancel(&self) {
        self.fence.retire();
        // A running confirmation owns its reservation until it observes the
        // fence. Cancelling a prepared attempt releases it without waiting.
        if let Ok(mut state) = self.state.lock()
            && matches!(*state, EnrollmentState::Prepared(_))
            && let EnrollmentState::Prepared(prepared) =
                std::mem::replace(&mut *state, EnrollmentState::Finished)
        {
            self.broker.discard_orphan(prepared.candidate);
        }
    }
}
pub(crate) fn with_open_worker<T>(
    worker: &Mutex<taypeer_runtime::Worker>,
    control: &taypeer_runtime::WorkerControl,
    action: impl FnOnce(&mut taypeer_runtime::Worker) -> Result<T, taypeer_runtime::RuntimeError>,
) -> Result<T, AndroidError> {
    if !control.is_open() {
        return Err(refusal(BiometryFailure::Stale));
    }
    let mut worker = worker.lock().map_err(crate::document::failure)?;
    if !worker.is_open() {
        return Err(refusal(BiometryFailure::Stale));
    }
    let result = action(&mut worker).map_err(crate::document::runtime)?;
    if !control.is_open() {
        return Err(refusal(BiometryFailure::Stale));
    }
    Ok(result)
}
impl Drop for BiometricEnrollment {
    fn drop(&mut self) {
        self.fence.retire();
        if let Ok(state) = self.state.get_mut()
            && matches!(*state, EnrollmentState::Prepared(_))
            && let EnrollmentState::Prepared(prepared) =
                std::mem::replace(state, EnrollmentState::Finished)
        {
            self.broker.discard_orphan(prepared.candidate);
        }
    }
}
