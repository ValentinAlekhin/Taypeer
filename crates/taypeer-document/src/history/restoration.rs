//! Explicit metadata restoration reuses causal Save and placement guards in one transaction.
use super::*;
use serde::de::DeserializeOwned;
use taypeer_core::{DatabaseMetadataPatch, FieldUpdate, GroupMetadataPatch};

/// Public, content-free reviewed intent for restoring one group's metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRevisionRestore {
    /// Stable active group identity; restoration never creates or revives a group.
    pub group: GroupId,
    /// Exact retained source confirmation.
    pub revision: RevisionId,
    /// Explicit destination, or the top level.
    pub parent: Option<GroupId>,
    /// Current-side causal context reviewed before confirming restoration.
    pub heads: Vec<String>,
}

/// Reviewed intent for restoring only the name and description of the same database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseRevisionRestore {
    /// Exact retained source confirmation.
    pub revision: RevisionId,
    /// Current-side causal context reviewed before confirming restoration.
    pub heads: Vec<String>,
}

impl Document {
    /// Confirm saved metadata and an explicit valid placement on the current group generation.
    /// A new operation always records one Restore, even for equal values. Exact replay is read
    /// before source and destination lookup so later edits or history purges are preserved.
    pub fn restore_group_revision_command(
        &mut self,
        request: &GroupRevisionRestore,
        now: Timestamp,
        receipt: &CommandReceipt<'_>,
    ) -> Result<Group, Error> {
        if let Some(result) = restored_receipt(self, receipt)? {
            return Ok(result);
        }
        let saved = self.group_revision(&request.group, &request.revision)?;
        self.validate_group_parent(
            Some(&request.group),
            request.parent.as_ref(),
            &request.heads,
        )?;
        self.confirm_group_metadata(
            &request.group,
            &GroupMetadataPatch {
                name: FieldUpdate::Set(saved.snapshot.group.name),
                description: saved
                    .snapshot
                    .description
                    .map_or(FieldUpdate::Clear, FieldUpdate::Set),
                icon: FieldUpdate::Set(saved.snapshot.group.icon),
                parent: request
                    .parent
                    .clone()
                    .map_or(FieldUpdate::Clear, FieldUpdate::Set),
            },
            &request.heads,
            now,
            Some(receipt),
            RevisionKind::Restore,
        )
    }

    /// Restore descriptive fields without changing database identity, policy or authority.
    /// Confirmation and its exact receipt share one Restore transaction.
    pub fn restore_database_revision_command(
        &mut self,
        request: &DatabaseRevisionRestore,
        now: Timestamp,
        receipt: &CommandReceipt<'_>,
    ) -> Result<(), Error> {
        if let Some(result) = restored_receipt(self, receipt)? {
            return Ok(result);
        }
        let saved = self.database_revision(&request.revision)?;
        self.confirm_database_metadata(
            &DatabaseMetadataPatch {
                name: FieldUpdate::Set(saved.snapshot.name),
                description: saved
                    .snapshot
                    .description
                    .map_or(FieldUpdate::Clear, FieldUpdate::Set),
            },
            &request.heads,
            now,
            Some(receipt),
            RevisionKind::Restore,
        )
    }
}

fn restored_receipt<T: DeserializeOwned>(
    document: &Document,
    receipt: &CommandReceipt<'_>,
) -> Result<Option<T>, Error> {
    let Some((fingerprint, result)) = document.command_receipt(receipt.operation, receipt.kind)?
    else {
        return Ok(None);
    };
    if &fingerprint != receipt.fingerprint {
        return Err(Error::DuplicateId);
    }
    serde_json::from_value(result)
        .map(Some)
        .map_err(|_| Error::InvalidDocument)
}
