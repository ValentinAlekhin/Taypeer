//! In-process adapter for macOS and integration tests; CLI uses the same port over IPC.
use crate::{Coordinator, Error};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use taypeer_core::DatabaseId;
use taypeer_storage::{ArchiveSnapshot, CipherPersistence, EncryptedObject, PreparedCommit};
use taypeer_trust::{ControlChain, Digest};

/// A handle to a registered writer, with no author key or decrypted state.
pub struct CoordinatorPersistence {
    coordinator: Arc<Coordinator>,
    database: DatabaseId,
    path: PathBuf,
    working_copy: Digest,
}
impl CoordinatorPersistence {
    /// Bind the service to an existing registered copy. The profile supplies its stable local ID.
    pub fn new(
        coordinator: Arc<Coordinator>,
        database: DatabaseId,
        path: PathBuf,
        working_copy: Digest,
    ) -> Result<Self, Error> {
        let path = path
            .canonicalize()
            .map_err(|_| Error::Storage(taypeer_storage::Error::Io))?;
        coordinator.check_path(&database, &path)?;
        Ok(Self {
            coordinator,
            database,
            path,
            working_copy,
        })
    }
}
fn storage(error: Error) -> taypeer_storage::Error {
    match error {
        Error::Storage(error) => error,
        Error::Trust(error) => error.into(),
        Error::State => taypeer_storage::Error::Changed,
        Error::Transport | Error::Timeout | Error::Protocol | Error::Unauthorized => {
            taypeer_storage::Error::Io
        }
    }
}
impl CipherPersistence for CoordinatorPersistence {
    fn snapshot(&self) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.coordinator.snapshot(&self.database).map_err(storage)
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.coordinator
            .commit(&self.database, request)
            .map_err(storage)
    }
    fn working_copy(&self) -> Digest {
        self.working_copy
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), taypeer_storage::Error> {
        taypeer_storage::save_local_draft(&self.path, self.working_copy, object)
    }
    fn load_draft(
        &self,
        chain: &ControlChain,
    ) -> Result<Option<EncryptedObject>, taypeer_storage::Error> {
        taypeer_storage::read_local_draft(&self.path, self.working_copy, chain)
    }
    fn discard_draft(&self) -> Result<(), taypeer_storage::Error> {
        taypeer_storage::discard_local_draft(&self.path, self.working_copy)
    }
    fn preserve_before(&self, operation: Digest) -> Result<(), taypeer_storage::Error> {
        self.coordinator
            .preserve_before(&self.database, operation)
            .map_err(storage)
    }
    fn path(&self) -> &Path {
        &self.path
    }
}
