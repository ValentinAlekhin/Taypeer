//! Public recovery receipts never contain the entered name, password or a content verifier.
use super::{DocumentSession, WorkingCopyView, digest, failure, runtime};
use crate::{AndroidError, CiphertextFiles, DocumentProcess, Host};
use std::sync::Arc;
use taypeer_runtime::{RecoveryPublication, RuntimeError};

const MAX_RECOVERY_METADATA: usize = 64 * 1024;

pub(super) fn decode_public<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, AndroidError> {
    if bytes.len() > MAX_RECOVERY_METADATA {
        return Err(AndroidError::InvalidFile);
    }
    serde_json::from_slice(bytes).map_err(|_| AndroidError::InvalidFile)
}
pub(super) fn encode_public(value: &impl serde::Serialize) -> Result<Vec<u8>, AndroidError> {
    let bytes = serde_json::to_vec(value).map_err(failure)?;
    if bytes.len() > MAX_RECOVERY_METADATA {
        return Err(AndroidError::InvalidFile);
    }
    Ok(bytes)
}
pub(super) fn recovery_error(error: RuntimeError) -> AndroidError {
    match error {
        error @ RuntimeError::Profile(_) => crate::credentials::host_error(error),
        other => runtime(other),
    }
}

/// Reconciled public recovery phase; none of these values alone authorizes adoption.
#[derive(Clone, Copy, uniffi::Enum)]
pub enum RecoveryPhase {
    /// Exact encrypted source exists; the original worker owns the password attempt.
    SourceFrozen,
    /// A separately encrypted candidate requires real password authentication.
    CandidatePublished,
    /// Protected proof is confirmed; source closure must still be verified.
    ProofConfirmed,
    /// The replacement working copy is durably catalogued.
    Adopted,
    /// The original worker vanished before publishing a candidate.
    RefusedBeforePublication,
}
impl From<taypeer_runtime::RecoveryPhase> for RecoveryPhase {
    fn from(value: taypeer_runtime::RecoveryPhase) -> Self {
        match value {
            taypeer_runtime::RecoveryPhase::SourceFrozen => Self::SourceFrozen,
            taypeer_runtime::RecoveryPhase::CandidatePublished => Self::CandidatePublished,
            taypeer_runtime::RecoveryPhase::ProofConfirmed => Self::ProofConfirmed,
            taypeer_runtime::RecoveryPhase::Adopted => Self::Adopted,
            taypeer_runtime::RecoveryPhase::RefusedBeforePublication => {
                Self::RefusedBeforePublication
            }
        }
    }
}
/// Public restart hint verified against protected host state, never a plaintext name/password.
#[derive(uniffi::Record)]
pub struct PendingRecoveryView {
    /// The frozen operation; a new identifier cannot displace its unknown result.
    pub operation: String,
    /// Registered source location and public identity.
    pub source: WorkingCopyView,
    /// Reconciled durable phase; source writer closure remains a separate requirement.
    pub phase: RecoveryPhase,
}

#[uniffi::export]
impl DocumentSession {
    /// Freeze exact input inside this worker, publish a separate encrypted candidate and confirm proof.
    /// The returned bounded locator cannot adopt the candidate without confirmed source closure.
    pub fn recover_working_copy(
        &self,
        operation: String,
        name: String,
        password: String,
    ) -> Result<Vec<u8>, AndroidError> {
        let publication: RecoveryPublication =
            self.command(taypeer_runtime::Command::RecoverWorkingCopy {
                operation: digest(&operation)?,
                name,
                password: zeroize::Zeroizing::new(password.into_bytes()),
            })?;
        encode_public(&publication).map_err(|_| AndroidError::CommitUncertain)
    }
}

#[uniffi::export]
impl Host {
    /// Discover exact unfinished operations after restart using protected profile state.
    pub fn pending_recoveries(&self) -> Result<Vec<PendingRecoveryView>, AndroidError> {
        self.runtime
            .pending_recoveries()
            .map(|rows| {
                rows.into_iter()
                    .map(|row| PendingRecoveryView {
                        operation: row.source.operation.to_string(),
                        source: row.working_copy.into(),
                        phase: row.phase.into(),
                    })
                    .collect()
            })
            .map_err(recovery_error)
    }
    /// Password-authenticated recovery boot uses the retained source and never permits old-chain exchange.
    pub fn open_recovery_document(
        &self,
        path: String,
        password: String,
        operation: String,
        process: Box<dyn DocumentProcess>,
        files: Box<dyn CiphertextFiles>,
    ) -> Result<Arc<DocumentSession>, AndroidError> {
        self.open_session_with_recovery(
            std::path::PathBuf::from(path),
            password,
            None,
            process,
            files,
            Some(digest(&operation)?),
        )
    }

    /// Adopt only the protected exact proof after real source workers and writer capabilities close.
    pub fn adopt_recovery(&self, publication: Vec<u8>) -> Result<WorkingCopyView, AndroidError> {
        let publication: RecoveryPublication = decode_public(&publication)?;
        self.runtime
            .adopt_closed_platform_recovery(&publication)
            .map(Into::into)
            .map_err(recovery_error)
    }
}
