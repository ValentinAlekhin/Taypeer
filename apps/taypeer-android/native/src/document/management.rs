//! Typed management phases. Consent, author keys and private process capabilities remain in Rust.
use super::{DocumentSession, digest, runtime};
use crate::{AndroidError, Host};

/// One immutable explicit transfer, bound to its original verified authority.
#[derive(Clone, uniffi::Record)]
pub struct ManagementIntentView {
    /// Logical database selected by the actual opened worker.
    pub database: String,
    /// Pinned trust-set genesis.
    pub root: String,
    /// Original control approved by both users.
    pub previous: String,
    /// Stable operation; retry must retain every field.
    pub operation: String,
    /// Original managing device identity, never an invented name.
    pub manager: String,
    /// Explicit admitted successor identity.
    pub recipient: String,
}
impl std::fmt::Debug for ManagementIntentView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementIntentView([REDACTED])")
    }
}
impl TryFrom<ManagementIntentView> for taypeer_sync::ManagementIntent {
    type Error = AndroidError;
    fn try_from(value: ManagementIntentView) -> Result<Self, Self::Error> {
        if value.database.is_empty() || value.database.len() > 256 {
            return Err(AndroidError::InvalidOptions);
        }
        Ok(Self {
            database: taypeer_core::DatabaseId::new(value.database),
            root: digest(&value.root)?,
            previous: digest(&value.previous)?,
            operation: digest(&value.operation)?,
            manager: value
                .manager
                .parse()
                .map_err(|_| AndroidError::InvalidOptions)?,
            recipient: value
                .recipient
                .parse()
                .map_err(|_| AndroidError::InvalidOptions)?,
        })
    }
}
impl From<taypeer_sync::ManagementIntent> for ManagementIntentView {
    fn from(value: taypeer_sync::ManagementIntent) -> Self {
        Self {
            database: value.database.to_string(),
            root: value.root.to_string(),
            previous: value.previous.to_string(),
            operation: value.operation.to_string(),
            manager: value.manager.to_string(),
            recipient: value.recipient.to_string(),
        }
    }
}

