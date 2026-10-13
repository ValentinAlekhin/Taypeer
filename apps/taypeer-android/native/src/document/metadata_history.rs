//! Bounded metadata history projections; causal validation and durable receipts stay shared.
use super::{
    DocumentSession, FormKind, GroupParentCandidate, IconView, runtime, session::identifier,
};
use crate::AndroidError;
use taypeer_core::{DatabaseId, GroupId, RevisionId};
use taypeer_runtime::{Command, RuntimeError};
use taypeer_services::{DatabaseRevisionView, GroupRevisionView, ServiceError};

/// Ordinary descriptive fields of the selected saved or current object.
#[derive(uniffi::Record)]
pub struct MetadataSnapshotView {
    /// Exact descriptive name, never an invented entry title.
    pub name: String,
    /// Exact optional description; empty and absent remain distinct.
    pub description: Option<String>,
    /// Selected group parent. Database metadata has no parent field.
    pub parent: Option<String>,
    /// Selected group icon without original content or storage references.
    pub icon: Option<IconView>,
}
impl std::fmt::Debug for MetadataSnapshotView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MetadataSnapshotView([REDACTED])")
    }
}

/// One original confirmation and the current projection read from the same shared basis.
#[derive(uniffi::Record)]
pub struct MetadataRevisionView {
    /// Group or database; editor-only and entry kinds are not returned.
    pub kind: FormKind,
    /// Exact group identity, absent for database metadata.
    pub target: Option<String>,
    /// Selected accessible original confirmation.
    pub revision: String,
    /// UTC milliseconds, without a fabricated actor label.
    pub saved_at: i64,
    /// Immutable selected saved metadata.
    pub saved: MetadataSnapshotView,
    /// Current selected metadata of the same object.
    pub current: MetadataSnapshotView,
    /// Reviewed current-side causal heads; retain exactly on restore retry.
    pub basis: Vec<String>,
    /// Valid destinations and labels from the shared basis; top level is None.
    pub parent_candidates: Vec<GroupParentCandidate>,
}
impl std::fmt::Debug for MetadataRevisionView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MetadataRevisionView([REDACTED])")
    }
}

fn group_snapshot(value: taypeer_core::GroupSnapshot) -> MetadataSnapshotView {
    MetadataSnapshotView {
        name: value.group.name,
        description: value.description,
        parent: value.group.parent.map(|id| id.to_string()),
        icon: Some(value.group.icon.into()),
    }
}
fn database_snapshot(value: taypeer_core::DatabaseSnapshot) -> MetadataSnapshotView {
    MetadataSnapshotView {
        name: value.name,
        description: value.description,
        parent: None,
        icon: None,
    }
}
fn group_view(value: GroupRevisionView) -> Result<MetadataRevisionView, AndroidError> {
    // Reject unbounded presentation metadata before constructing the public JNI DTO.
    super::encode(&value)?;
    Ok(MetadataRevisionView {
        kind: FormKind::Group,
        target: Some(value.saved.group_id.to_string()),
        revision: value.saved.id.to_string(),
        saved_at: value.saved.saved_at,
        saved: group_snapshot(value.saved.snapshot),
        current: group_snapshot(value.current),
        basis: value.basis,
        parent_candidates: value
            .parent_candidates
            .into_iter()
            .map(|row| GroupParentCandidate {
                id: row.id.to_string(),
                name: row.name,
            })
            .collect(),
    })
}
fn database_view(value: DatabaseRevisionView) -> Result<MetadataRevisionView, AndroidError> {
    super::encode(&value)?;
    Ok(MetadataRevisionView {
        kind: FormKind::Database,
        target: None,
        revision: value.saved.id.to_string(),
        saved_at: value.saved.saved_at,
        saved: database_snapshot(value.saved.snapshot),
        current: database_snapshot(value.current),
        basis: value.basis,
        parent_candidates: Vec::new(),
    })
}

fn reviewed_heads(heads: Vec<String>) -> Result<Vec<String>, AndroidError> {
    if heads.is_empty()
        || heads.len() > 4096
        || heads.iter().any(|head| {
            head.len() != 64
                || !head
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
    {
        return Err(AndroidError::InvalidOptions);
    }
    Ok(heads)
}
fn restore_error(error: RuntimeError) -> AndroidError {
    match error {
        RuntimeError::Service(
            ServiceError::OperationConflict | ServiceError::NotFound | ServiceError::InvalidContext,
        ) => AndroidError::InvalidInput,
        other => runtime(other),
    }
}

#[uniffi::export]
impl DocumentSession {
    /// View and compare exact metadata without opening a form or performing image requests.
    pub fn metadata_revision(
        &self,
        kind: FormKind,
        target: Option<String>,
        revision: String,
    ) -> Result<MetadataRevisionView, AndroidError> {
        let revision = RevisionId::new(identifier(revision)?);
        match kind {
            FormKind::Group => group_view(self.command(Command::GroupRevisionView {
                group: GroupId::new(identifier(target.ok_or(AndroidError::InvalidOptions)?)?),
                revision,
            })?),
            FormKind::Database if target.is_none() => {
                database_view(self.command(Command::DatabaseRevisionView(revision))?)
            }
            FormKind::Entry | FormKind::NewEntry | FormKind::NewGroup | FormKind::Database => {
                Err(AndroidError::InvalidOptions)
            }
        }
    }

    /// Restore the same active group once, with exact reviewed heads and explicit parent.
    pub fn restore_group_revision(
        &self,
        group: String,
        revision: String,
        parent: Option<String>,
        basis: Vec<String>,
        operation: String,
    ) -> Result<String, AndroidError> {
        let id: GroupId = self.command_with_error(
            Command::RestoreGroupRevision {
                group: GroupId::new(identifier(group)?),
                revision: RevisionId::new(identifier(revision)?),
                parent: parent.map(identifier).transpose()?.map(GroupId::new),
                heads: reviewed_heads(basis)?,
                operation: taypeer_core::OperationId::new(identifier(operation)?),
            },
            restore_error,
        )?;
        Ok(id.to_string())
    }

    /// Restore only name and description. Policy, authority and copy identity are unchanged.
    pub fn restore_database_revision(
        &self,
        revision: String,
        basis: Vec<String>,
        operation: String,
    ) -> Result<String, AndroidError> {
        let id: DatabaseId = self.command_with_error(
            Command::RestoreDatabaseRevision {
                revision: RevisionId::new(identifier(revision)?),
                heads: reviewed_heads(basis)?,
                operation: taypeer_core::OperationId::new(identifier(operation)?),
            },
            restore_error,
        )?;
        Ok(id.to_string())
    }
}

#[cfg(test)]
mod tests;
