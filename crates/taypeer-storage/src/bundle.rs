//! An authenticated payload catalog and streaming binary sections, outside Automerge.

use crate::{BlobStore, Error, MAX_FILE_SIZE, crypto};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{Cursor, Read},
    sync::Arc,
};
use taypeer_core::BlobId;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"TAYBLOB3";
const MAX_CATALOG: u64 = 16 * 1024 * 1024;
const MAX_ALIASES: usize = 100_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    document_length: u64,
    sections: Vec<Section>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    ids: Vec<BlobId>,
    length: u64,
    digest: [u8; 32],
}

/// A bounded-memory source for one complete container candidate.
pub struct BundleReader<'a> {
    length: u64,
    inputs: VecDeque<Box<dyn Read + 'a>>,
    blobs: &'a BlobStore,
    pending: std::vec::IntoIter<BlobId>,
}
impl BundleReader<'_> {
    /// Exact number of plaintext bytes; encryption verifies this length and EOF.
    pub fn length(&self) -> u64 {
        self.length
    }
}
impl Read for BundleReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(input) = self.inputs.front_mut() {
                let n = input.read(bytes)?;
                if n != 0 {
                    return Ok(n);
                }
                self.inputs.pop_front();
            } else if let Some(id) = self.pending.next() {
                // Only the current section needs an additional open file descriptor.
                self.inputs.push_back(Box::new(
                    self.blobs.reader(&id).map_err(std::io::Error::other)?,
                ));
            } else {
                return Ok(0);
            }
        }
    }
}

impl BlobStore {
    /// Compose metadata and encrypted staging readers without materializing binary contents.
    pub fn bundle<'a>(&'a self, document: &'a [u8]) -> Result<BundleReader<'a>, Error> {
        if document.len() > MAX_FILE_SIZE || self.blobs.len() > MAX_ALIASES {
            return Err(Error::TooLarge);
        }
        let mut aliases: BTreeMap<_, Vec<BlobId>> = BTreeMap::new();
        for (id, blob) in &self.blobs {
            aliases
                .entry(Arc::as_ptr(blob))
                .or_default()
                .push(id.clone());
        }
        let mut sections: Vec<_> = aliases
            .into_values()
            .map(|ids| {
                let blob = &self.blobs[&ids[0]];
                Section {
                    ids,
                    length: blob.length,
                    digest: blob.digest,
                }
            })
            .collect();
        sections.sort_by(|a, b| a.ids[0].cmp(&b.ids[0]));
        let catalog = Catalog {
            document_length: document.len() as u64,
            sections,
        };
        let metadata =
            Zeroizing::new(serde_json::to_vec(&catalog).map_err(|_| Error::InvalidFile)?);
        if metadata.len() as u64 > MAX_CATALOG {
            return Err(Error::TooLarge);
        }
        let mut prefix = Zeroizing::new(MAGIC.to_vec());
        prefix.extend((metadata.len() as u64).to_le_bytes());
        prefix.extend_from_slice(&metadata);
        let mut length = prefix.len() as u64 + document.len() as u64;
        let mut inputs: VecDeque<Box<dyn Read + 'a>> = VecDeque::new();
        inputs.push_back(Box::new(SecretCursor {
            bytes: prefix,
            offset: 0,
        }));
        inputs.push_back(Box::new(Cursor::new(document)));
        let mut pending = Vec::new();
        for section in catalog.sections {
            length = length.checked_add(section.length).ok_or(Error::TooLarge)?;
            pending.push(section.ids[0].clone());
        }
        if length > crypto::MAX_PAYLOAD {
            return Err(Error::TooLarge);
        }
        Ok(BundleReader {
            length,
            inputs,
            blobs: self,
            pending: pending.into_iter(),
        })
    }
    pub(super) fn read_bundle(mut input: impl Read) -> Result<(Zeroizing<Vec<u8>>, Self), Error> {
        let mut prefix = [0; 16];
        input.read_exact(&mut prefix)?;
        if &prefix[..8] != MAGIC {
            return Err(Error::InvalidFile);
        }
        let length = u64::from_le_bytes(prefix[8..].try_into().map_err(|_| Error::InvalidFile)?);
        if length > MAX_CATALOG {
            return Err(Error::TooLarge);
        }
        let mut metadata = Zeroizing::new(vec![0; length as usize]);
        input.read_exact(&mut metadata)?;
        let catalog: Catalog = serde_json::from_slice(&metadata).map_err(|_| Error::InvalidFile)?;
        if catalog.document_length > MAX_FILE_SIZE as u64 || catalog.sections.len() > MAX_ALIASES {
            return Err(Error::TooLarge);
        }
        let mut ids = BTreeSet::new();
        let mut total = 16 + length + catalog.document_length;
        for section in &catalog.sections {
            if section.ids.is_empty() {
                return Err(Error::InvalidFile);
            }
            for id in &section.ids {
                if id.as_str().len() != 32
                    || !id.as_str().bytes().all(|b| b.is_ascii_hexdigit())
                    || !ids.insert(id)
                {
                    return Err(Error::InvalidFile);
                }
                if ids.len() > MAX_ALIASES {
                    return Err(Error::TooLarge);
                }
            }
            total = total.checked_add(section.length).ok_or(Error::TooLarge)?;
            if total > crypto::MAX_PAYLOAD {
                return Err(Error::TooLarge);
            }
        }
        let mut document = Zeroizing::new(vec![0; catalog.document_length as usize]);
        input.read_exact(&mut document)?;
        let mut blobs = Self::new()?;
        for section in catalog.sections {
            blobs.insert_ids(
                (&mut input).take(section.length),
                section.length,
                crypto::MAX_PAYLOAD,
                &section.ids,
                Some(section.digest),
            )?;
        }
        let mut extra = [0];
        if input.read(&mut extra)? != 0 {
            return Err(Error::InvalidFile);
        }
        Ok((document, blobs))
    }
}

struct SecretCursor {
    bytes: Zeroizing<Vec<u8>>,
    offset: usize,
}
impl Read for SecretCursor {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let size = output.len().min(self.bytes.len() - self.offset);
        output[..size].copy_from_slice(&self.bytes[self.offset..self.offset + size]);
        self.offset += size;
        Ok(size)
    }
}
