//! Public, content-free bindings for one local trust recovery.
use serde::{Deserialize, Serialize};
use taypeer_core::DatabaseId;
use taypeer_trust::{Digest, Identity};

/// One authenticated immutable source. These local identifiers never authorize P2P exchange.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySource {
    /// Explicit operation, reused until its actual publication is reconciled.
    pub operation: Digest,
    /// Logical database, preserved by recovery.
    pub database: DatabaseId,
    /// Protected local physical-copy binding.
    pub working_copy: Digest,
    /// Original authority root.
    pub root: Digest,
    /// Exact complete ciphertext generation, not the latest mutable source path.
    pub fingerprint: Digest,
}

/// Public part of the frozen request. Name and password belong only to the worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryIntent {
    /// Exact immutable source and operation.
    pub source: RecoverySource,
    /// Full new manager identity, including its independent transport public key.
    pub identity: Identity,
}

/// Locator of an authenticated candidate. Only a matching protected proof permits adoption.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPublication {
    /// Public frozen bindings; no user-content hashes.
    pub intent: RecoveryIntent,
    /// Independent new authority root.
    pub root: Digest,
    /// Exact candidate ciphertext, checked again at confirmation and adoption.
    pub fingerprint: Digest,
}

/// A restart-visible public operation linked to its existing catalog source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRecovery {
    /// Exact protected operation and immutable source binding.
    pub source: RecoverySource,
    /// Existing source locator, never a candidate activation claim.
    pub working_copy: crate::WorkingCopy,
    /// Actual reconciled phase; source-writer closure is checked only during adoption.
    pub phase: RecoveryPhase,
}

/// Last reconciled durable phase. Writer closure must still be checked afresh on adoption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryPhase {
    /// Immutable ciphertext source is durable; the original worker owns the password attempt.
    SourceFrozen,
    /// The encrypted candidate exists; password authentication is still required.
    CandidatePublished,
    /// The protected registration and exact recovery proof have been confirmed.
    ProofConfirmed,
    /// The catalog contains the new copy.
    Adopted,
    /// No candidate was published, and the vanished password attempt cannot be resumed.
    RefusedBeforePublication,
}
