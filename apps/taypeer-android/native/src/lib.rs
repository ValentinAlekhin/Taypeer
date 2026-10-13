//! Typed Android application boundary. No document tree, JSON dispatch or read keys cross it.
mod biometry;
mod credentials;
mod descriptors;
mod document;
mod palette;
mod preferences;
#[cfg(all(unix, feature = "android-test-support"))]
pub use biometry::verify_public_biometric_pipe;
pub use biometry::{
    BiometricAvailability, BiometricBindingView, BiometricEnrollment, BiometricInputSpec,
    BiometricOpening, BiometricPromptCall, BiometricPromptOutcome, BiometricPurpose,
    BiometryFailure, BiometryState, BiometryStatusView, OsBiometricKeyLease, OsBiometryPort,
    PrivateBiometricInputs,
};
pub use credentials::{CredentialPort, Host};
pub use descriptors::{CiphertextArchive, CiphertextFiles};
pub use document::*;
pub use palette::{Palette, palette};
pub use preferences::DeviceSettingsView;

uniffi::setup_scaffolding!();

/// Sanitized platform/FFI failure, localized by the Android UI.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[error("{self:?}")]
pub enum AndroidError {
    /// Content-free OS/attempt refusal, separate from durable publication failure.
    Biometry {
        /// Exact local refusal, never an OS authentication assertion.
        failure: BiometryFailure,
    },
    /// Invalid generator parameters.
    InvalidOptions,
    /// The current form is incomplete or invalid and was not committed.
    InvalidInput,
    /// A submitted local device name has no non-whitespace characters.
    DeviceNameRequired,
    /// A submitted local device name exceeds 256 UTF-8 bytes.
    DeviceNameTooLong,
    /// Explicit image acquisition failed without changing the selected form.
    Icon {
        /// Content-free shared acquisition failure.
        failure: IconFailure,
    },
    /// The imported file could not be verified; no working copy was published.
    InvalidFile,
    /// The cryptographic random source is unavailable.
    RandomUnavailable,
    /// Native credentials cannot be read or durably written.
    Credentials,
    /// Runtime or profile initialization failed.
    Runtime,
    /// Another host already owns the profile.
    ProfileBusy,
    /// A public profile filesystem operation failed.
    ProfileIo,
    /// A public profile or protected registration is malformed.
    ProfileInvalid,
    /// Ciphertext could not be written; no durable success is claimed.
    StorageIo,
    /// Another live ciphertext writer still owns this working file.
    StorageBusy,
    /// A fresher ciphertext generation must be reconciled before retrying.
    StorageChanged,
    /// This external ciphertext-copy attempt was irreversibly cancelled.
    ExportCancelled,
    /// Publication may have happened, but durability is not confirmed.
    CommitUncertain,
    /// A different ciphertext file already occupies the destination.
    AlreadyExists,
    /// The vanished password attempt has no encrypted candidate; use a new explicit operation.
    RecoveryRequiresNewOperation,
    /// Content-free management workflow refusal. Protected exact receipts remain available for retry.
    Management {
        /// Localized phase/role/access reason, separate from uncertain publication.
        failure: ManagementFailure,
    },
}

/// Exact generator choices; validation belongs to the common Rust service.
#[derive(uniffi::Record)]
pub struct PasswordOptions {
    /// Number of characters.
    pub length: u16,
    /// Include uppercase ASCII letters.
    pub uppercase: bool,
    /// Include lowercase ASCII letters.
    pub lowercase: bool,
    /// Include digits.
    pub digits: bool,
    /// Include ASCII punctuation.
    pub punctuation: bool,
    /// Exclude visually similar characters.
    pub exclude_similar: bool,
    /// Exact additional exclusions.
    pub exclude: String,
}
impl From<PasswordOptions> for taypeer_services::generator::PasswordOptions {
    fn from(options: PasswordOptions) -> Self {
        Self {
            length: options.length,
            uppercase: options.uppercase,
            lowercase: options.lowercase,
            digits: options.digits,
            punctuation: options.punctuation,
            exclude_similar: options.exclude_similar,
            exclude: options.exclude,
        }
    }
}

/// Shared rating of known generated entropy; UI supplies localized labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum GeneratedQuality {
    /// Fewer than 40 bits.
    Low,
    /// At least 40 but fewer than 60 bits.
    Medium,
    /// At least 60 bits.
    High,
}
impl From<taypeer_services::generator::GeneratedQuality> for GeneratedQuality {
    fn from(quality: taypeer_services::generator::GeneratedQuality) -> Self {
        use taypeer_services::generator::GeneratedQuality as CommonQuality;
        match quality {
            CommonQuality::Low => Self::Low,
            CommonQuality::Medium => Self::Medium,
            CommonQuality::High => Self::High,
        }
    }
}

/// Generated value and entropy from the common uniform sampling method. This is
/// not a strength estimate for manually entered text. The caller clears the
/// entire result on lock and never logs it; JVM/FFI zeroization is not guaranteed.
#[derive(uniffi::Record)]
pub struct GeneratedSecretView {
    /// Explicitly generated secret for reveal, copy or insertion.
    pub value: String,
    /// Known entropy in bits, including the exact exclusions and word count.
    pub entropy_bits: f64,
    /// Common generator rating, not a Kotlin estimate.
    pub quality: GeneratedQuality,
}
impl std::fmt::Debug for GeneratedSecretView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GeneratedSecretView([REDACTED])")
    }
}
impl From<taypeer_services::generator::GeneratedSecret> for GeneratedSecretView {
    fn from(secret: taypeer_services::generator::GeneratedSecret) -> Self {
        Self {
            value: secret.expose().to_owned(),
            entropy_bits: secret.entropy_bits(),
            quality: secret.quality().into(),
        }
    }
}
fn generator_error(error: taypeer_services::generator::GeneratorError) -> AndroidError {
    use taypeer_services::generator::GeneratorError;
    match error {
        GeneratorError::Random => AndroidError::RandomUnavailable,
        GeneratorError::Length | GeneratorError::EmptyAlphabet | GeneratorError::Capacity => {
            AndroidError::InvalidOptions
        }
    }
}

