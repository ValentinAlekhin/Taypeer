//! Correlation for explicit management handoffs; authenticated transport supplies identity.
use crate::Error;
use serde::{Deserialize, Serialize};
use taypeer_core::DatabaseId;
use taypeer_trust::{ControlChain, DeviceId, Digest, HandoffConsent, Identity};

/// One immutable, explicit handoff between two admitted devices on an exact trust basis.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementIntent {
    /// Logical database; independent recovered trust sets remain separate.
    pub database: DatabaseId,
    /// Pinned genesis of the admitted trust set.
    pub root: Digest,
    /// Exact authority both users approve before the source transition.
    pub previous: Digest,
    /// Stable identity retained through retry, lock and restart.
    pub operation: Digest,
    /// Sole manager at `previous`.
    pub manager: DeviceId,
    /// Explicit successor, admitted at `previous`.
    pub recipient: DeviceId,
}
impl ManagementIntent {
    /// Check the intent against verified authority, including its original membership interval.
    /// This does not establish live readiness or authorize a transport caller.
    pub fn verify(&self, chain: &ControlChain) -> Result<(), Error> {
        let previous = chain.at(self.previous)?;
        if chain.root()? != self.root
            || previous.database != self.database
            || previous.manager != self.manager
            || self.manager == self.recipient
            || !previous.members.contains_key(&self.recipient)
        {
            return Err(Error::Unauthorized);
        }
        if !chain.continuous(self.manager, self.previous)?
            || !chain.continuous(self.recipient, self.previous)?
        {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    /// Find the original single signed transfer, including beneath later lawful successors.
    /// A receipt for another action/recipient never satisfies this intent.
    pub fn committed_control(&self, chain: &ControlChain) -> Result<Option<Digest>, Error> {
        self.verify(chain)?;
        for record in chain.records() {
            if record.body.operation != Some(self.operation) {
                continue;
            }
            let matching = record.body.previous == Some(self.previous)
                && record.body.manager == self.recipient
                && record.body.handoff.as_ref().is_some_and(|consent| {
                    consent.previous == self.previous
                        && consent.operation == self.operation
                        && consent.recipient == self.recipient
                });
            return if matching {
                Ok(Some(record.hash()?))
            } else {
                Err(Error::State)
            };
        }
        Ok(None)
    }
}

/// A fresh observation of the peer's actual unlocked worker, never a saved online label.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ManagementReadiness {
    /// No registered worker currently authorizes this database.
    Locked,
    /// A worker is open but has not confirmed application of current control/baseline.
    PendingApply,
    /// Exact content-free authority confirmed by the live generation for this request.
    Ready {
        /// Actual local process generation; a later unlock cannot reuse it.
        generation: u64,
        /// Admitted author and transport binding checked inside the worker.
        identity: Identity,
        /// Durably applied authority.
        control: Digest,
        /// Durably applied baseline.
        baseline: Digest,
    },
}

/// Durable application of the original handoff; receipt may reflect later accepted successors.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementApplied {
    /// Original immutable operation.
    pub intent: ManagementIntent,
    /// Signed control that transferred management exactly once.
    pub transition: Digest,
    /// Authority actually applied by the recipient's worker.
    pub control: Digest,
    /// Baseline actually applied and acknowledged by its persistence port.
    pub baseline: Digest,
}

/// Recipient's saved decision, separate from its current live readiness.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ManagementDecision {
    /// No proposal with this exact identity was accepted into protected local state.
    Unknown,
    /// The source's explicit intent is durable; readiness alone never implies approval.
    Proposed,
    /// The proposal is durable and awaits an explicit local decision.
    Waiting,
    /// Exact consent is durably recorded; it grants no applied managing role itself.
    Accepted(HandoffConsent),
    /// Explicit rejection is durable and idempotent.
    Rejected,
    /// The source confirmed the original control transition; delivery can continue locked.
    Committed(Digest),
    /// The recipient durably applied the related transition and saved this receipt.
    Applied(ManagementApplied),
}

/// Correlated result of one authenticated RPC, without document data or credentials.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementReply {
    /// Exact immutable intent supplied by the caller.
    pub intent: ManagementIntent,
    /// Fresh caller challenge; old readiness responses cannot be reused across requests.
    pub challenge: Digest,
    /// Saved decision/receipt, independent of worker lifetime.
    pub decision: ManagementDecision,
    /// Live generation/basis checked for this reply only.
    pub readiness: ManagementReadiness,
}

macro_rules! redacted_debug {
    ($($kind:ident),+ $(,)?) => {$ (
        impl std::fmt::Debug for $kind {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(concat!(stringify!($kind), "([REDACTED])"))
            }
        }
    )+};
}
redacted_debug!(
    ManagementIntent,
    ManagementReadiness,
    ManagementApplied,
    ManagementDecision,
    ManagementReply
);
