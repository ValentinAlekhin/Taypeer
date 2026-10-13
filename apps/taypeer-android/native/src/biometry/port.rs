//! The immutable bootstrap OS port receives opaque calls, never document keys.
use super::{BiometricBindingView, BiometricInputSpec, BiometryFailure, refusal};
use crate::{AndroidError, DocumentProcess};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use taypeer_runtime::platform_worker::biometry::BiometricInputLease;

/// Actual OS strong-biometric availability, not a database access grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BiometricAvailability {
    /// Strong biometric sensor and enrollment are available.
    Available,
    /// No suitable hardware exists.
    NoHardware,
    /// Strong biometrics have not been enrolled.
    NoEnrollment,
    /// The OS adapter cannot safely perform this operation.
    Unavailable,
}
/// Completion of the private OS writer, never proof of an OPEN document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BiometricPromptOutcome {
    /// JNI wrote exactly32 bytes and closed its sole writer.
    Written,
    /// OS/user cancellation ended this attempt.
    Cancelled,
    /// OS policy, enrollment or key is unavailable.
    Unavailable,
    /// The private writer/provider operation failed.
    Failed,
}
/// Validated by Native against the fixed port's exact binding and key metadata.
#[derive(Clone, uniffi::Record)]
pub struct OsBiometricKeyLease {
    /// Opaque fixed-port lease ID, lowercase64 hex.
    pub id: String,
    /// Actual randomly generated alias suffix, lowercase32 hex.
    pub generation: String,
    /// Permanent profile/copy/root/epoch binding.
    pub binding: BiometricBindingView,
    /// Exact Native attempt, lowercase64 hex.
    pub attempt: String,
    /// Scoped alias constructed by the fixed adapter, never supplied by UI.
    pub alias: String,
    /// Public canonical RSA SPKI, bounded4096 bytes.
    pub spki: Vec<u8>,
}
impl std::fmt::Debug for OsBiometricKeyLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OsBiometricKeyLease([redacted])")
    }
}
/// Installed once at Host construction. Implementations enforce actual KeyInfo
/// and strong per-use CryptoObject policy; none returns plaintext key bytes.
#[uniffi::export(callback_interface)]
pub trait OsBiometryPort: Send + Sync {
    /// Inspect strong-biometric availability without prompting.
    fn availability(&self) -> Result<BiometricAvailability, AndroidError>;
    /// Create one auth-bound generation for Native's exact current scope.
    fn create_key(
        &self,
        binding: BiometricBindingView,
        attempt: String,
    ) -> Result<OsBiometricKeyLease, AndroidError>;
    /// Resolve an existing exact generation and inspect actual key properties.
    fn resolve_key(
        &self,
        binding: BiometricBindingView,
        generation: String,
        alias: String,
        spki_digest: String,
        attempt: String,
    ) -> Result<OsBiometricKeyLease, AndroidError>;
    /// Wait off-main for OS prompt and private JNI32+EOF completion.
    fn run_prompt(
        &self,
        call: Arc<BiometricPromptCall>,
    ) -> Result<BiometricPromptOutcome, AndroidError>;
    /// Enqueue nonblocking exact writer/prompt cleanup. No key is deleted here.
    fn retire_lease(&self, id: String);
    /// Delete only this exact scoped key generation; already absent is success.
    fn delete_key(
        &self,
        binding: BiometricBindingView,
        generation: String,
        alias: String,
    ) -> Result<(), AndroidError>;
}

