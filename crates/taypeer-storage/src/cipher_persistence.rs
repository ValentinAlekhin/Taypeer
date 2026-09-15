//! The unlocked service's encrypted I/O port, implemented locally or over private IPC.

use crate::{ArchiveJournal, ArchiveSnapshot, EncryptedObject, Error, file};
use std::{collections::BTreeSet, fs::File, path::Path};
use taypeer_trust::{ControlChain, Digest, ObjectKind, SignedControl};
use tempfile::NamedTempFile;

/// Complete encrypted initial state, prepared by the author worker before registration.
pub struct ArchiveSeed {
    /// Signed genesis authority.
    pub controls: Vec<SignedControl>,
    /// Independently authenticated encrypted objects.
    pub objects: Vec<EncryptedObject>,
    /// Initial document checkpoint.
    pub checkpoint: Digest,
    /// Initial manager-authenticated accepted history.
    pub baseline: Digest,
}
impl ArchiveSeed {
    /// Persist initial ciphertext using only the admitted transport credential.
    /// Registration/creation is successful only after the returned writer is available.
    pub fn create(
        self,
        path: &Path,
        transport: &taypeer_trust::TransportKey,
        anchor: Option<std::sync::Arc<dyn crate::AnchorStore>>,
    ) -> Result<crate::ArchiveStore, Error> {
        let root = self.controls.first().ok_or(Error::InvalidFile)?.hash()?;
        let chain = ControlChain::validate(self.controls, root)?;
        let mut candidate = crate::ArchiveCandidate::new();
        for object in self.objects {
            candidate.insert(object)?;
        }
        let body = taypeer_trust::Manifest {
            version: 1,
            database: chain.head().database.clone(),
            trust_set: chain.head().trust_set,
            control: chain.head_hash()?,
            generation: 0,
            signer: chain.admit_transport(transport.public())?,
            objects: std::collections::BTreeMap::new(),
            checkpoint: self.checkpoint,
            baseline: self.baseline,
            auxiliary: Digest::from_bytes([0; 32]),
        };
        let metadata =
            candidate.metadata(&chain, transport, body, crate::ArchiveJournal::default())?;
        crate::ArchiveStore::create(path, &candidate, metadata, anchor)
    }
}

/// Ciphertext candidate submitted by an unlocked service. No read key crosses this port.
pub struct PreparedCommit {
    /// Exact prior file generation.
    pub expected: Digest,
    /// Exact latest-known authority checked by the author.
    pub control: Digest,
    /// Checked administrative successor or unchanged signed history.
    pub controls: Vec<SignedControl>,
    /// Independently authenticated encrypted objects.
    pub objects: Vec<EncryptedObject>,
    /// Explicit releases after complete unlocked retention analysis.
    pub remove: BTreeSet<Digest>,
    /// New active encrypted checkpoint.
    pub checkpoint: Digest,
    /// Manager-authenticated baseline for the checkpoint's control.
    pub baseline: Digest,
    /// Portable operation progress, pending inventories and retained roots.
    pub journal: ArchiveJournal,
}

/// Narrow persistence contract used by an unlocked database service. Implementations
/// provide one registered working copy and report successful commits only after sync.
pub trait CipherPersistence: Send {
    /// Fresh immutable ciphertext generation; no filesystem writer is handed to the worker.
    fn snapshot(&self) -> Result<ArchiveSnapshot, Error>;
    /// Reject stale control/generation; atomically commit all candidate objects and metadata.
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, Error>;
    /// Opaque local editor binding, excluded from all portable checkpoints.
    fn working_copy(&self) -> Digest;
    /// Store a self-contained encrypted editor outside the portable file.
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), Error>;
    /// Read an optional local editor against the verified historical chain.
    fn load_draft(&self, chain: &ControlChain) -> Result<Option<EncryptedObject>, Error>;
    /// Explicitly discard only this working copy's editor.
    fn discard_draft(&self) -> Result<(), Error>;
    /// Keep a separate source before an administrative rotation/recovery.
    fn preserve_before(&self, operation: Digest) -> Result<(), Error>;
    /// Physical working path, used only for scoped export protection and usage accounting.
    fn path(&self) -> &Path;
}

/// Persist an independently signed local-only editor with an atomic replacement.
pub fn save_local_draft(
    path: &Path,
    working_copy: Digest,
    object: &EncryptedObject,
) -> Result<(), Error> {
    if object.envelope().kind != ObjectKind::LocalDraft {
        return Err(Error::InvalidFile);
    }
    let destination = file::sibling(path, &format!(".{working_copy}.draft"));
    let mut temp = NamedTempFile::new_in(file::parent(&destination)?)?;
    crate::encrypted_object::copy_exact(
        &mut object.reader()?,
        &mut temp,
        object.descriptor().length,
    )?;
    file::persist(temp, &destination, false)
}
/// Parse only the expected local-only object role. A database/blob cannot be substituted.
pub fn read_local_draft(
    path: &Path,
    working_copy: Digest,
    chain: &ControlChain,
) -> Result<Option<EncryptedObject>, Error> {
    let source = file::sibling(path, &format!(".{working_copy}.draft"));
    if !source.try_exists()? {
        return Ok(None);
    }
    let object = EncryptedObject::open(&source, chain)?;
    if object.envelope().kind != ObjectKind::LocalDraft {
        return Err(Error::InvalidFile);
    }
    Ok(Some(object))
}
/// Durably discard an editor. Absence is idempotent; I/O errors remain distinct.
pub fn discard_local_draft(path: &Path, working_copy: Digest) -> Result<(), Error> {
    match std::fs::remove_file(file::sibling(path, &format!(".{working_copy}.draft"))) {
        Ok(()) => File::open(file::parent(path)?)?
            .sync_all()
            .map_err(Error::from),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::Io),
    }
}
