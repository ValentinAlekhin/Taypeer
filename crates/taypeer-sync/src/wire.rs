use crate::{EndpointAddr, Error};
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeSet, time::Duration};
use taypeer_core::DatabaseId;
use taypeer_storage::ArchiveMetadata;
use taypeer_trust::{CipherObject, Digest, Invitation, JoinProof};
use tokio::time::timeout;
use zeroize::Zeroizing;

pub(crate) const IDLE: Duration = Duration::from_secs(30);
pub(crate) const MAX_FRAME: usize = 16 * 1024 * 1024;
pub(crate) const MAX_OBJECT: u64 = 16 * 1024 * 1024 * 1024;
pub(crate) const CHUNK: usize = 1024 * 1024;

/// Explicit control-plane requests. No Debug implementation: Join contains a bearer secret.
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Command {
    /// Request the selected database's signed ciphertext inventory.
    Inventory {
        /// Selected logical database, not a public database listing.
        database: DatabaseId,
        /// Sender's current authenticated endpoint routes.
        address: EndpointAddr,
    },
    /// Offer a signed state. The coordinator records pending objects before returning needs.
    Offer(Box<ArchiveMetadata>),
    /// Present an explicitly entered one-time invitation; does not auto-approve it.
    Join {
        /// Issuer-authenticated public code body.
        invitation: Box<Invitation>,
        /// Explicitly supplied bearer secret; never log this command.
        secret: Zeroizing<[u8; 32]>,
        /// Proof of the actual recipient identity.
        proof: Box<JoinProof>,
        /// Current endpoint route for resuming the download.
        address: EndpointAddr,
    },
    /// Poll an existing request by its nonsecret ID, bound to the authenticated recipient.
    JoinStatus {
        /// Selected database.
        database: DatabaseId,
        /// Original invitation/request identity.
        request: Digest,
    },
}

/// Bounded responses without decrypted content or local credential material.
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Reply {
    /// Signed portable metadata; receiving it alone is not a complete database download.
    Inventory(Box<ArchiveMetadata>),
    /// Ciphertext objects absent from the receiver's durable generation.
    Needed(BTreeSet<Digest>),
    /// Explicit manager approval has not yet happened.
    JoinPending(Digest),
    /// Admission is durable; the recipient may fetch this signed inventory.
    Joined(Box<ArchiveMetadata>),
    /// Manager refusal/cancellation or expired request.
    JoinRejected,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum Request {
    Command(Box<Command>),
    Fetch {
        database: DatabaseId,
        id: Digest,
    },
    Put {
        database: DatabaseId,
        descriptor: CipherObject,
    },
}

pub(crate) async fn write<T: Serialize>(stream: &mut SendStream, value: &T) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::Protocol)?);
    if bytes.len() > MAX_FRAME {
        return Err(Error::Protocol);
    }
    timeout(IDLE, async {
        stream
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .await
            .map_err(|_| Error::Transport)?;
        stream.write_all(&bytes).await.map_err(|_| Error::Transport)
    })
    .await
    .map_err(|_| Error::Timeout)?
}
pub(crate) async fn read<T: DeserializeOwned>(stream: &mut RecvStream) -> Result<T, Error> {
    let mut length = [0; 4];
    timeout(IDLE, stream.read_exact(&mut length))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|_| Error::Transport)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(Error::Protocol);
    }
    let mut bytes = Zeroizing::new(vec![0; length]);
    timeout(IDLE, stream.read_exact(&mut bytes))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|_| Error::Transport)?;
    serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)
}
pub(crate) async fn eof(stream: &mut RecvStream) -> Result<(), Error> {
    let mut extra = [0];
    if timeout(IDLE, stream.read(&mut extra))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|_| Error::Transport)?
        .is_some()
    {
        return Err(Error::Protocol);
    }
    Ok(())
}
