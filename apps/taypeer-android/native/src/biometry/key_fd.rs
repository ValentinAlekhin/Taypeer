//! A held private PFD is duplicated directly; isolated UIDs never reopen /proc paths.
use crate::AndroidError;
use filedescriptor::FileDescriptor;
use rustix::fs::{FileType, OFlags, fcntl_getfl, fstat};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};

const PUBLIC_PATTERN: [u8; 32] = [0x50; 32];

// This adapter borrows only a number for the duration of dup; it neither claims
// ownership nor creates a BorrowedFd. The Binder caller retains its original PFD
// until import returns. Only the new FileDescriptor owns and closes the duplicate.
struct HeldKeyFd(RawFd);
impl AsRawFd for HeldKeyFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

struct ImportedKeyInput(FileDescriptor);
impl Read for ImportedKeyInput {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
impl ImportedKeyInput {
    fn duplicate(fd: RawFd) -> Result<Self, AndroidError> {
        if fd < 0 {
            return Err(AndroidError::InvalidFile);
        }
        let owned = FileDescriptor::dup(&HeldKeyFd(fd)).map_err(|_| AndroidError::Runtime)?;
        let stat = fstat(&owned).map_err(|_| AndroidError::Runtime)?;
        let flags = fcntl_getfl(&owned).map_err(|_| AndroidError::Runtime)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Fifo
            || flags & OFlags::ACCMODE != OFlags::RDONLY
            || flags.contains(OFlags::NONBLOCK)
        {
            return Err(AndroidError::InvalidFile);
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if flags.contains(OFlags::PATH) {
            return Err(AndroidError::InvalidFile);
        }
        Ok(Self(owned))
    }

    fn verify_public(self) -> Result<bool, AndroidError> {
        let key =
            taypeer_runtime::platform_worker::read_private_key_input(self.0).map_err(|error| {
                match error {
                    taypeer_runtime::RuntimeError::Protocol => AndroidError::InvalidFile,
                    error => crate::document::runtime(error),
                }
            })?;
        Ok(key.secret_bytes() == &PUBLIC_PATTERN)
    }
}

pub(super) fn verify_public(fd: RawFd) -> Result<bool, AndroidError> {
    ImportedKeyInput::duplicate(fd)?.verify_public()
}

/// Import a held private PFD directly into the isolated Rust input owner. Only
/// the new duplicate is owned; this never reads or closes the caller's original.
pub(super) fn import_private(fd: RawFd) -> Result<Box<dyn Read + Send>, AndroidError> {
    Ok(Box::new(ImportedKeyInput::duplicate(fd)?))
}

#[cfg(test)]
mod tests;
