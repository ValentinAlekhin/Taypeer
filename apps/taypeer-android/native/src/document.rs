//! Typed Android document commands; private framed IPC is hidden from application consumers.
mod ciphertext;
mod editor;
mod network;
mod selected;
mod session;
pub use ciphertext::{CipherCommit, CipherSeed, CipherSnapshot, CipherWriter, DocumentPersistence};
pub use editor::*;
pub use network::*;
pub use selected::*;
pub use session::*;

use crate::AndroidError;
pub(crate) const MAX_METADATA: usize = 1024 * 1024;
pub(crate) fn failure(_: impl std::fmt::Debug) -> AndroidError {
    AndroidError::Runtime
}
pub(crate) fn runtime(error: taypeer_runtime::RuntimeError) -> AndroidError {
    match error {
        taypeer_runtime::RuntimeError::Service(taypeer_services::ServiceError::Storage(error)) => {
            match error {
                taypeer_storage::Error::Io => AndroidError::StorageIo,
                taypeer_storage::Error::Changed => AndroidError::StorageChanged,
                taypeer_storage::Error::CommitUncertain => AndroidError::CommitUncertain,
                taypeer_storage::Error::AlreadyExists => AndroidError::AlreadyExists,
                _ => AndroidError::InvalidFile,
            }
        }
        _ => AndroidError::Runtime,
    }
}
pub(crate) fn digest(value: &str) -> Result<taypeer_trust::Digest, AndroidError> {
    value.parse().map_err(|_| AndroidError::InvalidFile)
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, AndroidError> {
    if bytes.len() > MAX_METADATA {
        return Err(AndroidError::InvalidFile);
    }
    serde_json::from_slice(bytes).map_err(|_| AndroidError::InvalidFile)
}
pub(crate) fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>, AndroidError> {
    let bytes = serde_json::to_vec(value).map_err(failure)?;
    if bytes.len() > MAX_METADATA {
        return Err(AndroidError::InvalidFile);
    }
    Ok(bytes)
}
