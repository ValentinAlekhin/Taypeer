//! Version-one streaming encrypted files under development. See docs/storage.md.

mod archive;
mod blobs;
mod bundle;
mod cipher_persistence;
mod ciphertext_file;
mod crypto;
mod encrypted_object;
mod file;
mod local_state;
mod locking;
mod publication;
mod stream;
mod temporary;

pub use archive::{
    Anchor, AnchorStore, ArchiveCandidate, ArchiveJournal, ArchiveMetadata, ArchiveSnapshot,
    ArchiveStore, InvitationRecord, OfferedState,
};
pub use cipher_persistence::{
    ArchiveSeed, CipherPersistence, PreparedCommit, discard_local_draft, read_local_draft,
    save_local_draft,
};
pub use encrypted_object::{EncryptedObject, ObjectReader, create_epoch};

pub use blobs::BlobStore;
pub use bundle::BundleReader;
pub use ciphertext_file::{CiphertextFile, CiphertextIo};
pub use crypto::ReadKey;
pub use file::{BinaryDraft, FileStore};
pub use local_state::{LocalCredentialStore, LocalStateKey, LocalStateStore};
pub use locking::try_lock_exclusive;
pub use publication::{PublicationMode, publish_file};
pub use temporary::{TemporaryFileProvider, TemporaryStorage};

/// Categorized failures without paths, passwords or parser diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[error("{self:?}")]
pub enum Error {
    /// Empty master passwords are forbidden; whitespace remains significant.
    EmptyPassword,
    /// Password authentication failed or encrypted content was altered.
    Authentication,
    /// Header or payload structure is invalid.
    InvalidFile,
    /// A different container or schema version is required.
    UnsupportedVersion,
    /// The bounded reader or writer rejected an oversized file.
    TooLarge,
    /// Required immutable content is not locally available.
    MissingBlob,
    /// An immutable binary identity was reused for different content.
    BlobMismatch,
    /// The operating system could not complete an I/O operation.
    Io,
    /// The file is already open by another writer.
    Busy,
    /// Creating a destination would replace an existing file.
    AlreadyExists,
    /// Another writer changed the file; reopen before further edits.
    Changed,
    /// Replacement occurred but directory synchronization failed; reopen to reconcile.
    CommitUncertain,
    /// The operating system could not supply random bytes.
    Random,
    /// Signed authority or provenance validation failed.
    Trust(taypeer_trust::Error),
}
impl From<taypeer_trust::Error> for Error {
    fn from(error: taypeer_trust::Error) -> Self {
        Self::Trust(error)
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        // Streaming adapters preserve our sanitized authentication/format categories.
        // Ordinary OS diagnostics, including paths, never escape this boundary.
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
            .copied()
            .unwrap_or(Self::Io)
    }
}

/// Maximum in-memory document or draft size; streaming payloads have a separate bound.
pub const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod binary_tests;

#[cfg(test)]
mod descriptor_tests;
