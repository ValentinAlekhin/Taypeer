//! Immutable, independently authenticated ciphertext objects with bounded readers.

use crate::{Error, ReadKey, crypto, stream::DecryptReader};
use sha2::{Digest as _, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, Write},
    sync::Arc,
};
use taypeer_trust::{AuthorKey, CipherObject, ControlChain, Digest, ObjectEnvelope, ObjectKind};
use tempfile::NamedTempFile;

const MAGIC: &[u8; 8] = b"TAYOBJ4\0";
const MAX_ENVELOPE: usize = 16 * 1024;

/// An immutable encrypted spool. Its lifetime keeps ciphertext, never a read key.
#[derive(Clone)]
pub struct EncryptedObject {
    file: Arc<NamedTempFile>,
    descriptor: CipherObject,
    envelope: ObjectEnvelope,
    offset: u64,
}
impl std::fmt::Debug for EncryptedObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedObject")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// A bounded independently positioned reader over an immutable encrypted file.
pub struct ObjectReader {
    file: Arc<File>,
    offset: u64,
    remaining: u64,
}
impl ObjectReader {
    pub(crate) fn new(file: Arc<File>, offset: u64, length: u64) -> Self {
        Self {
            file,
            offset,
            remaining: length,
        }
    }
    /// Remaining encoded bytes, useful for a bounded transport frame.
    pub fn remaining(&self) -> u64 {
        self.remaining
    }
}
impl Read for ObjectReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        use std::os::unix::fs::FileExt;
        let length = self.remaining.min(out.len() as u64) as usize;
        if length == 0 {
            return Ok(0);
        }
        let n = self.file.read_at(&mut out[..length], self.offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        self.offset += n as u64;
        self.remaining -= n as u64;
        Ok(n)
    }
}

impl EncryptedObject {
    /// Authenticate a local encrypted spool by deriving its descriptor from bounded bytes.
    /// The caller separately checks whether its role is allowed at this local boundary.
    pub fn open(path: &std::path::Path, chain: &ControlChain) -> Result<Self, Error> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        if !(12..=crypto::MAX_ENCODED_SIZE).contains(&length) {
            return Err(Error::TooLarge);
        }
        let digest = hash_exact(&mut file, length)?;
        file.rewind()?;
        let mut prefix = [0; 12];
        file.read_exact(&mut prefix)?;
        if &prefix[..8] != MAGIC {
            return Err(Error::InvalidFile);
        }
        let json_length =
            u32::from_le_bytes(prefix[8..].try_into().map_err(|_| Error::InvalidFile)?) as usize;
        if json_length > MAX_ENVELOPE {
            return Err(Error::TooLarge);
        }
        let mut json = vec![0; json_length];
        file.read_exact(&mut json)?;
        let envelope: ObjectEnvelope =
            serde_json::from_slice(&json).map_err(|_| Error::InvalidFile)?;
        file.rewind()?;
        Self::receive(
            file,
            &CipherObject {
                digest,
                length,
                kind: envelope.kind,
            },
            chain,
        )
    }
    /// Encrypt and sign a stream. The password wrapper is supplied by the current
    /// epoch; it is reused without recalibrating KDF for each object.
    pub fn seal(
        chain: &ControlChain,
        author: &AuthorKey,
        kind: ObjectKind,
        header: &[u8],
        key: &ReadKey,
        clear: impl Read,
        length: u64,
    ) -> Result<Self, Error> {
        if header.len() != crypto::HEADER {
            return Err(Error::InvalidFile);
        }
        let mut encrypted = NamedTempFile::new()?;
        crypto::encrypt_stream(header, key, clear, length, &mut encrypted)?;
        let ciphertext_length = encrypted.as_file().metadata()?.len();
        encrypted.as_file_mut().rewind()?;
        let ciphertext = hash_exact(encrypted.as_file_mut(), ciphertext_length)?;
        let envelope = ObjectEnvelope::sign(chain, author, kind, ciphertext_length, ciphertext)?;
        let prefix = serde_json::to_vec(&envelope).map_err(|_| Error::InvalidFile)?;
        if prefix.len() > MAX_ENVELOPE {
            return Err(Error::TooLarge);
        }
        let mut file = NamedTempFile::new()?;
        file.write_all(MAGIC)?;
        file.write_all(&(prefix.len() as u32).to_le_bytes())?;
        file.write_all(&prefix)?;
        encrypted.as_file_mut().rewind()?;
        std::io::copy(encrypted.as_file_mut(), &mut file)?;
        let length = file.as_file().metadata()?.len();
        file.as_file_mut().rewind()?;
        let digest = hash_exact(file.as_file_mut(), length)?;
        Ok(Self {
            file: Arc::new(file),
            descriptor: CipherObject {
                digest,
                length,
                kind,
            },
            envelope,
            offset: 12 + prefix.len() as u64,
        })
    }
    /// Stage a complete received object. Length, ciphertext and author signature
    /// are checked before a coordinator can make this object durable and ACK it.
    pub fn receive(
        mut input: impl Read,
        descriptor: &CipherObject,
        chain: &ControlChain,
    ) -> Result<Self, Error> {
        if descriptor.length > crypto::MAX_ENCODED_SIZE || descriptor.length < 12 {
            return Err(Error::TooLarge);
        }
        let mut file = NamedTempFile::new()?;
        copy_exact(&mut input, &mut file, descriptor.length)?;
        file.as_file_mut().rewind()?;
        if hash_exact(file.as_file_mut(), descriptor.length)? != descriptor.digest {
            return Err(Error::Authentication);
        }
        file.as_file_mut().rewind()?;
        let (envelope, offset) = read_envelope(file.as_file_mut(), descriptor, chain)?;
        if hash_exact(file.as_file_mut(), envelope.length)? != envelope.ciphertext {
            return Err(Error::Authentication);
        }
        Ok(Self {
            file: Arc::new(file),
            descriptor: descriptor.clone(),
            envelope,
            offset,
        })
    }
    /// Complete-object identity used for deduplication and inventory exchange.
    pub fn descriptor(&self) -> &CipherObject {
        &self.descriptor
    }
    /// Verified public provenance; applying contents still requires an unlocked service.
    pub fn envelope(&self) -> &ObjectEnvelope {
        &self.envelope
    }
    /// Independent reader for ciphertext transport or portable archive assembly.
    pub fn reader(&self) -> Result<ObjectReader, Error> {
        Ok(ObjectReader::new(
            Arc::new(self.file.reopen()?),
            0,
            self.descriptor.length,
        ))
    }
    /// Read the password wrapper from a signed encrypted stream without unlocking it.
    pub fn password_header(&self) -> Result<Vec<u8>, Error> {
        let mut reader = self.cipher_reader()?;
        let mut header = vec![0; crypto::HEADER];
        reader.read_exact(&mut header)?;
        crypto::validate(&header, self.envelope.length)?;
        Ok(header)
    }
    /// Authenticate the epoch's password wrapper. The resulting key remains in the
    /// unlocked worker and cannot authorize a device or control transition.
    pub fn unlock_key(&self, password: &[u8]) -> Result<ReadKey, Error> {
        crypto::unlock_key(&self.password_header()?, password)
    }
    /// Decrypt into a caller-owned candidate stream. The caller must not publish
    /// any output until this method succeeds, including the final authentication tag.
    pub fn decrypt(&self, key: &ReadKey, output: &mut impl Write) -> Result<(), Error> {
        let mut reader = self.cipher_reader()?;
        let mut header = vec![0; crypto::HEADER];
        reader.read_exact(&mut header)?;
        crypto::validate(&header, self.envelope.length)?;
        crypto::decrypt_stream(&header, key, &mut reader, output)
    }
    /// Decode a full document/blob bundle into encrypted staging owned by a worker.
    pub fn unlock_bundle(
        &self,
        key: &ReadKey,
    ) -> Result<(zeroize::Zeroizing<Vec<u8>>, crate::BlobStore), Error> {
        let mut reader = self.cipher_reader()?;
        let mut header = vec![0; crypto::HEADER];
        reader.read_exact(&mut header)?;
        crypto::validate(&header, self.envelope.length)?;
        crate::BlobStore::read_bundle(DecryptReader::new(reader, key, header)?)
    }
    fn cipher_reader(&self) -> Result<ObjectReader, Error> {
        Ok(ObjectReader::new(
            Arc::new(self.file.reopen()?),
            self.offset,
            self.envelope.length,
        ))
    }
}

