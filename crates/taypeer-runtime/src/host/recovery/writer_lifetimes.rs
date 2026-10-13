//! Actual host writer capabilities, separate from retained ciphertext registration.
use crate::RuntimeError;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};
use taypeer_storage::{ArchiveSnapshot, CipherPersistence, EncryptedObject, PreparedCommit};
use taypeer_sync::CoordinatorPersistence;
use taypeer_trust::{ControlChain, Digest};

#[derive(Default)]
pub(crate) struct WriterLifetimes(Mutex<Vec<(PathBuf, Weak<()>)>>);
impl WriterLifetimes {
    pub(crate) fn acquire(&self, path: &Path) -> Result<Arc<()>, RuntimeError> {
        let mut writers = self.0.lock().map_err(|_| RuntimeError::Transport)?;
        writers.retain(|(_, lifetime)| lifetime.strong_count() != 0);
        let lifetime = Arc::new(());
        writers.push((path.to_owned(), Arc::downgrade(&lifetime)));
        Ok(lifetime)
    }

    pub(super) fn with_closed_source<T>(
        &self,
        path: &Path,
        adopt: impl FnOnce() -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let mut writers = self.0.lock().map_err(|_| RuntimeError::Transport)?;
        writers.retain(|(_, lifetime)| lifetime.strong_count() != 0);
        if writers.iter().any(|(active, _)| active == path) {
            return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Busy));
        }
        // A new local/Binder writer cannot pass acquire between closure verification,
        // coordinator removal and catalog publication. No worker/control lock is held.
        adopt()
    }
}

pub(crate) struct LocalWriter {
    persistence: CoordinatorPersistence,
    _lifetime: Arc<()>,
}
impl LocalWriter {
    pub(crate) fn new(persistence: CoordinatorPersistence, lifetime: Arc<()>) -> Self {
        Self {
            persistence,
            _lifetime: lifetime,
        }
    }
}
impl CipherPersistence for LocalWriter {
    fn snapshot(&self) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.persistence.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.persistence.commit(request)
    }
    fn working_copy(&self) -> Digest {
        self.persistence.working_copy()
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), taypeer_storage::Error> {
        self.persistence.save_draft(object)
    }
    fn load_draft(
        &self,
        chain: &ControlChain,
    ) -> Result<Option<EncryptedObject>, taypeer_storage::Error> {
        self.persistence.load_draft(chain)
    }
    fn discard_draft(&self) -> Result<(), taypeer_storage::Error> {
        self.persistence.discard_draft()
    }
    fn path(&self) -> &Path {
        self.persistence.path()
    }
}