/// Generate a password and known entropy on an explicit foreground action.
/// Validation and unbiased sampling belong to the common Rust service.
#[uniffi::export]
pub fn generate_password_view(
    options: PasswordOptions,
) -> Result<GeneratedSecretView, AndroidError> {
    taypeer_services::generator::password(&options.into())
        .map(Into::into)
        .map_err(generator_error)
}

/// Generate only on an explicit foreground action. The caller must clear the
/// returned JVM string on lock; FFI/JVM physical zeroization is not guaranteed.
#[uniffi::export]
pub fn generate_password(options: PasswordOptions) -> Result<String, AndroidError> {
    generate_password_view(options).map(|secret| secret.value)
}

/// Version of the application-owned typed bridge, checked independently of the file format.
#[uniffi::export]
pub fn bridge_version() -> u32 {
    3
}

/// Validate a transferred encrypted file before publishing it in the private catalog.
/// This does not acquire credentials, decrypt content or modify the source.
#[uniffi::export]
pub fn inspect_import(path: String) -> Result<String, AndroidError> {
    let (database, _) =
        taypeer_runtime::RuntimeHost::inspect_compatibility(std::path::Path::new(&path))
            .map_err(|_| AndroidError::InvalidFile)?;
    Ok(database.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn digits(length: u16) -> PasswordOptions {
        PasswordOptions {
            length,
            uppercase: false,
            lowercase: false,
            digits: true,
            punctuation: false,
            exclude_similar: false,
            exclude: "012345678".into(),
        }
    }
    #[test]
    fn generator_uses_common_rules_and_preserves_exact_exclusions() {
        assert_eq!(generate_password(digits(32)).unwrap(), "9".repeat(32));
        assert!(matches!(
            generate_password(digits(0)),
            Err(AndroidError::InvalidOptions)
        ));
        let mut empty = digits(32);
        empty.exclude.push('9');
        assert!(matches!(
            generate_password(empty),
            Err(AndroidError::InvalidOptions)
        ));
    }
    #[test]
    fn generated_password_view_preserves_the_actual_sampling_entropy() {
        let single = generate_password_view(digits(32)).unwrap();
        assert_eq!(single.value, "9".repeat(32));
        assert_eq!(single.entropy_bits, 0.0);
        assert_eq!(single.quality, GeneratedQuality::Low);
        let mut binary = digits(32);
        binary.exclude = "23456789".into();
        let result = generate_password_view(binary).unwrap();
        assert_eq!(result.value.len(), 32);
        assert!(result.value.bytes().all(|byte| matches!(byte, b'0' | b'1')));
        assert_eq!(result.entropy_bits, 32.0);
        assert_eq!(result.quality, GeneratedQuality::Low);
        assert_eq!(format!("{result:?}"), "GeneratedSecretView([REDACTED])");
        assert!(matches!(
            generate_password_view(digits(0)),
            Err(AndroidError::InvalidOptions)
        ));
    }
    #[test]
    fn generated_passphrase_entropy_comes_from_the_dictionary_not_its_separator() {
        let plain = generate_passphrase_view(6, " ".into()).unwrap();
        let decorated = generate_passphrase_view(6, "PUBLIC☂".into()).unwrap();
        assert_eq!(plain.value.split(' ').count(), 6);
        assert_eq!(decorated.value.split("PUBLIC☂").count(), 6);
        assert!((plain.entropy_bits - 77.54887502163469).abs() < 0.0001);
        assert_eq!(plain.entropy_bits, decorated.entropy_bits);
        assert_eq!(plain.quality, GeneratedQuality::High);
        assert_eq!(plain.quality, decorated.quality);
        assert!(matches!(
            generate_passphrase_view(2, " ".into()),
            Err(AndroidError::InvalidOptions)
        ));
    }
    #[test]
    fn generated_quality_is_projected_without_reclassification() {
        for (length, quality) in [
            (39, GeneratedQuality::Low),
            (40, GeneratedQuality::Medium),
            (59, GeneratedQuality::Medium),
            (60, GeneratedQuality::High),
        ] {
            let mut options = digits(length);
            options.exclude = "23456789".into();
            let result = generate_password_view(options).unwrap();
            assert_eq!(result.entropy_bits, f64::from(length));
            assert_eq!(result.quality, quality);
        }
    }
    #[test]
    fn invalid_import_never_becomes_a_database() {
        assert!(matches!(
            inspect_import("/nonexistent/PUBLIC.taypeer".into()),
            Err(AndroidError::InvalidFile)
        ));
    }
}

/// Generate a passphrase with the common bundled EFF wordlist and exact separator.
#[uniffi::export]
pub fn generate_passphrase(words: u8, separator: String) -> Result<String, AndroidError> {
    generate_passphrase_view(words, separator).map(|secret| secret.value)
}

/// Generate a passphrase and known EFF dictionary entropy. Separators add no
/// entropy and remain exact; no network access or manual-password score is used.
#[uniffi::export]
pub fn generate_passphrase_view(
    words: u8,
    separator: String,
) -> Result<GeneratedSecretView, AndroidError> {
    taypeer_services::generator::passphrase(words, &separator)
        .map(Into::into)
        .map_err(generator_error)
}
