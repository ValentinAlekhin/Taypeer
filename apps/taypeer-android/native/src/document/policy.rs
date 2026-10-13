//! Content-free actual policy/usage/KDF views; calibration never publishes a policy.
use super::{DocumentSession, PolicyView};
use crate::AndroidError;
use taypeer_runtime::Command;
use taypeer_services::{DatabasePolicyDetails, StorageUsage};
use taypeer_storage::{KdfCalibration, KdfParameters};

/// The authenticated password-wrapper algorithm supported by the current format.
#[derive(uniffi::Enum)]
pub enum KdfAlgorithm {
    /// Existing version-one Argon2id parameters, without a new crypto policy.
    Argon2id,
}

/// Actual parameters, distinct from a requested wall-clock calibration budget.
#[derive(uniffi::Record)]
pub struct KdfParametersView {
    /// Password-wrapper algorithm.
    pub algorithm: KdfAlgorithm,
    /// Actual Argon2id memory cost in KiB.
    pub memory_kib: u32,
    /// Actual Argon2id lane count.
    pub parallelism: u32,
    /// Number of passes encoded by the authenticated current header.
    pub iterations: u32,
}
impl From<KdfParameters> for KdfParametersView {
    fn from(value: KdfParameters) -> Self {
        Self {
            algorithm: KdfAlgorithm::Argon2id,
            memory_kib: value.memory_kib,
            parallelism: value.parallelism,
            iterations: value.iterations,
        }
    }
}

/// Product quota usage remains separate from retention, drafts and physical ciphertext.
#[derive(uniffi::Record)]
pub struct DatabaseUsageView {
    /// Unique accessible attachments in active data, history, trash and alternatives.
    pub attachment_bytes: u64,
    /// Applied total attachment quota, without a fabricated default.
    pub attachment_limit: u64,
    /// Accepted offline merges can exceed the quota.
    pub over_limit: bool,
    /// Unique document/pending retained content bytes, including icons.
    pub retained_bytes: u64,
    /// Separate locally staged content bytes.
    pub draft_bytes: u64,
    /// Physical encrypted working-file size.
    pub file_bytes: u64,
    /// At least one retained reference lacks its actual content.
    pub has_missing_contents: bool,
    /// Pending references are not fully known; deletion must remain conservative.
    pub unknown_references: bool,
}
impl From<StorageUsage> for DatabaseUsageView {
    fn from(value: StorageUsage) -> Self {
        Self {
            attachment_bytes: value.attachment_bytes,
            attachment_limit: value.attachment_limit,
            over_limit: value.over_limit,
            retained_bytes: value.retained_bytes,
            draft_bytes: value.draft_bytes,
            file_bytes: value.file_bytes,
            has_missing_contents: !value.missing.is_empty(),
            unknown_references: value.unknown_references,
        }
    }
}

/// One worker command reads all three projections in the same live generation.
#[derive(uniffi::Record)]
pub struct PolicyDetailsView {
    /// Acknowledged administrative policy.
    pub policy: PolicyView,
    /// Existing actual storage usage calculation.
    pub usage: DatabaseUsageView,
    /// Authenticated current wrapper parameters.
    pub parameters: KdfParametersView,
    /// Last successful password KDF in this worker, not total open time.
    /// Keyed unlock and an obsolete header/epoch return no measurement.
    pub password_derivation_ms: Option<u64>,
}

/// Explicit calibration preview; no settings or encrypted bytes have been applied.
#[derive(uniffi::Record)]
pub struct CalibrationView {
    /// Requested calibration budget in milliseconds.
    pub target_ms: u32,
    /// Actual selected preview parameters, without a key or salt.
    pub parameters: KdfParametersView,
    /// Actual measured duration of the selected Argon2id pass.
    pub elapsed_ms: u64,
}

#[uniffi::export]
impl DocumentSession {
    /// Read policy, usage and KDF parameters atomically inside the actual OPEN worker.
    pub fn policy_details(&self) -> Result<PolicyDetailsView, AndroidError> {
        let value: DatabasePolicyDetails = self.command(Command::PolicyDetails)?;
        Ok(PolicyDetailsView {
            policy: PolicyView {
                attachment_bytes: value.policy.attachment_bytes(),
                total_attachment_bytes: value.policy.total_attachment_bytes(),
                kdf_target_ms: value.policy.kdf_target_ms(),
            },
            usage: value.usage.into(),
            parameters: value.protection.parameters.into(),
            password_derivation_ms: value.protection.password_derivation_ms,
        })
    }

    /// Run bounded manager calibration only on this explicit action; publish nothing.
    pub fn calibrate_protection(&self, target_ms: u32) -> Result<CalibrationView, AndroidError> {
        let value: KdfCalibration = self.command(Command::CalibrateProtection { target_ms })?;
        Ok(CalibrationView {
            target_ms: value.target_ms,
            parameters: value.measurement.parameters.into(),
            elapsed_ms: value.measurement.elapsed_ms,
        })
    }
}
