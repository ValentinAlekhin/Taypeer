use super::*;
use crate::{encrypted_object::copy_exact, file};
use std::io::Write;
use tempfile::NamedTempFile;

impl ArchiveCandidate {
    fn prepare(
        &self,
        metadata: ArchiveMetadata,
        directory: &Path,
    ) -> Result<(NamedTempFile, ArchiveSnapshot), Error> {
        let root = metadata
            .controls
            .first()
            .ok_or(Error::InvalidFile)?
            .hash()?;
        metadata.verify(root)?;
        if metadata.manifest.body.objects != self.descriptors() {
            return Err(Error::InvalidFile);
        }
        let bytes = serde_json::to_vec(&metadata).map_err(|_| Error::InvalidFile)?;
        if bytes.len() as u64 > MAX_METADATA {
            return Err(Error::TooLarge);
        }
        let length = metadata
            .manifest
            .body
            .objects
            .values()
            .try_fold(PREFIX as u64 + bytes.len() as u64, |sum, object| {
                sum.checked_add(object.length).ok_or(Error::TooLarge)
            })?;
        if length > MAX_ARCHIVE {
            return Err(Error::TooLarge);
        }
        let mut temp = NamedTempFile::new_in(directory)?;
        temp.write_all(MAGIC)?;
        temp.write_all(&FORMAT)?;
        temp.write_all(&(bytes.len() as u64).to_le_bytes())?;
        temp.write_all(&bytes)?;
        for (id, descriptor) in &metadata.manifest.body.objects {
            copy_exact(&mut self.reader(*id)?, &mut temp, descriptor.length)?;
        }
        temp.as_file().sync_all()?;
        let snapshot = ArchiveSnapshot::read(temp.reopen()?, Some(root))?;
        Ok((temp, snapshot))
    }
    /// Write an explicit portable copy without overwriting an existing destination.
    /// Copying does not transfer local credentials, anchors or drafts.
    pub fn export(&self, metadata: ArchiveMetadata, path: &Path) -> Result<(), Error> {
        let path = file::canonical_destination(path)?;
        let (temp, _) = self.prepare(metadata, file::parent(&path)?)?;
        file::persist(temp, &path, true)
    }
}

impl ArchiveStore {
    /// Atomically create the first signed generation. No incomplete file is published.
    pub fn create(
        path: &Path,
        candidate: &ArchiveCandidate,
        metadata: ArchiveMetadata,
        anchor: Option<Arc<dyn AnchorStore>>,
    ) -> Result<Self, Error> {
        if metadata.manifest.body.generation != 0 || candidate.base.is_some() {
            return Err(Error::InvalidFile);
        }
        let path = file::canonical_destination(path)?;
        let lock = file::lock(&path)?;
        if path.try_exists()? {
            return Err(Error::AlreadyExists);
        }
        let (temp, snapshot) = candidate.prepare(metadata, file::parent(&path)?)?;
        if let Some(store) = &anchor {
            if store.load()?.is_some_and(|a| a.accepted.is_some()) {
                return Err(Error::Changed);
            }
            store.save(&Anchor {
                accepted: None,
                prepared: Some(snapshot.fingerprint),
            })?;
        }
        file::persist(temp, &path, true)?;
        if let Some(store) = &anchor {
            store
                .save(&Anchor {
                    accepted: Some(snapshot.fingerprint),
                    prepared: None,
                })
                .map_err(|_| Error::CommitUncertain)?;
        }
        Ok(Self {
            path,
            _lock: lock,
            snapshot,
            anchor,
            uncertain: false,
            last_commit: None,
        })
    }
    /// Commit against an exact prior fingerprint. All callers, including receive,
    /// share this writer. A failure after replacement requires reopening the store.
    pub fn commit(
        &mut self,
        expected: Digest,
        candidate: &ArchiveCandidate,
        metadata: ArchiveMetadata,
    ) -> Result<(), Error> {
        if self.confirm_commit(expected, &metadata)? {
            return Ok(());
        }
        if self.uncertain {
            return Err(Error::CommitUncertain);
        }
        if expected != self.snapshot.fingerprint
            || candidate.base.as_ref().map(|s| s.fingerprint) != Some(expected)
            || self
                .snapshot
                .metadata
                .manifest
                .body
                .generation
                .checked_add(1)
                != Some(metadata.manifest.body.generation)
        {
            return Err(Error::Changed);
        }
        let next_chain = metadata.verify(self.snapshot.chain.root()?)?;
        if self.snapshot.chain.reconcile(&next_chain)? != next_chain {
            return Err(Error::Changed);
        }
        self.check_unchanged()?;
        let (temp, snapshot) = candidate.prepare(metadata, file::parent(&self.path)?)?;
        if let Some(anchor) = &self.anchor {
            anchor.save(&Anchor {
                accepted: Some(expected),
                prepared: Some(snapshot.fingerprint),
            })?;
        }
        self.check_unchanged()?;
        let retained = snapshot
            .clone()
            .with_temporary_storage(self.snapshot.temporary_storage());
        if let Err(error) = file::persist(temp, &self.path, false) {
            if error == Error::CommitUncertain {
                self.uncertain = true;
                self.last_commit = Some((expected, retained));
            }
            return Err(error);
        }
        self.snapshot = snapshot.with_temporary_storage(self.snapshot.temporary_storage());
        self.last_commit = Some((expected, retained));
        let finish = (|| {
            if let Some(anchor) = &self.anchor {
                anchor.save(&Anchor {
                    accepted: Some(self.snapshot.fingerprint),
                    prepared: None,
                })?;
            }
            Ok::<_, Error>(())
        })();
        if finish.is_err() {
            self.uncertain = true;
            return Err(Error::CommitUncertain);
        }
        Ok(())
    }

