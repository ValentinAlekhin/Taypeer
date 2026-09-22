//! Explicit synthetic adapters only. This module is absent from production builds.
use std::{
    path::Path,
    time::{Duration, Instant},
};
use taypeer_storage::{ArchiveSnapshot, CipherPersistence, EncryptedObject, Error, PreparedCommit};
use taypeer_trust::{ControlChain, Digest};
pub(super) struct Persistence<T>(pub T);
fn marker(port: &impl CipherPersistence, suffix: &str) -> bool {
    port.path().with_extension(suffix).exists()
}
pub(super) fn commit(
    port: &impl CipherPersistence,
    request: PreparedCommit,
) -> Result<ArchiveSnapshot, Error> {
    if marker(port, "fail-before") {
        return Err(Error::Io);
    }
    let result = port.commit(request)?;
    if marker(port, "fail-after") {
        return Err(Error::CommitUncertain);
    }
    if marker(port, "hold-after") {
        std::fs::write(port.path().with_extension("committed"), b"PUBLIC committed")
            .map_err(|_| Error::Io)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while marker(port, "hold-after") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(result)
}
pub(super) fn discard(port: &impl CipherPersistence) -> Result<(), Error> {
    if marker(port, "fail-cleanup") {
        return Err(Error::Io);
    }
    port.discard_draft()
}
impl<T: CipherPersistence> CipherPersistence for Persistence<T> {
    fn snapshot(&self) -> Result<ArchiveSnapshot, Error> {
        self.0.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, Error> {
        commit(&self.0, request)
    }
    fn working_copy(&self) -> Digest {
        self.0.working_copy()
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), Error> {
        self.0.save_draft(object)
    }
    fn load_draft(&self, chain: &ControlChain) -> Result<Option<EncryptedObject>, Error> {
        self.0.load_draft(chain)
    }
    fn discard_draft(&self) -> Result<(), Error> {
        discard(&self.0)
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
}
