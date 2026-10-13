//! Authenticated selected metadata history and exact, durable restoration commands.
use crate::*;
use serde::{Deserialize, Serialize};
use taypeer_core::{
    DatabaseSnapshot, GroupSnapshot, OperationId, SavedDatabaseRevision, SavedGroupRevision,
};

pub use taypeer_document::{DatabaseRevisionRestore, GroupRevisionRestore};

/// A saved group and its current projection read from one authenticated document basis.
#[derive(Clone, Serialize, Deserialize)]
pub struct GroupRevisionView {
    /// Exact accessible immutable confirmation.
    pub saved: SavedGroupRevision,
    /// Selected metadata of the same active group generation.
    pub current: GroupSnapshot,
    /// Reviewed current-side heads for an explicit restoration.
    pub basis: Vec<String>,
    /// Shared valid destinations with labels from this basis; None represents top level.
    pub parent_candidates: Vec<GroupParentCandidate>,
}

/// Descriptive database history excludes policy, authority and local copy settings.
#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseRevisionView {
    /// Exact accessible immutable confirmation.
    pub saved: SavedDatabaseRevision,
    /// Current name and description of the same logical database.
    pub current: DatabaseSnapshot,
    /// Reviewed current-side heads for an explicit restoration.
    pub basis: Vec<String>,
}

impl std::fmt::Debug for GroupRevisionView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupRevisionView([REDACTED])")
    }
}
impl std::fmt::Debug for DatabaseRevisionView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DatabaseRevisionView([REDACTED])")
    }
}

impl DatabaseService {
    /// Read both comparison sides without opening a draft, mutating history or acquiring images.
    pub fn group_revision_view(
        &self,
        session: &SessionToken,
        group: &GroupId,
        revision: &RevisionId,
    ) -> Result<SessionValue<GroupRevisionView>, ServiceError> {
        let document = self.checked(session)?.document();
        let saved = document.group_revision(group, revision)?;
        let current = document.group_snapshot(group)?;
        let basis = document.heads();
        let parent_candidates = document
            .group_parent_candidates(Some(group), &basis)?
            .into_iter()
            .map(|group| GroupParentCandidate {
                id: group.id,
                name: group.name,
            })
            .collect();
        Ok(stamped(
            session,
            GroupRevisionView {
                saved,
                current,
                basis,
                parent_candidates,
            },
        ))
    }

    /// Read one retained database confirmation alongside its current descriptive projection.
    pub fn database_revision_view(
        &self,
        session: &SessionToken,
        revision: &RevisionId,
    ) -> Result<SessionValue<DatabaseRevisionView>, ServiceError> {
        let document = self.checked(session)?.document();
        Ok(stamped(
            session,
            DatabaseRevisionView {
                saved: document.database_revision(revision)?,
                current: document.database_snapshot()?,
                basis: document.heads(),
            },
        ))
    }

    /// Atomically restore an active group's metadata and explicit placement once.
    /// Exact receipts are checked before looking up a purged source or a changed destination.
    pub fn restore_group_revision(
        &mut self,
        session: &SessionToken,
        request: &GroupRevisionRestore,
        operation: &OperationId,
    ) -> Result<SessionValue<GroupId>, ServiceError> {
        let fingerprint = commands::fingerprint(request)?;
        let now = (self.clock)();
        let restored: taypeer_core::Group = self.checked_mut(session)?.command(
            operation,
            "restore_group_revision",
            fingerprint,
            |document, receipt| {
                document
                    .restore_group_revision_command(request, now, receipt)
                    .map_err(operations::candidate_input_error)
            },
        )?;
        Ok(stamped(session, restored.id))
    }

    /// Restore only name/description with one durable Restore and a secret-free exact receipt.
    pub fn restore_database_revision(
        &mut self,
        session: &SessionToken,
        request: &DatabaseRevisionRestore,
        operation: &OperationId,
    ) -> Result<SessionValue<DatabaseId>, ServiceError> {
        let fingerprint = commands::fingerprint(request)?;
        let now = (self.clock)();
        self.checked_mut(session)?.command(
            operation,
            "restore_database_revision",
            fingerprint,
            |document, receipt| {
                document
                    .restore_database_revision_command(request, now, receipt)
                    .map_err(operations::candidate_input_error)
            },
        )?;
        Ok(stamped(session, session.database.clone()))
    }
}