    /// Confirm only this writer's retained exact encrypted commit, including a lost reply.
    /// Matching metadata still requires fresh file/directory sync and protected-anchor ACK.
    /// `false` means there is no matching retained candidate; it never adopts an arbitrary file.
    /// A different visible generation/anchor refuses, leaving uncertainty unchanged.
    pub fn confirm_commit(
        &mut self,
        expected: Digest,
        metadata: &ArchiveMetadata,
    ) -> Result<bool, Error> {
        let Some((previous, candidate)) = &self.last_commit else {
            return Ok(false);
        };
        if *previous != expected || candidate.metadata() != metadata {
            return Ok(false);
        }
        let fingerprint = candidate.fingerprint();
        let mut input = File::open(&self.path)?;
        if Digest::from_bytes(file::fingerprint(&mut input)?) != fingerprint {
            return Err(Error::Changed);
        }
        if let Some(store) = &self.anchor {
            let anchor = store.load()?.ok_or(Error::Changed)?;
            let prepared =
                anchor.accepted == Some(expected) && anchor.prepared == Some(fingerprint);
            let acknowledged = anchor.accepted == Some(fingerprint) && anchor.prepared.is_none();
            if !prepared && !acknowledged {
                return Err(Error::Changed);
            }
        }
        input.sync_all().map_err(|_| Error::CommitUncertain)?;
        File::open(file::parent(&self.path)?)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| Error::CommitUncertain)?;
        if Digest::from_bytes(file::fingerprint(&mut input)?) != fingerprint
            || Digest::from_bytes(file::fingerprint(&mut File::open(&self.path)?)?) != fingerprint
        {
            return Err(Error::Changed);
        }
        if let Some(store) = &self.anchor {
            store
                .save(&Anchor {
                    accepted: Some(fingerprint),
                    prepared: None,
                })
                .map_err(|_| Error::CommitUncertain)?;
        }
        if Digest::from_bytes(file::fingerprint(&mut File::open(&self.path)?)?) != fingerprint {
            return Err(Error::Changed);
        }
        self.snapshot = candidate.clone();
        self.uncertain = false;
        Ok(true)
    }
    fn check_unchanged(&self) -> Result<(), Error> {
        if Digest::from_bytes(file::fingerprint(&mut File::open(&self.path)?)?)
            != self.snapshot.fingerprint
        {
            return Err(Error::Changed);
        }
        Ok(())
    }
}
