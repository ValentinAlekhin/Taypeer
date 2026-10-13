//! External ciphertext copying confirms the actual bytes before output completion.
use super::*;
use sha2::{Digest as _, Sha256};
use std::io::Write;

impl ArchiveSnapshot {
    /// Copy the exact captured ciphertext generation, checking its physical length
    /// and streamed fingerprint. No object is decrypted or re-encoded. An in-place
    /// mutation of the retained inode returns `Changed`; callers must not finish
    /// their destination until this operation succeeds. Output I/O remains typed.
    pub fn copy_verified_ciphertext(&self, output: &mut impl Write) -> Result<(), Error> {
        if self.file.length()? != self.length {
            return Err(Error::Changed);
        }
        let mut verified = FingerprintOutput {
            output,
            hash: Sha256::new(),
            written: 0,
        };
        self.copy_ciphertext(&mut verified)?;
        if verified.written != self.length
            || Digest::from_bytes(verified.hash.finalize().into()) != self.fingerprint
            || self.file.length()? != self.length
        {
            return Err(Error::Changed);
        }
        Ok(())
    }
}

struct FingerprintOutput<'a, W> {
    output: &'a mut W,
    hash: Sha256,
    written: u64,
}
impl<W: Write> Write for FingerprintOutput<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self.output.write(bytes)?;
        if length > bytes.len() {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        self.written = self
            .written
            .checked_add(length as u64)
            .ok_or(std::io::ErrorKind::InvalidData)?;
        self.hash.update(&bytes[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}
