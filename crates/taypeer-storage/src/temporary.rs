//! Explicit ownership of seekable files that contain only encrypted staging bytes.
use crate::CiphertextFile;
use std::{
    io::{self, Seek},
    path::PathBuf,
    sync::Arc,
};

/// Platform allocation port. Every call transfers a fresh, empty, private,
/// readable/writable seekable file. Never return a working database or a reused file.
/// The provider must arrange cleanup on process death, including for transferred FDs.
/// Only ciphertext may be written to the returned file by storage consumers.
pub trait TemporaryFileProvider: Send + Sync {
    /// Allocate a new file, or report failure without falling back to another location.
    fn create(&self) -> io::Result<CiphertextFile>;
}

/// Session-owned allocation policy; clones share the provider, never an output file.
#[derive(Clone)]
pub struct TemporaryStorage(Arc<dyn TemporaryFileProvider>);
impl TemporaryStorage {
    /// Use a platform provider, such as descriptors delivered over private Binder.
    pub fn new(provider: Arc<dyn TemporaryFileProvider>) -> Self {
        Self(provider)
    }
    /// Allocate anonymous files in an explicitly selected directory.
    pub fn in_directory(directory: PathBuf) -> Self {
        Self::new(Arc::new(Directory(directory)))
    }
    /// Allocate ciphertext staging. Never use this file for decrypted documents or keys.
    pub fn create(&self) -> io::Result<CiphertextFile> {
        let mut file = self.0.create()?;
        if file.length()? != 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        file.rewind()?;
        Ok(file)
    }
}
impl Default for TemporaryStorage {
    /// Desktop convenience policy. Isolated workers must supply their own provider.
    fn default() -> Self {
        Self::in_directory(std::env::temp_dir())
    }
}
struct Directory(PathBuf);
impl TemporaryFileProvider for Directory {
    fn create(&self) -> io::Result<CiphertextFile> {
        tempfile::tempfile_in(&self.0).map(CiphertextFile::from)
    }
}
