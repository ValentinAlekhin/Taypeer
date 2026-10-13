//! A captured portable ciphertext lease has no worker, author or path-writing capability.
use super::*;
use serde::{Deserialize, Serialize};
use std::io::Write;

/// Public identity of exactly one immutable external-copy attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiphertextExportIdentity {
    /// Logical encrypted database.
    pub database: DatabaseId,
    /// Local protected identity of the selected registered file.
    pub working_copy: Digest,
    /// Pinned trust lineage.
    pub root: Digest,
    /// Exact physical ciphertext generation.
    pub fingerprint: Digest,
    /// Exact physical byte length, within the archive's existing bound.
    pub bytes: u64,
}

/// Immutable ciphertext-only export. Subsequent Save/incoming never replaces this source.
/// This owns no host/profile lease, filesystem writer, author seed or plaintext worker.
pub struct CiphertextExport {
    snapshot: ArchiveSnapshot,
    identity: CiphertextExportIdentity,
}
impl CiphertextExport {
    /// Content-free captured identity; it conveys no write or network permission.
    pub fn identity(&self) -> &CiphertextExportIdentity {
        &self.identity
    }

    /// Copy the captured bytes without opening the current path or acquiring a key.
    /// The storage-owned fingerprint check precedes the caller's explicit destination
    /// finish. This method alone does not confirm output durability or publication.
    pub fn copy_to(&self, output: &mut impl Write) -> Result<(), RuntimeError> {
        self.snapshot
            .copy_verified_ciphertext(output)
            .map_err(cipher_ipc::storage)
    }
}

impl RuntimeHost {
    /// Capture one exact durable catalog/registration binding for an explicit external copy.
    /// Locked copies are permitted. No author credential, worker or writer is created.
    /// Wrong/unknown bindings and protected-state failures remain errors before output I/O.
    pub fn capture_working_copy_export(
        &self,
        copy: &WorkingCopy,
    ) -> Result<CiphertextExport, RuntimeError> {
        {
            let mut catalog = self.catalog.lock().map_err(|_| RuntimeError::Transport)?;
            catalog.reconcile()?;
            if !catalog.contains_copy(copy) {
                return Err(RuntimeError::Closed);
            }
        }
        let context = self.context.for_database(&copy.database)?;
        let registration = context
            .profile
            .registration(&copy.path)?
            .ok_or(RuntimeError::Closed)?;
        if registration.database != copy.database || registration.root != copy.root {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
        }
        let snapshot = {
            // Registration changes and writer removal share this short ownership lock.
            // The returned inode lease remains valid after the lock is released.
            let attached = context.copies.lock().map_err(|_| RuntimeError::Transport)?;
            if let Some(active) = attached.get(&copy.path) {
                if active.database != registration.database
                    || active.root != registration.root
                    || active.working_copy != registration.working_copy
                {
                    return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
                }
                context
                    .coordinator
                    .snapshot(&copy.database)
                    .map_err(sync_error)?
            } else {
                ArchiveSnapshot::open(&copy.path, Some(copy.root)).map_err(cipher_ipc::storage)?
            }
        };
        if snapshot.chain().head().database != copy.database
            || snapshot
                .chain()
                .root()
                .map_err(|_| RuntimeError::Protocol)?
                != copy.root
        {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
        }
        // A closed registered file must not silently export a rolled-back replacement.
        // Exporting a prepared generation does not finalize its unknown source Save.
        let anchor = context
            .profile
            .anchor(&registration)
            .load()
            .map_err(cipher_ipc::storage)?
            .ok_or_else(|| cipher_ipc::storage(taypeer_storage::Error::Changed))?;
        if anchor.accepted != Some(snapshot.fingerprint())
            && anchor.prepared != Some(snapshot.fingerprint())
        {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
        }
        let identity = CiphertextExportIdentity {
            database: copy.database.clone(),
            working_copy: registration.working_copy,
            root: copy.root,
            fingerprint: snapshot.fingerprint(),
            bytes: snapshot.length(),
        };
        Ok(CiphertextExport { snapshot, identity })
    }
}

#[cfg(test)]
mod tests;