/// Last durable phase, distinct from a peer's current presence or readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ManagementPhase {
    /// Source intent is saved and delivery/recipient decision remains outstanding.
    Proposed,
    /// Recipient awaits explicit acceptance or rejection.
    WaitingDecision,
    /// Recipient's explicit acceptance is retained and must resume; rejection cannot replace it.
    AcceptRequested,
    /// Recipient consent is durable; management is not granted yet.
    ConsentRecorded,
    /// Explicit rejection is durable.
    Rejected,
    /// Source control/baseline is confirmed and its old role is relinquished.
    SourceCommitted,
    /// Ciphertext delivery succeeded, while recipient application remains outstanding.
    AwaitingApply,
    /// Recipient application and local completion receipt are confirmed.
    Completed,
}
impl From<taypeer_runtime::ManagementPhase> for ManagementPhase {
    fn from(value: taypeer_runtime::ManagementPhase) -> Self {
        use taypeer_runtime::ManagementPhase as Common;
        match value {
            Common::Proposed => Self::Proposed,
            Common::WaitingDecision => Self::WaitingDecision,
            Common::AcceptRequested => Self::AcceptRequested,
            Common::ConsentRecorded => Self::ConsentRecorded,
            Common::Rejected => Self::Rejected,
            Common::SourceCommitted => Self::SourceCommitted,
            Common::AwaitingApply => Self::AwaitingApply,
            Common::Completed => Self::Completed,
        }
    }
}
/// Content-free proof supplied only after the recipient actually applied the related transition.
#[derive(uniffi::Record)]
pub struct ManagementAppliedView {
    /// Original transfer's one signed control.
    pub transition: String,
    /// Recipient's actual durably applied authority, possibly a later lawful successor.
    pub control: String,
    /// Recipient's actual durably applied encrypted baseline.
    pub baseline: String,
}
impl std::fmt::Debug for ManagementAppliedView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementAppliedView([REDACTED])")
    }
}
/// Protected local operation receipt. Neither phase nor endpoint lifetime claims live presence.
#[derive(uniffi::Record)]
pub struct ManagementHandoffView {
    /// Frozen user-approved operation and authority.
    pub intent: ManagementIntentView,
    /// Last durably confirmed local phase.
    pub phase: ManagementPhase,
    /// Confirmed actual application; absent before completion.
    pub applied: Option<ManagementAppliedView>,
}
/// Fresh authenticated readiness; every subsequent action performs its own fresh check.
#[derive(uniffi::Enum)]
pub enum ManagementReadinessView {
    /// No matching actual OPEN recipient/source worker exists.
    Locked,
    /// Actual worker has not durably applied its current authority and baseline.
    PendingApply,
    /// Momentary actual OPEN worker observation for one correlated RPC.
    Ready {
        /// Actual process generation, invalidated by lock/reopen.
        generation: u64,
        /// Verified admitted device identity.
        device: String,
        /// Authority actually applied by this generation.
        control: String,
        /// Baseline actually applied by this generation.
        baseline: String,
    },
}
impl std::fmt::Debug for ManagementReadinessView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementReadinessView([REDACTED])")
    }
}
/// One explicit peer query. Reading protected handoff rows performs no such query.
#[derive(uniffi::Record)]
pub struct ManagementPeerStatusView {
    /// Correlated immutable request, for route/generation-safe presentation.
    pub intent: ManagementIntentView,
    /// Peer's durable phase, absent if it has no exact proposal or decision.
    pub phase: Option<ManagementPhase>,
    /// Fresh actual readiness; never cache this as a persistent online label.
    pub readiness: ManagementReadinessView,
}
impl std::fmt::Debug for ManagementPeerStatusView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementPeerStatusView([REDACTED])")
    }
}
impl From<taypeer_runtime::ManagementPeerStatus> for ManagementPeerStatusView {
    fn from(value: taypeer_runtime::ManagementPeerStatus) -> Self {
        let readiness = match value.readiness {
            taypeer_sync::ManagementReadiness::Locked => ManagementReadinessView::Locked,
            taypeer_sync::ManagementReadiness::PendingApply => {
                ManagementReadinessView::PendingApply
            }
            taypeer_sync::ManagementReadiness::Ready {
                generation,
                identity,
                control,
                baseline,
            } => ManagementReadinessView::Ready {
                generation,
                device: identity.device.to_string(),
                control: control.to_string(),
                baseline: baseline.to_string(),
            },
        };
        Self {
            intent: value.intent.into(),
            phase: value.phase.map(Into::into),
            readiness,
        }
    }
}
impl std::fmt::Debug for ManagementHandoffView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementHandoffView([REDACTED])")
    }
}
impl From<taypeer_runtime::ManagementHandoff> for ManagementHandoffView {
    fn from(value: taypeer_runtime::ManagementHandoff) -> Self {
        Self {
            intent: value.intent.into(),
            phase: value.phase.into(),
            applied: value.applied.map(|applied| ManagementAppliedView {
                transition: applied.transition.to_string(),
                control: applied.control.to_string(),
                baseline: applied.baseline.to_string(),
            }),
        }
    }
}
/// Workflow refusals are separate from unknown publication, storage or transport errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ManagementFailure {
    /// No matching actual OPEN worker; authentication is never automatic.
    NeedsUnlock,
    /// The selected worker has not durably applied its current authority/baseline.
    PendingApply,
    /// Original authority or membership interval changed.
    Stale,
    /// The selected identity or role cannot perform this action.
    WrongRole,
    /// No exact protected request exists.
    Unknown,
    /// Operation identity already belongs to another immutable intent or decision.
    Conflict,
    /// Bounded protected receipts have no room for another operation.
    Capacity,
    /// The peer does not implement the explicit workflow.
    Unsupported,
}
impl From<taypeer_runtime::ManagementFailure> for ManagementFailure {
    fn from(value: taypeer_runtime::ManagementFailure) -> Self {
        use taypeer_runtime::ManagementFailure as Common;
        match value {
            Common::NeedsUnlock => Self::NeedsUnlock,
            Common::PendingApply => Self::PendingApply,
            Common::Stale => Self::Stale,
            Common::WrongRole => Self::WrongRole,
            Common::Unknown => Self::Unknown,
            Common::Conflict => Self::Conflict,
            Common::Capacity => Self::Capacity,
            Common::Unsupported => Self::Unsupported,
        }
    }
}
fn management_error(error: taypeer_runtime::RuntimeError) -> AndroidError {
    match error {
        error @ taypeer_runtime::RuntimeError::Profile(_) => crate::credentials::host_error(error),
        taypeer_runtime::RuntimeError::Service(
            taypeer_services::ServiceError::Unauthorized
            | taypeer_services::ServiceError::Trust(taypeer_trust::Error::Unauthorized),
        ) => AndroidError::Management {
            failure: ManagementFailure::WrongRole,
        },
        other => runtime(other),
    }
}
impl DocumentSession {
    fn management_session(&self) -> Result<taypeer_runtime::ManagementSession, AndroidError> {
        let (database, generation) = self.identity().map_err(|_| AndroidError::Management {
            failure: ManagementFailure::NeedsUnlock,
        })?;
        Ok(taypeer_runtime::ManagementSession {
            database: taypeer_core::DatabaseId::new(database),
            generation,
        })
    }
    fn management_result(
        &self,
        session: &taypeer_runtime::ManagementSession,
        result: Result<taypeer_runtime::ManagementHandoff, taypeer_runtime::RuntimeError>,
    ) -> Result<ManagementHandoffView, AndroidError> {
        let result = result.map_err(management_error)?;
        if self.management_session()? != *session {
            return Err(AndroidError::Management {
                failure: ManagementFailure::NeedsUnlock,
            });
        }
        Ok(result.into())
    }
}
#[uniffi::export]
impl DocumentSession {
    /// Explicit source confirmation. No command mutex is held through the bounded network workflow.
    /// Unknown/late results retain the original protected operation for exact host retry.
    pub fn propose_management(
        &self,
        recipient: String,
        operation: String,
    ) -> Result<ManagementHandoffView, AndroidError> {
        let session = self.management_session()?;
        let recipient = recipient
            .parse()
            .map_err(|_| AndroidError::InvalidOptions)?;
        let operation = digest(&operation)?;
        self.management_result(
            &session,
            self.host.propose_management(&session, recipient, operation),
        )
    }
    /// Explicit recipient acceptance; signature never crosses the UI boundary.
    pub fn accept_management(
        &self,
        intent: ManagementIntentView,
    ) -> Result<ManagementHandoffView, AndroidError> {
        let session = self.management_session()?;
        self.management_result(
            &session,
            self.host.accept_management(&session, &intent.try_into()?),
        )
    }
    /// Explicit recipient rejection, durably recorded before acknowledgement.
    pub fn reject_management(
        &self,
        intent: ManagementIntentView,
    ) -> Result<ManagementHandoffView, AndroidError> {
        let session = self.management_session()?;
        self.management_result(
            &session,
            self.host.reject_management(&session, &intent.try_into()?),
        )
    }
}
#[uniffi::export]
impl Host {
    /// Explicit bounded authenticated query for this exact operation's peer.
    /// A successful previous exchange or mere endpoint lifetime never produces Ready.
    pub fn management_status(
        &self,
        intent: ManagementIntentView,
    ) -> Result<ManagementPeerStatusView, AndroidError> {
        self.runtime
            .management_status(&intent.try_into()?)
            .map(Into::into)
            .map_err(management_error)
    }
    /// Read confirmed protected rows without network, automatic unlocking or author access.
    pub fn management_handoffs(
        &self,
        database: String,
    ) -> Result<Vec<ManagementHandoffView>, AndroidError> {
        if database.is_empty() || database.len() > 256 {
            return Err(AndroidError::InvalidOptions);
        }
        self.runtime
            .management_handoffs(&taypeer_core::DatabaseId::new(database))
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(management_error)
    }
    /// Continue exactly the original operation; committed delivery also works while source is locked.
    pub fn retry_management(
        &self,
        intent: ManagementIntentView,
    ) -> Result<ManagementHandoffView, AndroidError> {
        self.runtime
            .retry_management(&intent.try_into()?)
            .map(Into::into)
            .map_err(management_error)
    }
}
