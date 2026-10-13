//! Private scoped key-FD operations. The host protocol carries bindings/ciphertext only.
mod boot;
mod enrollment;
mod host;
mod pending;
mod wrapping;

use super::PlatformDocument;
use crate::RuntimeError;
use serde::{Deserialize, Serialize};
use std::{fmt, io::Read};
use taypeer_services::{DatabaseService, ManagedKeyScope};
use taypeer_trust::Digest;

pub(super) use boot::open_keyed;
pub(super) use enrollment::Enrollment;
pub use pending::{BiometricBootAttempt, PendingBiometricOpen};
pub use wrapping::validate_biometric_spki;

/// Nonsecret identity of one private OS/worker attempt; never a key or author grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiometricAttemptScope {
    /// Canonical private profile namespace supplied by the fixed Native owner.
    pub namespace: Digest,
    /// Shared exact copy/current-checkpoint scope.
    pub managed: ManagedKeyScope,
    /// Actual supervised process generation, not a UI-selected counter.
    pub generation: u64,
    /// One Native-minted random attempt identity.
    pub attempt: Digest,
}
/// Purpose is part of the exact private lease; enrollment cannot satisfy a keyed boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiometricInputPurpose {
    /// Prove that the wrapped current password-authenticated key can return through the OS.
    EnrollmentProof,
    /// Authenticate an opening worker without retaining a master password.
    KeyedBoot,
}
/// One armed private input, scoped independently of ordinary worker streams.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiometricInputLease {
    /// Exact profile/copy/process/attempt binding.
    pub scope: BiometricAttemptScope,
    /// Native-minted input identity; it is never a numeric FD.
    pub input: Digest,
    /// The only operation allowed to consume this input.
    pub purpose: BiometricInputPurpose,
}
impl BiometricInputLease {
    pub(super) fn require(&self, purpose: BiometricInputPurpose) -> Result<(), RuntimeError> {
        if self.scope.generation == 0 || self.purpose != purpose {
            return Err(RuntimeError::Protocol);
        }
        Ok(())
    }
}
/// Trusted private Native input owner installed once by the isolated worker bootstrap.
/// No method returns bytes to Kotlin, the host, selected-file ports or generic streams.
pub trait BiometricInputs: Send + Sync {
    /// Arm one exact validated lease; consumed/retired attempts cannot be rearmed.
    fn arm(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError>;
    /// Recheck the independent Native lifecycle fence, including after consumption.
    /// This is an access check, never an OS/database-authentication success boolean.
    fn check(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError>;
    /// Transfer the accepted owned FD reader once, exclusively to the plaintext worker.
    /// The OS writer must already have completed 32 bytes and EOF before this call.
    fn consume(&self, lease: &BiometricInputLease) -> Result<Box<dyn Read + Send>, RuntimeError>;
    /// Terminally revoke this slot, even if its FD has not arrived. Repeated revoke is safe.
    fn retire(&self, lease: &BiometricInputLease);
}
/// Ciphertext result of the private enrollment phase; no plaintext key is serialized.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiometricWrappedKey {
    /// Exact private proof input armed by this worker.
    pub lease: BiometricInputLease,
    /// Digest of the fixed OS owner's verified SPKI, not its secret key.
    pub spki: Digest,
    /// Exactly 256 bytes using the fixed RSA/OAEP scheme. Native bounds it again on receipt.
    pub ciphertext: Vec<u8>,
}
/// Content-free result of an actual current-key round-trip inside the worker.
/// Native may mint its private publication owner only after receiving this response.
#[derive(Debug)]
pub struct BiometricEnrollmentProof {
    lease: BiometricInputLease,
}
impl BiometricEnrollmentProof {
    /// Consume this actual worker proof and transfer its exact authenticated lease.
    /// This type has no public constructor, clone or serialized representation.
    pub fn into_lease(self) -> BiometricInputLease {
        self.lease
    }
}
impl fmt::Debug for BiometricWrappedKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BiometricWrappedKey")
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope {
    pub(super) biometry: Request,
}
#[derive(Serialize, Deserialize)]
pub(super) enum Request {
    EnrollmentScope,
    PrepareEnrollment {
        lease: BiometricInputLease,
        spki: Vec<u8>,
    },
    ConfirmEnrollment(BiometricInputLease),
    Cancel(BiometricInputLease),
    CompleteBoot(BiometricInputLease),
}
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Incoming {
    Biometry(Envelope),
    Command(crate::Command),
}

pub(super) fn current_scope(
    document: &dyn PlatformDocument,
) -> Result<ManagedKeyScope, RuntimeError> {
    let snapshot = document
        .snapshot()
        .map_err(taypeer_services::ServiceError::from)?;
    DatabaseService::archive_key_scope(&snapshot, document.working_copy()).map_err(Into::into)
}

pub(super) fn require_scope(
    actual: &ManagedKeyScope,
    lease: &BiometricInputLease,
) -> Result<(), RuntimeError> {
    if actual != &lease.scope.managed {
        return Err(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::Changed).into(),
        );
    }
    Ok(())
}

struct RetireInput<'a> {
    inputs: &'a dyn BiometricInputs,
    lease: &'a BiometricInputLease,
}
impl Drop for RetireInput<'_> {
    fn drop(&mut self) {
        self.inputs.retire(self.lease);
    }
}

#[cfg(all(test, unix))]
mod tests;