/// Immediate Native fence independent of OS/provider completion and worker mutex.
pub(super) struct AttemptFence {
    lease: BiometricInputLease,
    process: Arc<dyn DocumentProcess>,
    port: Arc<dyn OsBiometryPort>,
    retired: AtomicBool,
    os_lease: Mutex<Option<String>>,
    reservation: Option<Arc<super::broker::Reservation>>,
    control: Option<taypeer_runtime::WorkerControl>,
}
impl AttemptFence {
    pub(super) fn new(
        lease: BiometricInputLease,
        process: Arc<dyn DocumentProcess>,
        port: Arc<dyn OsBiometryPort>,
        reservation: Arc<super::broker::Reservation>,
        control: taypeer_runtime::WorkerControl,
    ) -> Arc<Self> {
        Self::owned(lease, process, port, Some(reservation), Some(control))
    }
    fn owned(
        lease: BiometricInputLease,
        process: Arc<dyn DocumentProcess>,
        port: Arc<dyn OsBiometryPort>,
        reservation: Option<Arc<super::broker::Reservation>>,
        control: Option<taypeer_runtime::WorkerControl>,
    ) -> Arc<Self> {
        Arc::new(Self {
            lease,
            process,
            port,
            retired: AtomicBool::new(false),
            os_lease: Mutex::new(None),
            reservation,
            control,
        })
    }
    pub(super) fn lease(&self) -> &BiometricInputLease {
        &self.lease
    }
    pub(super) fn check(&self) -> Result<(), AndroidError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(refusal(BiometryFailure::Cancelled));
        }
        if let Some(control) = &self.control {
            let actual = control.status();
            let live = match self.lease.purpose {
                taypeer_runtime::platform_worker::biometry::BiometricInputPurpose::EnrollmentProof => actual.phase == taypeer_runtime::session::SessionPhase::Open,
                taypeer_runtime::platform_worker::biometry::BiometricInputPurpose::KeyedBoot => matches!(actual.phase, taypeer_runtime::session::SessionPhase::Opening | taypeer_runtime::session::SessionPhase::Open),
            };
            if actual.generation != self.lease.scope.generation || !live {
                self.retire();
                return Err(refusal(BiometryFailure::Stale));
            }
        }
        Ok(())
    }
    pub(super) fn bind_os(&self, id: String) -> Result<(), AndroidError> {
        let mut current = self.os_lease.lock().map_err(crate::document::failure)?;
        if current.is_some() {
            return Err(refusal(BiometryFailure::Stale));
        }
        *current = Some(id.clone());
        drop(current);
        if self.check().is_err() {
            self.port.retire_lease(id);
            return Err(refusal(BiometryFailure::Cancelled));
        }
        Ok(())
    }
    pub(super) fn retire(&self) {
        if self.retired.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(reservation) = &self.reservation {
            reservation.release();
        }
        // Callback contract enqueues independent Binder control without waiting.
        self.process
            .revoke_biometric_input(BiometricInputSpec::from(&self.lease));
        let id = self.os_lease.lock().ok().and_then(|id| id.clone());
        if let Some(id) = id {
            self.port.retire_lease(id);
        }
    }
}
impl Drop for AttemptFence {
    fn drop(&mut self) {
        self.retire();
    }
}
/// Opaque call handed only to the fixed bootstrap OS adapter. No public ctor.
#[derive(uniffi::Object)]
pub struct BiometricPromptCall {
    fence: Arc<AttemptFence>,
    lease_id: String,
    ciphertext: [u8; 256],
    installed: AtomicBool,
}
impl BiometricPromptCall {
    pub(super) fn new(
        fence: Arc<AttemptFence>,
        lease_id: String,
        ciphertext: [u8; 256],
    ) -> Arc<Self> {
        Arc::new(Self {
            fence,
            lease_id,
            ciphertext,
            installed: AtomicBool::new(false),
        })
    }
    pub(super) fn check(&self) -> Result<(), AndroidError> {
        self.fence.check()
    }
    pub(super) fn run(self: &Arc<Self>, port: &dyn OsBiometryPort) -> Result<(), AndroidError> {
        self.check()?;
        let result = port.run_prompt(Arc::clone(self));
        self.check()?;
        match result? {
            BiometricPromptOutcome::Written if self.installed.load(Ordering::Acquire) => Ok(()),
            BiometricPromptOutcome::Written => Err(refusal(BiometryFailure::Stale)),
            BiometricPromptOutcome::Cancelled => Err(refusal(BiometryFailure::Cancelled)),
            BiometricPromptOutcome::Unavailable | BiometricPromptOutcome::Failed => {
                Err(refusal(BiometryFailure::Unavailable))
            }
        }
    }
}
#[cfg(test)]
mod tests;
#[uniffi::export]
impl BiometricPromptCall {
    /// Exact separately armed input scope, not an authentication claim.
    pub fn scope(&self) -> BiometricInputSpec {
        BiometricInputSpec::from(self.fence.lease())
    }
    /// Fixed OS key lease ID for this single prompt.
    pub fn lease_id(&self) -> String {
        self.lease_id.clone()
    }
    /// RSA ciphertext only; never returns a ReadKey or master password.
    pub fn wrapped_ciphertext(&self) -> Vec<u8> {
        self.ciphertext.to_vec()
    }
    /// Synchronically duplicate the caller-held reader in the exact isolated owner.
    pub fn install_input(&self, fd: i32) -> Result<(), AndroidError> {
        self.check()?;
        if self.installed.swap(true, Ordering::AcqRel) {
            return Err(refusal(BiometryFailure::Stale));
        }
        self.fence
            .process
            .install_biometric_input(self.scope(), fd)?;
        self.check()
    }
    /// Terminal Native fence first; OS cleanup is merely queued afterward.
    pub fn retire_input(&self) {
        self.fence.retire();
    }
}
