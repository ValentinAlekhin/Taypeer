//! Version-one streaming encrypted files under development. See docs/storage.md.

mod crypto;
mod file;

pub use crypto::ReadKey;
pub use file::FileStore;

/// Categorized failures without paths, passwords or parser diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// The operating system could not complete an I/O operation.
    Io,
    /// The file is already open by another writer.
    Busy,
    /// Creating a database would replace an existing file.
    AlreadyExists,
    /// Another writer changed the file; reopen before further edits.
    Changed,
    /// Replacement occurred but directory synchronization failed; reopen to reconcile.
    CommitUncertain,
    /// The operating system could not supply random bytes.
    Random,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

/// Maximum in-memory document or draft size; streaming payloads have a separate bound.
pub const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;

#[cfg(test)]
mod tests;
