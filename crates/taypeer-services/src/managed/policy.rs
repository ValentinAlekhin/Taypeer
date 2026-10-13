//! Read-only numeric policy/KDF projections and explicit nonmutating calibration.
use super::*;
use serde::{Deserialize, Serialize};
use taypeer_storage::{KdfCalibration, KdfMeasurement, KdfParameters};

/// Current password-wrapper parameters and an optional actual password KDF duration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtectionInfo {
    /// Parameters authenticated by the current managed password header.
    pub parameters: KdfParameters,
    /// Successful password derivation in this worker; keyed authentication has none.
    pub password_derivation_ms: Option<u64>,
}
/// One worker command projects policy, product usage and protection consistently.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabasePolicyDetails {
    /// Acknowledged authenticated administrative settings.
    pub policy: DatabasePolicy,
    /// Existing product/retention/draft/physical usage, without invented zero values.
    pub usage: StorageUsage,
    /// Numeric KDF information; no header or key crosses the worker boundary.
    pub protection: ProtectionInfo,
}
pub(super) struct PasswordKdfMeasurement {
    header: Vec<u8>,
    epoch: u64,
    value: KdfMeasurement,
}
impl PasswordKdfMeasurement {
    pub(super) fn new(
        object: &EncryptedObject,
        value: KdfMeasurement,
    ) -> Result<Self, ServiceError> {
        Ok(Self {
            header: object.password_header()?,
            epoch: object.envelope().epoch,
            value,
        })
    }
    fn matches(&self, header: &[u8]) -> bool {
        taypeer_storage::password_key_binding_matches(&self.header, header)
    }
}
impl Unlocked {
    pub(super) fn replace_header(&mut self, header: Vec<u8>) {
        if self
            .password_kdf
            .as_ref()
            .is_some_and(|measurement| !measurement.matches(&header))
        {
            self.password_kdf = None;
        }
        self.header = header;
    }
}
impl DatabaseService {
    /// Read one generation's actual policy, usage and password-wrapper parameters.
    pub fn database_policy_details(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<DatabasePolicyDetails>, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        let parameters = taypeer_storage::password_kdf_parameters(&open.header)?;
        let password_derivation_ms = open
            .password_kdf
            .as_ref()
            .filter(|measurement| {
                measurement.epoch == managed.snapshot.chain().head().epoch
                    && measurement.matches(&open.header)
            })
            .map(|measurement| measurement.value.elapsed_ms);
        Ok(stamped(
            session,
            DatabasePolicyDetails {
                policy: open.metadata.policy,
                usage: self.storage_usage(session)?.value,
                protection: ProtectionInfo {
                    parameters,
                    password_derivation_ms,
                },
            },
        ))
    }
    /// Explicitly preview the existing manager calibration without writing any state.
    pub fn calibrate_protection(
        &self,
        session: &SessionToken,
        target_ms: u32,
    ) -> Result<SessionValue<KdfCalibration>, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        managed.manager()?;
        let policy = managed
            .open
            .as_ref()
            .ok_or(ServiceError::Locked)?
            .metadata
            .policy;
        DatabasePolicy::new(
            policy.attachment_bytes(),
            policy.total_attachment_bytes(),
            target_ms,
        )
        .map_err(|_| ServiceError::InvalidInput)?;
        Ok(stamped(
            session,
            taypeer_storage::calibrate_password_kdf(target_ms)?,
        ))
    }
}
