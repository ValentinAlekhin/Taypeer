//! Signed authority and opaque object identities. No filesystem, network or UI.
//!
//! A transport signature authenticates ciphertext, never the authority to apply
//! its contents. Callers separately verify source authors and the control chain.

mod control;
mod identity;
mod invitation;
mod manifest;

pub use control::{
    Control, ControlChain, ControlTransition, HandoffConsent, Member, SignedControl,
};
pub use identity::{
    AuthorKey, DeviceId, Digest, Identity, PublicKey, Signature, TransportKey, TrustSetId,
};
pub use invitation::{Invitation, InvitationSecret, InvitationStatus, JoinProof};
pub use manifest::{
    CipherObject, Manifest, ObjectEnvelope, ObjectKind, SignedManifest, SourceProof,
};

use serde::{Deserialize, Serialize};

/// Failures intentionally exclude signatures, credentials and user content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Error {
    /// A public key, identifier, canonical body or object was malformed.
    Invalid,
    /// A signature did not authenticate its claimed signer.
    Signature,
    /// A caller has no authority for this operation.
    Unauthorized,
    /// An operation was prepared against a different control head.
    Stale,
    /// Two independently valid signed successors contradict one another.
    Fork,
    /// The invitation has expired, was consumed, cancelled or rejected.
    InvitationUnavailable,
    /// An operation identity was reused with a different intent.
    OperationMismatch,
    /// A bounded protocol collection or counter exceeded its limit.
    Limit,
    /// The operating system could not supply random bytes.
    Random,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

/// Canonical serialization of a protocol type, with a separate signature domain.
/// Protocol structs have fixed field order, integer counters and sorted maps/sets.
pub(crate) fn canonical(domain: &[u8], value: &impl Serialize) -> Result<Vec<u8>, Error> {
    let body = serde_json::to_vec(value).map_err(|_| Error::Invalid)?;
    let mut bytes = Vec::with_capacity(domain.len() + 8 + body.len());
    bytes.extend(domain);
    bytes.extend((body.len() as u64).to_le_bytes());
    bytes.extend(body);
    Ok(bytes)
}

#[cfg(test)]
mod tests;
