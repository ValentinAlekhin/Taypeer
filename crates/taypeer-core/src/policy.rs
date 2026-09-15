//! Shared manager-controlled limits. Acceptance comes from signed control, not this type.
use serde::{Deserialize, Serialize};

/// Validated shared attachment and KDF policy, stored inside encryption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PolicyFields", into = "PolicyFields")]
pub struct DatabasePolicy {
    attachment: u64,
    total: u64,
    target_ms: u32,
}
/// A shared policy is outside the product's bounded nonzero quota/KDF range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyError;
impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid database policy")
    }
}
impl std::error::Error for PolicyError {}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFields {
    attachment_bytes: u64,
    total_attachment_bytes: u64,
    kdf_target_ms: u32,
}
impl DatabasePolicy {
    /// Check the product's absolute upper bounds. Smaller positive quotas are allowed.
    pub fn new(
        attachment_bytes: u64,
        total_attachment_bytes: u64,
        kdf_target_ms: u32,
    ) -> Result<Self, PolicyError> {
        if !(1..=100 * 1024 * 1024).contains(&attachment_bytes)
            || !(1..=1024 * 1024 * 1024).contains(&total_attachment_bytes)
            || !(500..=5000).contains(&kdf_target_ms)
        {
            return Err(PolicyError);
        }
        Ok(Self {
            attachment: attachment_bytes,
            total: total_attachment_bytes,
            target_ms: kdf_target_ms,
        })
    }
    /// Maximum size of newly introduced attachment content.
    pub fn attachment_bytes(self) -> u64 {
        self.attachment
    }
    /// Maximum unique retained attachment bytes before accepting another addition.
    pub fn total_attachment_bytes(self) -> u64 {
        self.total
    }
    /// Calibration target on the managing device, not an artificial unlock delay.
    pub fn kdf_target_ms(self) -> u32 {
        self.target_ms
    }
}
impl Default for DatabasePolicy {
    fn default() -> Self {
        Self {
            attachment: crate::ATTACHMENT_LIMIT,
            total: crate::DATABASE_ATTACHMENT_LIMIT,
            target_ms: 1000,
        }
    }
}
impl TryFrom<PolicyFields> for DatabasePolicy {
    type Error = PolicyError;
    fn try_from(value: PolicyFields) -> Result<Self, Self::Error> {
        Self::new(
            value.attachment_bytes,
            value.total_attachment_bytes,
            value.kdf_target_ms,
        )
    }
}
impl From<DatabasePolicy> for PolicyFields {
    fn from(value: DatabasePolicy) -> Self {
        Self {
            attachment_bytes: value.attachment,
            total_attachment_bytes: value.total,
            kdf_target_ms: value.target_ms,
        }
    }
}
