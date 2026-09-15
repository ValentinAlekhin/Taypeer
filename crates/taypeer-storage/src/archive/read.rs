use super::*;
use crate::{
    encrypted_object::{hash_exact, read_envelope},
    file,
};
use std::io::{Read, Seek, SeekFrom};

impl ArchiveSnapshot {
    /// Read a portable file without registering credentials or obtaining write access.
    /// When supplied, the root must come from local registration or an invitation.
    pub fn open(path: &Path, pinned: Option<Digest>) -> Result<Self, Error> {
        Self::read(File::open(path)?, pinned)
    }
    pub(super) fn read(mut file: File, pinned: Option<Digest>) -> Result<Self, Error> {
        let length = file.metadata()?.len();
        if length < PREFIX as u64 || length > MAX_ARCHIVE {
            return Err(Error::TooLarge);
        }
        let mut prefix = [0; PREFIX];
        file.read_exact(&mut prefix)?;
        if &prefix[..8] != MAGIC {
            return Err(Error::InvalidFile);
        }
        if prefix[8..12] != [1, 0, 4, 0] {
            return Err(Error::UnsupportedVersion);
        }
        let meta_length =
            u64::from_le_bytes(prefix[12..20].try_into().map_err(|_| Error::InvalidFile)?);
        if meta_length > MAX_METADATA || meta_length + PREFIX as u64 > length {
            return Err(Error::TooLarge);
        }
        let mut bytes = vec![0; meta_length as usize];
        file.read_exact(&mut bytes)?;
        let metadata: ArchiveMetadata =
            serde_json::from_slice(&bytes).map_err(|_| Error::InvalidFile)?;
        let root = metadata
            .controls
            .first()
            .ok_or(Error::InvalidFile)?
            .hash()?;
        let chain = metadata.verify(pinned.unwrap_or(root))?;
        let mut offset = PREFIX as u64 + meta_length;
        let mut offsets = BTreeMap::new();
        for (id, object) in &metadata.manifest.body.objects {
            let end = offset.checked_add(object.length).ok_or(Error::TooLarge)?;
            if end > length {
                return Err(Error::InvalidFile);
            }
            file.seek(SeekFrom::Start(offset))?;
            if hash_exact(&mut (&mut file).take(object.length), object.length)? != *id {
                return Err(Error::Authentication);
            }
            file.seek(SeekFrom::Start(offset))?;
            let mut bounded = (&mut file).take(object.length);
            let (envelope, _) = read_envelope(&mut bounded, object, &chain)?;
            if (*id == metadata.manifest.body.checkpoint || *id == metadata.manifest.body.baseline)
                && envelope.control != metadata.manifest.body.control
            {
                return Err(Error::InvalidFile);
            }
            if hash_exact(&mut bounded, envelope.length)? != envelope.ciphertext {
                return Err(Error::Authentication);
            }
            offsets.insert(*id, offset);
            offset = end;
        }
        if offset != length {
            return Err(Error::InvalidFile);
        }
        let fingerprint = Digest::from_bytes(file::fingerprint(&mut file)?);
        Ok(Self {
            file: Arc::new(file),
            metadata,
            chain,
            offsets,
            fingerprint,
            length,
        })
    }
}

impl ArchiveStore {
    /// Lock a registered file and validate its pinned authority and local marker.
    /// A missing marker is initialized only after the file has been fully verified.
    pub fn open(
        path: &Path,
        pinned: Option<Digest>,
        anchor: Option<Arc<dyn AnchorStore>>,
    ) -> Result<Self, Error> {
        let path = file::canonical_destination(path)?;
        let lock = file::lock(&path)?;
        let snapshot = ArchiveSnapshot::open(&path, pinned)?;
        if let Some(store) = &anchor {
            if let Some(marker) = store.load()?
                && marker.accepted != Some(snapshot.fingerprint)
                && marker.prepared != Some(snapshot.fingerprint)
            {
                return Err(Error::Changed);
            }
            store.save(&Anchor {
                accepted: Some(snapshot.fingerprint),
                prepared: None,
            })?;
        }
        Ok(Self {
            path,
            _lock: lock,
            snapshot,
            anchor,
            uncertain: false,
        })
    }
}
