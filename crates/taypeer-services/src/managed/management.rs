//! A management role belongs to a durably applied basis, not merely a received inventory.
use super::*;
use serde::{Deserialize, Serialize};
use taypeer_trust::DeviceId;

pub(super) struct PendingTransfer {
    pub consent: taypeer_trust::HandoffConsent,
    pub candidate: PreparedPublication,
}

/// Content-free authority accepted by this unlocked worker and confirmed in its archive.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenManagementBasis {
    /// Logical database authenticated by the worker.
    pub database: DatabaseId,
    /// Pinned authority root; independent trust sets cannot share a handoff.
    pub root: Digest,
    /// Exact control accepted into the worker's durable projection.
    pub control: Digest,
    /// Exact authenticated baseline whose application was acknowledged.
    pub baseline: Digest,
    /// Current admitted author and transport binding of this worker.
    pub identity: Identity,
    /// Sole manager in the accepted control.
    pub manager: DeviceId,
}
impl std::fmt::Debug for OpenManagementBasis {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OpenManagementBasis([REDACTED])")
    }
}

impl ManagedState {
    pub(super) fn applied_management_basis(
        &self,
    ) -> Result<Option<OpenManagementBasis>, ServiceError> {
        if self.pending_management.is_some() {
            return Err(StorageError::CommitUncertain.into());
        }
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let current = self.port.snapshot()?;
        let control = current.chain().head_hash()?;
        // apply() can install received metadata before a later baseline read/commit fails.
        // Only these fields are advanced together after acknowledged publication.
        if open.control != control
            || self.snapshot.chain().head_hash()? != control
            || self.snapshot.fingerprint() != current.fingerprint()
            || self.baseline != current.metadata().manifest.body.baseline
            || self.accepted != current.metadata().manifest.body.checkpoint
        {
            return Ok(None);
        }
        let author = self.writer()?;
        let head = current.chain().head();
        let identity = head
            .members
            .get(&author.device_id())
            .ok_or(ServiceError::ReadOnly)?
            .identity;
        Ok(Some(OpenManagementBasis {
            database: head.database.clone(),
            root: current.chain().root()?,
            control,
            baseline: self.baseline,
            identity,
            manager: head.manager,
        }))
    }
}

impl DatabaseService {
    /// Read current applied management authority without granting a role from received metadata.
    /// `None` means application is pending; locked/read-only/storage failures remain typed errors.
    pub fn management_basis(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Option<OpenManagementBasis>>, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        if state.write_uncertain || state.pending_icon.is_some() {
            return Err(StorageError::CommitUncertain.into());
        }
        Ok(stamped(session, managed.applied_management_basis()?))
    }
}
