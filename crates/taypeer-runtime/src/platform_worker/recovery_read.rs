//! Read-only recovery bootstrap. Ordinary authority and frozen-chain guards stay unchanged.
use super::*;
use taypeer_storage::Error;

pub(super) struct ReadPort {
    snapshot: ArchiveSnapshot,
    copy: Digest,
}
impl ReadPort {
    pub(super) fn new(snapshot: ArchiveSnapshot, copy: Digest) -> Self {
        Self { snapshot, copy }
    }
}
impl CipherPersistence for ReadPort {
    fn snapshot(&self) -> Result<ArchiveSnapshot, Error> {
        Ok(self.snapshot.clone())
    }
    fn commit(&self, _request: PreparedCommit) -> Result<ArchiveSnapshot, Error> {
        Err(Error::Busy)
    }
    fn working_copy(&self) -> Digest {
        self.copy
    }
    fn save_draft(&self, _object: &EncryptedObject) -> Result<(), Error> {
        Err(Error::Busy)
    }
    fn load_draft(
        &self,
        _chain: &taypeer_trust::ControlChain,
    ) -> Result<Option<EncryptedObject>, Error> {
        Ok(None)
    }
    // No local collection was loaded. Opening this read-only snapshot must not delete the real source form.
    fn discard_draft(&self) -> Result<(), Error> {
        Ok(())
    }
    fn path(&self) -> &Path {
        Path::new("")
    }
}
pub(super) fn allowed(command: &crate::Command, operation: Option<Digest>) -> bool {
    match command {
        crate::Command::DatabaseInfo | crate::Command::SessionAuthority | crate::Command::Lock => {
            true
        }
        crate::Command::RecoverWorkingCopy {
            operation: requested,
            ..
        } => Some(*requested) == operation,
        _ => false,
    }
}
