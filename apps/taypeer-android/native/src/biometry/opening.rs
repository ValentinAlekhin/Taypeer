//! OS completion never owns OPEN; a real pending worker completes authentication.
use super::{
    BiometricInputSpec, BiometricPromptCall, BiometryFailure,
    broker::{Reservation, random_digest},
    port::AttemptFence,
    refusal,
    registry::SetupState,
    registry_error,
};
use crate::{
    AndroidError, CipherWriter, CiphertextFiles, DocumentProcess, DocumentSession, Host,
    WorkingCopyView,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
use taypeer_runtime::{
    WorkerControl,
    platform_worker::biometry::{BiometricBootAttempt, PendingBiometricOpen},
    session::LockReason,
};

struct Pending {
    worker: PendingBiometricOpen,
    _reservation: Arc<Reservation>,
}
/// One real Opening generation and exact private prompt. No public constructor.
#[derive(uniffi::Object)]
pub struct BiometricOpening {
    pending: Mutex<Option<Pending>>,
    control: WorkerControl,
    broker: Arc<super::Broker>,
    host: Arc<taypeer_runtime::RuntimeHost>,
    process: Arc<dyn DocumentProcess>,
    selected: Arc<crate::SelectedTransfersHost>,
    fence: Arc<AttemptFence>,
    call: Arc<BiometricPromptCall>,
    completion: AtomicU8,
}
const PREPARED: u8 = 0;
const RUNNING: u8 = 1;
const ACCEPTED: u8 = 2;
const CANCELLED: u8 = 3;
const FAILED: u8 = 4;
#[uniffi::export]
impl Host {
    /// Prepare a registered copy with an exact confirmed current wrapper and real
    /// supervised Opening worker. Password, form and recovery parameters are absent.
    /// Run off-main; this method does not prompt or acquire author/read authority.
    pub fn prepare_biometric_open(
        &self,
        copy: WorkingCopyView,
        process: Box<dyn DocumentProcess>,
        files: Box<dyn CiphertextFiles>,
    ) -> Result<Arc<BiometricOpening>, AndroidError> {
        let broker = Arc::clone(self.biometric_broker()?);
        let revision = broker.revision();
        let (binding, managed, writer) = broker.current(&copy)?;
        let reservation = broker.reserve(&binding)?;
        let record = match reservation
            .slot()
            .registry
            .lock()
            .map_err(crate::document::failure)?
            .confirmed()
            .map_err(registry_error)?
        {
            Some(SetupState::Enabled(record)) if record.binding() == &binding => record,
            Some(SetupState::PendingEnable { .. } | SetupState::PendingDisable { .. }) => {
                return Err(refusal(BiometryFailure::Busy));
            }
            _ => return Err(refusal(BiometryFailure::Unavailable)),
        };
        broker.require_available()?;
        let attempt = random_digest()?;
        let key = broker.resolve(&record, attempt)?;
        let launcher = crate::document::session::Launcher {
            writer: Arc::new(CipherWriter {
                writer,
                files: Arc::from(files),
            }),
            process: Arc::from(process),
            selected: Arc::new(crate::SelectedTransfersHost::default()),
        };
        let worker = match self.runtime.begin_platform_biometric_open(
            &launcher,
            BiometricBootAttempt {
                namespace: binding.namespace(),
                managed,
                attempt,
                input: random_digest()?,
            },
        ) {
            Ok(worker) => worker,
            Err(error) => {
                broker.port.retire_lease(key.id);
                return Err(crate::document::runtime(error));
            }
        };
        let control = worker.control();
        let fence = AttemptFence::new(
            worker.challenge().clone(),
            Arc::clone(&launcher.process),
            Arc::clone(&broker.port),
            Arc::clone(&reservation),
            control.clone(),
        );
        let prepared = (|| {
            fence.bind_os(key.id.clone())?;
            broker.track(&fence, revision)?;
            if control.status().phase != taypeer_runtime::session::SessionPhase::Opening {
                return Err(refusal(BiometryFailure::Stale));
            }
            let call =
                BiometricPromptCall::new(Arc::clone(&fence), key.id.clone(), *record.ciphertext());
            Ok(Arc::new(BiometricOpening {
                pending: Mutex::new(Some(Pending {
                    worker,
                    _reservation: reservation,
                })),
                control: control.clone(),
                broker: Arc::clone(&broker),
                host: Arc::clone(&self.runtime),
                process: launcher.process,
                selected: launcher.selected,
                fence: Arc::clone(&fence),
                call,
                completion: AtomicU8::new(PREPARED),
            }))
        })();
        if prepared.is_err() {
            fence.retire();
            control.invalidate(LockReason::Transport);
        }
        prepared
    }
}
#[uniffi::export]
impl BiometricOpening {
    /// Exact challenge, including the real supervised process generation.
    pub fn identity(&self) -> BiometricInputSpec {
        self.call.scope()
    }
    /// Await actual OS writer completion outside a worker lock, then consume the
    /// pending worker. Key authentication and author/application checks precede
    /// the returned session. Cancel/stale generations cannot return an OPEN handle.
    pub fn complete(&self) -> Result<Arc<DocumentSession>, AndroidError> {
        self.fence.check()?;
        self.completion
            .compare_exchange(PREPARED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refusal(BiometryFailure::Stale))?;
        let pending = self
            .pending
            .lock()
            .map_err(crate::document::failure)?
            .take()
            .ok_or_else(|| refusal(BiometryFailure::Stale))?;
        let result = (|| {
            self.call.run(self.broker.port.as_ref())?;
            let worker = self
                .host
                .complete_platform_biometric_open(pending.worker, self.fence.lease())
                .map_err(crate::document::runtime)?;
            self.fence.check()?;
            if !worker.is_open() {
                return Err(refusal(BiometryFailure::Stale));
            }
            let session = DocumentSession::from_platform_worker(
                worker,
                Arc::clone(&self.host),
                Some(Arc::clone(&self.broker)),
                Arc::clone(&self.process),
                Arc::clone(&self.selected),
            )?;
            self.fence.check()?;
            // Cancellation and handing custody to the caller have one atomic
            // winner. An accepted opening is no longer a pending launch handle.
            self.completion
                .compare_exchange(RUNNING, ACCEPTED, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| refusal(BiometryFailure::Cancelled))?;
            Ok(session)
        })();
        if result.is_err() {
            let _ = self.completion.compare_exchange(
                RUNNING,
                FAILED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        self.fence.retire();
        if result.is_err() {
            self.control.invalidate(LockReason::Transport);
        }
        result
    }
    /// Retire Native input first, then revoke actual pending supervision without
    /// waiting for prompt, Binder, pipe read or worker command mutex.
    pub fn cancel(&self) {
        if self
            .completion
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |phase| {
                if matches!(phase, PREPARED | RUNNING) {
                    Some(CANCELLED)
                } else {
                    None
                }
            })
            .is_err()
        {
            return;
        }
        self.fence.retire();
        self.control.invalidate(LockReason::Manual);
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        }
    }
}
impl Drop for BiometricOpening {
    fn drop(&mut self) {
        self.fence.retire();
        // A successfully completed worker belongs to DocumentSession. Dropping
        // the UI's consumed opening must not revoke that independently held session.
        if self
            .pending
            .get_mut()
            .is_ok_and(|pending| pending.is_some())
        {
            self.control.invalidate(LockReason::HostExited);
        }
    }
}