pub(crate) fn read_envelope(
    input: &mut impl Read,
    descriptor: &CipherObject,
    chain: &ControlChain,
) -> Result<(ObjectEnvelope, u64), Error> {
    let mut prefix = [0; 12];
    input.read_exact(&mut prefix)?;
    if &prefix[..8] != MAGIC {
        return Err(Error::InvalidFile);
    }
    let length =
        u32::from_le_bytes(prefix[8..12].try_into().map_err(|_| Error::InvalidFile)?) as usize;
    if length > MAX_ENVELOPE {
        return Err(Error::TooLarge);
    }
    let mut json = vec![0; length];
    input.read_exact(&mut json)?;
    let envelope: ObjectEnvelope = serde_json::from_slice(&json).map_err(|_| Error::InvalidFile)?;
    envelope.verify(chain)?;
    let offset = 12 + length as u64;
    if envelope.length.checked_add(offset) != Some(descriptor.length)
        || envelope.kind != descriptor.kind
    {
        return Err(Error::InvalidFile);
    }
    Ok((envelope, offset))
}
pub(crate) fn hash_exact(input: &mut impl Read, length: u64) -> Result<Digest, Error> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; crypto::CHUNK];
    let mut left = length;
    while left != 0 {
        let length = left.min(buffer.len() as u64) as usize;
        input.read_exact(&mut buffer[..length])?;
        hash.update(&buffer[..length]);
        left -= length as u64;
    }
    let mut extra = [0];
    if input.read(&mut extra)? != 0 {
        return Err(Error::InvalidFile);
    }
    Ok(Digest::from_bytes(hash.finalize().into()))
}
pub(crate) fn copy_exact(
    input: &mut impl Read,
    output: &mut impl Write,
    length: u64,
) -> Result<(), Error> {
    let mut buffer = vec![0; crypto::CHUNK];
    let mut left = length;
    while left != 0 {
        let size = left.min(buffer.len() as u64) as usize;
        input.read_exact(&mut buffer[..size])?;
        output.write_all(&buffer[..size])?;
        left -= size as u64;
    }
    let mut extra = [0];
    if input.read(&mut extra)? != 0 {
        return Err(Error::InvalidFile);
    }
    Ok(())
}

/// Calibrate a fresh independent epoch key and its password wrapper.
/// Callers must durably save the old state before making a rotation visible.
pub fn create_epoch(password: &[u8], target_ms: u32) -> Result<(Vec<u8>, ReadKey), Error> {
    crypto::create_header(password, target_ms)
}
