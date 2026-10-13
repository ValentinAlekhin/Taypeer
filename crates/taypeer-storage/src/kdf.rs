//! Content-free measurements of the existing password wrapper KDF.
use crate::{Error, crypto};
use serde::{Deserialize, Serialize};

/// Parameters projected from a validated version-one password header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParameters {
    /// Argon2id memory cost in KiB, fixed by the current format.
    pub memory_kib: u32,
    /// Argon2id lane count, fixed by the current format.
    pub parallelism: u32,
    /// Actual number of passes encoded by the password wrapper.
    pub iterations: u32,
}
/// A successful Argon2id derivation; this is not the duration of opening a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfMeasurement {
    /// Actual parameters used for this derivation.
    pub parameters: KdfParameters,
    /// Monotonic elapsed milliseconds, excluding file/decryption/author work.
    pub elapsed_ms: u64,
}
/// Measured preview for an explicit calibration request, without publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfCalibration {
    /// Requested calibration budget; not an artificial delay or guarantee.
    pub target_ms: u32,
    /// Measured selected pass, not an applied epoch.
    pub measurement: KdfMeasurement,
}

/// Inspect only an already authenticated/validated password header. No key is returned.
pub fn password_kdf_parameters(header: &[u8]) -> Result<KdfParameters, Error> {
    crypto::kdf_parameters(header)
}
/// Compare the existing password-wrapper binding, excluding per-stream nonce and length.
/// Callers obtain both headers through validated encrypted-object reads.
pub fn password_key_binding_matches(left: &[u8], right: &[u8]) -> bool {
    crypto::same_key_binding(left, right)
}
/// Run the same bounded calibration used by epoch creation with a non-secret trial input.
/// Derived allocations remain local and are zeroized on drop. This writes nothing.
pub fn calibrate_password_kdf(target_ms: u32) -> Result<KdfCalibration, Error> {
    crypto::validate_kdf_target(target_ms)?;
    let salt = crypto::random::<16>()?;
    let (_key, measurement) =
        crypto::calibrated_kek(b"Taypeer public KDF calibration input", &salt, target_ms)?;
    Ok(KdfCalibration {
        target_ms,
        measurement,
    })
}

#[cfg(test)]
mod tests;
