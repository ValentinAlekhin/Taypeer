//! Bounded Iroh transport for ciphertext and explicit admission requests.
//!
//! The backend owns admission and durable acknowledgements. This crate never
//! receives database passwords, author keys, read keys or decrypted documents.

mod coordinator;
mod node;
mod persistence;
mod wire;

pub use coordinator::{Coordinator, CoordinatorEvent};
pub use iroh::{EndpointAddr, RelayUrl};
pub use node::{ExchangeReport, Node, RelaySetting};
pub use persistence::CoordinatorPersistence;
pub use taypeer_storage::PreparedCommit;
pub use wire::{Command, Reply};

use serde::{Deserialize, Serialize};
use taypeer_core::DatabaseId;
use taypeer_storage::ObjectReader;
use taypeer_trust::{CipherObject, Digest, PublicKey};
use tempfile::NamedTempFile;

/// Sanitized transport/backend failures, without payloads, paths or invitations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Error {
    /// Connection, framing or peer shutdown prevented completion.
    Transport,
    /// A configured connection or idle deadline elapsed.
    Timeout,
    /// A peer sent a malformed or oversized frame.
    Protocol,
    /// The authenticated peer has no permission for this action/database.
    Unauthorized,
    /// A command is not valid for the current workflow state.
    State,
    /// Local durable storage refused the operation; no successful receipt is sent.
    Storage(taypeer_storage::Error),
    /// Signature, authority or lineage verification failed.
    Trust(taypeer_trust::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<taypeer_storage::Error> for Error {
    fn from(error: taypeer_storage::Error) -> Self {
        Self::Storage(error)
    }
}
impl From<taypeer_trust::Error> for Error {
    fn from(error: taypeer_trust::Error) -> Self {
        Self::Trust(error)
    }
}

/// The coordinator's concrete I/O boundary. Implementations serialize file writes
/// and return success only after durable storage. Calls run outside async workers.
pub trait Backend: Send + Sync + 'static {
    /// Process a bounded public control request or explicit invitation presentation.
    /// `peer` comes from the authenticated QUIC connection, never request JSON.
    fn command(&self, peer: PublicKey, command: Command) -> Result<Reply, Error>;
    /// Check permission and an announced object's bounds before staging its bytes.
    fn authorize_object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: &CipherObject,
    ) -> Result<(), Error>;
    /// Obtain only ciphertext which this admitted peer may receive.
    fn object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        id: Digest,
    ) -> Result<(CipherObject, ObjectReader), Error>;
    /// Verify a complete staged object and durably record it before returning its ID.
    /// The temporary file is private and contains only encrypted bytes.
    fn receive(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: CipherObject,
        file: NamedTempFile,
    ) -> Result<Digest, Error>;
}
