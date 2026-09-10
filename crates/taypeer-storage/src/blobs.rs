//! Immutable content backed by encrypted temporary files and an ephemeral key.

use crate::{Error, ReadKey, crypto, stream::DecryptReader};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};
use taypeer_core::BlobId;
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

struct Staging {
    header: Vec<u8>,
    key: ReadKey,
}
pub(super) struct SealedBlob {
    file: NamedTempFile,
    header: Vec<u8>,
    pub(super) length: u64,
    pub(super) digest: [u8; 32],
}

/// Immutable content aliases owned by one open session. Clones share encrypted files.
/// No plaintext, digest or staging key is printable through this type.
#[derive(Clone)]
pub struct BlobStore {
    staging: Arc<Staging>,
    pub(super) blobs: BTreeMap<BlobId, Arc<SealedBlob>>,
}

impl BlobStore {
    /// Create independent temporary encryption state, never persisted in a local filename.
    pub fn new() -> Result<Self, Error> {
        let (header, key) = crypto::staging()?;
        Ok(Self {
            staging: Arc::new(Staging { header, key }),
            blobs: BTreeMap::new(),
        })
    }
    /// Stream exactly `length` bytes into encrypted staging and reuse equal contents.
    pub fn insert(&mut self, input: impl Read, length: u64, limit: u64) -> Result<BlobId, Error> {
        let id = BlobId::new(
            crypto::random::<16>()?
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        );
        self.insert_ids(input, length, limit, std::slice::from_ref(&id), None)?;
        Ok(id)
    }
    pub(super) fn insert_ids(
        &mut self,
        input: impl Read,
        length: u64,
        limit: u64,
        ids: &[BlobId],
        expected: Option<[u8; 32]>,
    ) -> Result<(), Error> {
        if length > limit {
            return Err(Error::TooLarge);
        }
        if ids.is_empty() || ids.iter().any(|id| self.blobs.contains_key(id)) {
            return Err(Error::BlobMismatch);
        }
        let mut input = HashReader {
            input,
            hash: Sha256::new(),
        };
        let mut file = NamedTempFile::new()?;
        let header = crypto::encrypt_stream(
            &self.staging.header,
            &self.staging.key,
            &mut input,
            length,
            &mut file,
        )?;
        let digest: [u8; 32] = input.hash.finalize().into();
        if expected.is_some_and(|expected| expected != digest) {
            return Err(Error::BlobMismatch);
        }
        let mut blob = Arc::new(SealedBlob {
            file,
            header,
            length,
            digest,
        });
        for existing in self.blobs.values() {
            if existing.digest == digest
                && existing.length == length
                && self.equal(existing, &blob)?
            {
                blob = existing.clone();
                break;
            }
        }
        for id in ids {
            self.blobs.insert(id.clone(), blob.clone());
        }
        Ok(())
    }
    fn reader_for<'a>(&'a self, blob: &SealedBlob) -> Result<DecryptReader<'a, File>, Error> {
        let mut file = blob.file.reopen()?;
        file.seek(SeekFrom::Start(crypto::HEADER as u64))?;
        DecryptReader::new(file, &self.staging.key, blob.header.clone())
    }
    fn equal(&self, left: &SealedBlob, right: &SealedBlob) -> Result<bool, Error> {
        equal_readers(self.reader_for(left)?, self.reader_for(right)?)
    }
    /// Authenticated reader; consume through EOF before publishing an export.
    pub fn reader(&self, id: &BlobId) -> Result<impl Read + '_, Error> {
        self.reader_for(self.blobs.get(id).ok_or(Error::MissingBlob)?)
    }
    /// Verified original length, or absence while a late source waits for content.
    pub fn length(&self, id: &BlobId) -> Option<u64> {
        self.blobs.get(id).map(|blob| blob.length)
    }
    /// Retain an exact set of aliases; shared physical bytes live while any alias needs them.
    pub fn retained(&self, ids: &BTreeSet<BlobId>) -> Self {
        Self {
            staging: self.staging.clone(),
            blobs: self
                .blobs
                .iter()
                .filter(|(id, _)| ids.contains(*id))
                .map(|(id, blob)| (id.clone(), blob.clone()))
                .collect(),
        }
    }
    /// All known immutable aliases, without content fingerprints.
    pub fn ids(&self) -> impl Iterator<Item = &BlobId> {
        self.blobs.keys()
    }
    /// Unique physical bytes used by the given logical references.
    pub fn unique_bytes(&self, ids: &BTreeSet<BlobId>) -> u64 {
        let mut seen = BTreeSet::new();
        self.blobs
            .iter()
            .filter(|(id, blob)| ids.contains(*id) && seen.insert(Arc::as_ptr(blob)))
            .map(|(_, blob)| blob.length)
            .sum()
    }
    /// Import another encrypted store without borrowing its key after this call.
    pub fn import(&mut self, other: &Self) -> Result<(), Error> {
        for (id, blob) in &other.blobs {
            if let Some(existing) = self.blobs.get(id) {
                if existing.digest != blob.digest || existing.length != blob.length {
                    return Err(Error::BlobMismatch);
                }
                // Compare bytes as well as digests; each reader keeps its own staging key.
                if !equal_readers(self.reader(id)?, other.reader(id)?)? {
                    return Err(Error::BlobMismatch);
                }
            } else {
                self.insert_ids(
                    other.reader(id)?,
                    blob.length,
                    crypto::MAX_PAYLOAD,
                    std::slice::from_ref(id),
                    Some(blob.digest),
                )?;
            }
        }
        Ok(())
    }
}
struct HashReader<R> {
    input: R,
    hash: Sha256,
}
impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = self.input.read(bytes)?;
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
}

fn equal_readers(mut left: impl Read, mut right: impl Read) -> Result<bool, Error> {
    let mut a = Zeroizing::new(vec![0; crypto::CHUNK]);
    let mut b = Zeroizing::new(vec![0; crypto::CHUNK]);
    loop {
        let n = left.read(&mut a)?;
        right.read_exact(&mut b[..n])?;
        if a[..n] != b[..n] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(right.read(&mut b)? == 0);
        }
    }
}
