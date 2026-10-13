//! Fixed Android OS adapter, exact wrapper publication and one-shot private input.
mod broker;
pub(crate) mod enrollment;
mod inputs;
mod key_fd;
mod opening;
mod port;
mod registry;
mod spec;
mod status;

pub(crate) use broker::Broker;
pub use enrollment::BiometricEnrollment;
pub use inputs::PrivateBiometricInputs;
pub use opening::BiometricOpening;
pub use port::{
    BiometricAvailability, BiometricPromptCall, BiometricPromptOutcome, OsBiometricKeyLease,
    OsBiometryPort,
};
pub use spec::{BiometricBindingView, BiometricInputSpec, BiometricPurpose};
pub use status::{BiometryState, BiometryStatusView};

/// Content-free local biometric refusal; it never represents authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BiometryFailure {
    /// OS strong biometry or the exact auth-bound key is unavailable.
    Unavailable,
    /// This one-shot attempt was irreversibly cancelled.
    Cancelled,
    /// Copy, epoch, generation or exact operation no longer matches.
    Stale,
    /// No exact retained setup exists for this retry.
    NoPending,
    /// Another exact operation owns this copy's publication or prompt.
    Busy,
    /// This worker exhausted its bounded one-shot inputs.
    Limit,
}
pub(super) fn refusal(failure: BiometryFailure) -> crate::AndroidError {
    crate::AndroidError::Biometry { failure }
}
fn registry_error(error: registry::RegistryError) -> crate::AndroidError {
    use registry::RegistryError;
    match error {
        RegistryError::Invalid => crate::AndroidError::ProfileInvalid,
        RegistryError::Io => crate::AndroidError::ProfileIo,
        RegistryError::CommitUncertain => crate::AndroidError::CommitUncertain,
        RegistryError::Stale => refusal(BiometryFailure::Stale),
        RegistryError::NoPending => refusal(BiometryFailure::NoPending),
        RegistryError::Conflict => refusal(BiometryFailure::Busy),
    }
}

#[cfg(feature = "android-test-support")]
/// Verify one PUBLIC pipe in the isolated Native entry, never return its bytes.
///
/// The private Binder caller holds its original PFD across synchronous duplication.
/// Only a blocking read-only FIFO is accepted. The owned duplicate must contain
/// exactly 32 bytes of the fixed PUBLIC pattern followed by EOF. A different
/// 32-byte pattern returns false; malformed length, descriptor and I/O are errors.
/// The duplicate and worker-owned scratch/key are released on every return.
///
/// This support-only function is absent from release bindings. It is not an unlock
/// API and must never be called with a user key or routed through ordinary streams.
#[uniffi::export]
pub fn verify_public_biometric_pipe(fd: i32) -> Result<bool, crate::AndroidError> {
    key_fd::verify_public(fd)
}
