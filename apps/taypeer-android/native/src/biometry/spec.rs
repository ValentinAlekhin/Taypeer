//! Typed public stamps for the private OS/Binder handoff; no key bytes or paths.
use crate::AndroidError;
use taypeer_core::DatabaseId;
use taypeer_runtime::platform_worker::biometry::{
    BiometricAttemptScope, BiometricInputLease, BiometricInputPurpose,
};
use taypeer_services::ManagedKeyScope;

/// Exact purpose of one private worker input. It never grants access by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BiometricPurpose {
    /// Verify a new wrapper in the current password-authenticated worker.
    EnrollmentProof,
    /// Authenticate a pending worker before it acquires an author capability.
    KeyedBoot,
}

/// Public, exact scope of one private input, passed only by platform bootstrap.
/// No field contains a key, password, SPKI, alias, document value or path.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct BiometricInputSpec {
    /// Canonical private profile digest supplied by Native.
    pub namespace: String,
    /// Registered local working-copy identity.
    pub working_copy: String,
    /// Logical database identity under the shared UTF-8 identifier bound.
    pub database: String,
    /// Pinned authority root.
    pub root: String,
    /// Current key epoch; historical keys are never accepted for this input.
    pub epoch: u64,
    /// Exact current authority head.
    pub control: String,
    /// Exact ciphertext generation captured for this attempt.
    pub snapshot: String,
    /// Current checkpoint or baseline selected by the shared Rust helper.
    pub checkpoint: String,
    /// Actual supervised process generation.
    pub generation: u64,
    /// One Native-minted random attempt identity.
    pub attempt: String,
    /// One Native-minted random input identity, never a numeric FD.
    pub input: String,
    /// Operation allowed to consume the private input once.
    pub purpose: BiometricPurpose,
}
impl From<&BiometricInputLease> for BiometricInputSpec {
    fn from(lease: &BiometricInputLease) -> Self {
        let managed = &lease.scope.managed;
        Self {
            namespace: lease.scope.namespace.to_string(),
            working_copy: managed.working_copy.to_string(),
            database: managed.database.to_string(),
            root: managed.root.to_string(),
            epoch: managed.epoch,
            control: managed.control.to_string(),
            snapshot: managed.snapshot.to_string(),
            checkpoint: managed.checkpoint.to_string(),
            generation: lease.scope.generation,
            attempt: lease.scope.attempt.to_string(),
            input: lease.input.to_string(),
            purpose: match lease.purpose {
                BiometricInputPurpose::EnrollmentProof => BiometricPurpose::EnrollmentProof,
                BiometricInputPurpose::KeyedBoot => BiometricPurpose::KeyedBoot,
            },
        }
    }
}
impl BiometricInputSpec {
    pub(super) fn checked(&self) -> Result<BiometricInputLease, AndroidError> {
        if self.database.is_empty() || self.database.len() > 128 || self.generation == 0 {
            return Err(AndroidError::InvalidOptions);
        }
        let digest = canonical_digest;
        Ok(BiometricInputLease {
            scope: BiometricAttemptScope {
                namespace: digest(&self.namespace)?,
                managed: ManagedKeyScope {
                    working_copy: digest(&self.working_copy)?,
                    database: DatabaseId::new(self.database.clone()),
                    root: digest(&self.root)?,
                    epoch: self.epoch,
                    control: digest(&self.control)?,
                    snapshot: digest(&self.snapshot)?,
                    checkpoint: digest(&self.checkpoint)?,
                },
                generation: self.generation,
                attempt: digest(&self.attempt)?,
            },
            input: digest(&self.input)?,
            purpose: match self.purpose {
                BiometricPurpose::EnrollmentProof => BiometricInputPurpose::EnrollmentProof,
                BiometricPurpose::KeyedBoot => BiometricInputPurpose::KeyedBoot,
            },
        })
    }
}
fn canonical_digest(value: &str) -> Result<taypeer_trust::Digest, AndroidError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AndroidError::InvalidOptions);
    }
    crate::document::digest(value)
}
impl std::fmt::Debug for BiometricInputSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BiometricInputSpec([redacted])")
    }
}

/// Permanent binding supplied by the fixed Native owner to its OS key adapter.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct BiometricBindingView {
    /// Digest used in the strictly scoped OS alias; contains no human name.
    pub scope_digest: String,
    /// Profile namespace digest.
    pub namespace: String,
    /// Registered local working-copy identity.
    pub working_copy: String,
    /// Logical database identity.
    pub database: String,
    /// Pinned authority root.
    pub root: String,
    /// Current read-key epoch.
    pub epoch: u64,
}
impl BiometricBindingView {
    pub(super) fn from_binding(binding: &super::registry::Binding) -> Result<Self, AndroidError> {
        Ok(Self {
            scope_digest: binding
                .permanent_digest()
                .map_err(super::registry_error)?
                .to_string(),
            namespace: binding.namespace().to_string(),
            working_copy: binding.working_copy().to_string(),
            database: binding.database().to_string(),
            root: binding.root().to_string(),
            epoch: binding.epoch(),
        })
    }
}
impl std::fmt::Debug for BiometricBindingView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BiometricBindingView([redacted])")
    }
}
