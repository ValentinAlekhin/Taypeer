//! Kotlin owns ParcelFileDescriptors; Rust owns typed, bounded ciphertext capabilities.
use crate::AndroidError;
use std::{io, sync::Arc};
use taypeer_storage::{
    ArchiveSnapshot, CiphertextFile, CiphertextIo, TemporaryFileProvider, TemporaryStorage,
};

pub(crate) const TRANSFER_CHUNK: usize = 64 * 1024;

/// Private worker file table. Handles are opaque leases, not OS descriptor numbers.
/// Methods address only already-transferred ciphertext files, never arbitrary paths.
#[uniffi::export(callback_interface)]
pub trait CiphertextFiles: Send + Sync {
    /// Allocate an empty private read/write file; do not fall back to a global path.
    fn allocate(&self) -> Result<u64, AndroidError>;
    /// Obtain the encoded byte length.
    fn length(&self, file: u64) -> Result<u64, AndroidError>;
    /// Read at most `count` ciphertext bytes at an absolute offset (maximum 64 KiB).
    fn read(&self, file: u64, offset: u64, count: u32) -> Result<Vec<u8>, AndroidError>;
    /// Write ciphertext at an absolute offset (maximum 64 KiB), returning bytes written.
    fn write(&self, file: u64, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError>;
    /// Close and forget the descriptor lease. No durable commit is implied by closing.
    fn release(&self, file: u64);
}
struct Files(Arc<dyn CiphertextFiles>);
pub(crate) fn temporary(files: Arc<dyn CiphertextFiles>) -> TemporaryStorage {
    TemporaryStorage::new(Arc::new(Files(files)))
}
pub(crate) fn source(files: Arc<dyn CiphertextFiles>, id: u64) -> CiphertextFile {
    CiphertextFile::new(Arc::new(Lease { files, id }))
}
impl TemporaryFileProvider for Files {
    fn create(&self) -> io::Result<CiphertextFile> {
        let id = self.0.allocate().map_err(file_error)?;
        Ok(CiphertextFile::new(Arc::new(Lease {
            files: Arc::clone(&self.0),
            id,
        })))
    }
}
struct Lease {
    files: Arc<dyn CiphertextFiles>,
    id: u64,
}
fn file_error(_: AndroidError) -> io::Error {
    io::ErrorKind::Other.into()
}
impl CiphertextIo for Lease {
    fn length(&self) -> io::Result<u64> {
        self.files.length(self.id).map_err(file_error)
    }
    fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        let count = bytes.len().min(TRANSFER_CHUNK);
        if count == 0 {
            return Ok(0);
        }
        let received = self
            .files
            .read(self.id, offset, count as u32)
            .map_err(file_error)?;
        if received.len() > count {
            return Err(io::ErrorKind::InvalidData.into());
        }
        bytes[..received.len()].copy_from_slice(&received);
        Ok(received.len())
    }
    fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
        let count = bytes.len().min(TRANSFER_CHUNK);
        if count == 0 {
            return Ok(0);
        }
        let written = self
            .files
            .write(self.id, offset, bytes[..count].to_vec())
            .map_err(file_error)? as usize;
        if written > count {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(written)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.files.release(self.id);
    }
}

/// Verified ciphertext view with no password, plaintext document, or write authority.
/// The sending coordinator must keep the underlying generation immutable.
#[derive(uniffi::Object)]
pub struct CiphertextArchive {
    snapshot: ArchiveSnapshot,
}
#[uniffi::export]
impl CiphertextArchive {
    /// Take ownership of an incoming file lease, including on validation failure.
    #[uniffi::constructor]
    pub fn new(file: u64, files: Box<dyn CiphertextFiles>) -> Result<Arc<Self>, AndroidError> {
        let files: Arc<dyn CiphertextFiles> = Arc::from(files);
        let source = CiphertextFile::new(Arc::new(Lease {
            files: Arc::clone(&files),
            id: file,
        }));
        let temporary = TemporaryStorage::new(Arc::new(Files(files)));
        let snapshot = ArchiveSnapshot::from_source(source, None, temporary)
            .map_err(|_| AndroidError::InvalidFile)?;
        Ok(Arc::new(Self { snapshot }))
    }
    /// Public logical identity from the verified signed control chain.
    pub fn database_id(&self) -> String {
        self.snapshot.chain().head().database.to_string()
    }
    /// Verify standalone ciphertext objects using the worker's explicit allocator.
    /// No key is acquired and no decrypted bytes are returned.
    pub fn verify_objects(&self) -> Result<u64, AndroidError> {
        let objects = &self.snapshot.metadata().manifest.body.objects;
        for id in objects.keys() {
            self.snapshot
                .object(*id)
                .map_err(|_| AndroidError::InvalidFile)?;
        }
        Ok(objects.len() as u64)
    }
}
