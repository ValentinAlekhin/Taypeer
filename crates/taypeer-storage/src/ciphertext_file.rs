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
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::read_at(self, bytes, offset)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            // ReOpenFile creates an independent cursor; try_clone does not.
            reopen(self, false)?.seek_read(bytes, offset)
        }
    }
    fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::write_at(self, bytes, offset)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            reopen(self, true)?.seek_write(bytes, offset)
        }
    }
}

#[cfg(windows)]
fn reopen(file: &File, write: bool) -> io::Result<File> {
    use std::os::windows::io::AsHandle;
    use winx::file::{AccessMode, Flags, ShareMode, reopen_file};
    let access = if write {
        AccessMode::GENERIC_WRITE
    } else {
        AccessMode::GENERIC_READ
    };
    reopen_file(
        file.as_handle(),
        access,
        ShareMode::all(),
        Flags::FILE_FLAG_RANDOM_ACCESS,
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positioned_io_preserves_external_cursor_and_parallel_reader_offsets() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"PUBLIC positioned ciphertext").unwrap();
        file.seek(SeekFrom::Start(3)).unwrap();
        let external = file.try_clone().unwrap();
        let source = CiphertextFile::from(file);
        let mut first = source.clone();
        let mut second = source.clone();
        first.seek(SeekFrom::Start(7)).unwrap();
        second.seek(SeekFrom::Start(18)).unwrap();
        let a = std::thread::spawn(move || {
            let mut bytes = [0; 10];
            first.read_exact(&mut bytes).unwrap();
            bytes
        });
        let b = std::thread::spawn(move || {
            let mut bytes = [0; 10];
            second.read_exact(&mut bytes).unwrap();
            bytes
        });
        assert_eq!(&a.join().unwrap(), b"positioned");
        assert_eq!(&b.join().unwrap(), b"ciphertext");
        let mut write = source;
        write.seek(SeekFrom::Start(7)).unwrap();
        write.write_all(b"POSITIONED").unwrap();
        assert_eq!((&external).stream_position().unwrap(), 3);
        let mut check = CiphertextFile::from(external);
        let mut all = String::new();
        check.read_to_string(&mut all).unwrap();
        assert_eq!(all, "PUBLIC POSITIONED ciphertext");
    }
}
