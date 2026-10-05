//! Typed Android application boundary. No document tree, JSON dispatch or read keys cross it.
mod credentials;
mod descriptors;
mod document;
mod palette;
pub use credentials::{CredentialPort, Host};
pub use descriptors::{CiphertextArchive, CiphertextFiles};
pub use document::*;
pub use palette::{Palette, palette};

uniffi::setup_scaffolding!();

/// Sanitized platform/FFI failure, localized by the Android UI.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[error("{self:?}")]
pub enum AndroidError {
    /// Invalid generator parameters.
    InvalidOptions,
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
    /// A fresher ciphertext generation must be reconciled before retrying.
    StorageChanged,
    /// Publication may have happened, but durability is not confirmed.
    CommitUncertain,
    /// A different ciphertext file already occupies the destination.
    AlreadyExists,
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

/// Generate only on an explicit foreground action. The caller must clear the
/// returned JVM string on lock; FFI/JVM physical zeroization is not guaranteed.
#[uniffi::export]
pub fn generate_password(options: PasswordOptions) -> Result<String, AndroidError> {
    use taypeer_services::generator::{self, GeneratorError};
    generator::password(&generator::PasswordOptions {
        length: options.length,
        uppercase: options.uppercase,
        lowercase: options.lowercase,
        digits: options.digits,
        punctuation: options.punctuation,
        exclude_similar: options.exclude_similar,
        exclude: options.exclude,
    })
    .map(|secret| secret.expose().to_owned())
    .map_err(|error| match error {
        GeneratorError::Random => AndroidError::RandomUnavailable,
        GeneratorError::Length | GeneratorError::EmptyAlphabet | GeneratorError::Capacity => {
            AndroidError::InvalidOptions
        }
    })
}

/// Version of the application-owned typed bridge, checked independently of the file format.
#[uniffi::export]
pub fn bridge_version() -> u32 {
    2
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
    use taypeer_services::generator::{self, GeneratorError};
    generator::passphrase(words, &separator)
        .map(|secret| secret.expose().to_owned())
        .map_err(|error| match error {
            GeneratorError::Random => AndroidError::RandomUnavailable,
            GeneratorError::Length | GeneratorError::EmptyAlphabet | GeneratorError::Capacity => {
                AndroidError::InvalidOptions
            }
        })
}
