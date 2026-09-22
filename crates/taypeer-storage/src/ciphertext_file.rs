//! Positioned ciphertext I/O, including descriptors owned by another language runtime.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    sync::Arc,
};

/// File capability used only for ciphertext. Positional operations must not mutate
/// a shared cursor; the owner keeps the underlying descriptor alive until drop.
/// Successful writes must be immediately visible to subsequent reads through this
/// capability (no deferred user-space buffer); durability is a separate host operation.
pub trait CiphertextIo: Send + Sync {
    /// Current encoded length.
    fn length(&self) -> io::Result<u64>;
    /// Read at most `bytes.len()` bytes at an absolute offset, returning zero only at EOF.
    fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize>;
    /// Write a prefix at the absolute offset. Immutable sources reject this operation.
    fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize>;
}
impl CiphertextIo for File {
    fn length(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }
    fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        std::os::unix::fs::FileExt::read_at(self, bytes, offset)
    }
    fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
        std::os::unix::fs::FileExt::write_at(self, bytes, offset)
    }
}

/// Owned ciphertext capability with an independent cursor per clone. It does not
/// promise durable publication; the coordinator separately performs sync and commit.
#[derive(Clone)]
pub struct CiphertextFile {
    source: Arc<dyn CiphertextIo>,
    position: u64,
}
impl CiphertextFile {
    /// Retain a platform descriptor owner with positional operations.
    pub fn new(source: Arc<dyn CiphertextIo>) -> Self {
        Self {
            source,
            position: 0,
        }
    }
    /// Current encoded length, without seeking the shared underlying descriptor.
    pub fn length(&self) -> io::Result<u64> {
        self.source.length()
    }
    pub(crate) fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        let count = self.source.read_at(bytes, offset)?;
        if count > bytes.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(count)
    }
}
impl From<File> for CiphertextFile {
    fn from(file: File) -> Self {
        Self::new(Arc::new(file))
    }
}
impl From<Arc<File>> for CiphertextFile {
    fn from(file: Arc<File>) -> Self {
        Self::new(file)
    }
}
impl Read for CiphertextFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.read_at(bytes, self.position)?;
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or(io::ErrorKind::InvalidInput)?;
        Ok(count)
    }
}
impl Write for CiphertextFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.source.write_at(bytes, self.position)?;
        if count > bytes.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or(io::ErrorKind::InvalidInput)?;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for CiphertextFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length()?.checked_add_signed(delta),
        }
        .ok_or(io::ErrorKind::InvalidInput)?;
        self.position = next;
        Ok(next)
    }
}
