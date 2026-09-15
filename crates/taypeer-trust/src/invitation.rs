use crate::{
    AuthorKey, ControlChain, DeviceId, Digest, Error, Identity, PublicKey, Signature, TrustSetId,
};
use serde::{Deserialize, Serialize};
use taypeer_core::DatabaseId;
use zeroize::Zeroizing;

const INVITATION: &[u8] = b"taypeer/invitation/1";
const JOIN: &[u8] = b"taypeer/join/1";

/// A bearer secret, revealed only by the explicit invitation-code operation.
/// It is never part of ordinary status views or the portable journal.
pub struct InvitationSecret(Zeroizing<[u8; 32]>);
impl InvitationSecret {
    /// Create 256 independent random bits.
    pub fn generate() -> Result<Self, Error> {
        Ok(Self(Zeroizing::new(crate::identity::random()?)))
    }
    /// Read an explicitly entered code. No normalization of its secret is performed.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
    /// Explicit code rendering; callers must not put it in diagnostics or argv.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
    /// Non-secret lookup value used by the durable invitation journal.
    pub fn commitment(&self) -> Digest {
        let mut bytes = b"taypeer/invitation-secret/1".to_vec();
        bytes.extend(self.0.as_ref());
        Digest::of(&bytes)
    }
}

/// Signed public part of a five-minute invitation. Routes travel in the code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    /// Encoding version.
    pub version: u16,
    /// Selected database only.
    pub database: DatabaseId,
    /// Admission lineage, not the logical database identity.
    pub trust_set: TrustSetId,
    /// Root fingerprint trusted through the explicitly entered code.
    pub root: Digest,
    /// Accepted state when the invitation was issued.
    pub control: Digest,
    /// Issuing managing device.
    pub manager: DeviceId,
    /// Issuer's authenticated Iroh key.
    pub transport: PublicKey,
    /// Hash of the random bearer secret.
    pub token: Digest,
    /// Unix seconds according to the issuing device.
    pub issued_at: u64,
    /// Exactly 300 seconds after issuance.
    pub expires_at: u64,
    /// Manager's author signature over the public invitation body.
    pub signature: Signature,
}
#[derive(Serialize)]
struct InvitationBody<'a> {
    version: u16,
    database: &'a DatabaseId,
    trust_set: TrustSetId,
    root: Digest,
    control: Digest,
    manager: DeviceId,
    transport: PublicKey,
    token: Digest,
    issued_at: u64,
    expires_at: u64,
}
impl Invitation {
    fn body(&self) -> InvitationBody<'_> {
        InvitationBody {
            version: self.version,
            database: &self.database,
            trust_set: self.trust_set,
            root: self.root,
            control: self.control,
            manager: self.manager,
            transport: self.transport,
            token: self.token,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
        }
    }
    /// Prepare an invitation. Its journal must be durable before revealing the code.
    pub fn create(
        chain: &ControlChain,
        key: &AuthorKey,
        secret: &InvitationSecret,
        now: u64,
    ) -> Result<Self, Error> {
        let c = chain.head();
        if c.manager != key.device_id() {
            return Err(Error::Unauthorized);
        }
        let mut result = Self {
            version: 1,
            database: c.database.clone(),
            trust_set: c.trust_set,
            root: chain.root()?,
            control: chain.head_hash()?,
            manager: c.manager,
            transport: c.members[&c.manager].identity.transport,
            token: secret.commitment(),
            issued_at: now,
            expires_at: now.checked_add(300).ok_or(Error::Limit)?,
            signature: Signature::from_bytes([0; 64]),
        };
        result.signature = key.sign(INVITATION, &result.body())?;
        Ok(result)
    }
    /// Verify issuer, clock window and uninterrupted management since issuance.
    pub fn verify(&self, chain: &ControlChain, now: u64) -> Result<(), Error> {
        if self.version != 1
            || chain.root()? != self.root
            || self.database != chain.head().database
            || self.trust_set != chain.head().trust_set
            || self.manager != chain.head().manager
        {
            return Err(Error::Unauthorized);
        }
        if self.issued_at.checked_add(300) != Some(self.expires_at)
            || now < self.issued_at
            || now >= self.expires_at
        {
            return Err(Error::InvitationUnavailable);
        }
        let issued = chain.at(self.control)?;
        if issued.manager != self.manager
            || chain.records().iter().any(|r| {
                r.body.sequence >= issued.sequence
                    && (r.body.manager != self.manager || r.body.epoch != issued.epoch)
            })
        {
            return Err(Error::Stale);
        }
        let issuer = issued
            .members
            .get(&self.manager)
            .ok_or(Error::Unauthorized)?
            .identity;
        if issuer.transport != self.transport {
            return Err(Error::Unauthorized);
        }
        issuer
            .author
            .verify(INVITATION, &self.body(), &self.signature)
    }
    /// Stable request binding; does not expose the bearer secret.
    pub fn id(&self) -> Result<Digest, Error> {
        Digest::object(INVITATION, self)
    }
}

/// Proof of possession of the recipient's author key and its QUIC binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinProof {
    /// Exact invitation being redeemed.
    pub invitation: Digest,
    /// Both independently generated recipient keys.
    pub recipient: Identity,
    /// Recipient's author signature.
    pub signature: Signature,
}
impl JoinProof {
    /// Signing this joins no database until the managing device explicitly approves.
    pub fn sign(
        invitation: &Invitation,
        recipient: Identity,
        key: &AuthorKey,
    ) -> Result<Self, Error> {
        recipient.validate()?;
        if key.public() != recipient.author {
            return Err(Error::Unauthorized);
        }
        let invitation = invitation.id()?;
        let signature = key.sign(JOIN, &(invitation, recipient))?;
        Ok(Self {
            invitation,
            recipient,
            signature,
        })
    }
    /// Validate against the actual authenticated QUIC public key, never a claimed peer ID.
    pub fn verify(&self, invitation: &Invitation, peer: PublicKey) -> Result<(), Error> {
        self.recipient.validate()?;
        if self.invitation != invitation.id()? || self.recipient.transport != peer {
            return Err(Error::Unauthorized);
        }
        self.recipient
            .author
            .verify(JOIN, &(self.invitation, self.recipient), &self.signature)
    }
}

/// Persisted one-time invitation lifecycle. Transition and admission share a commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum InvitationStatus {
    /// Not yet presented.
    Available,
    /// A verified recipient is waiting for explicit manager approval.
    Requested(JoinProof),
    /// Admission is durable; this recipient can resume a download after expiry.
    Accepted {
        /// Admitted identity.
        recipient: DeviceId,
        /// Control which issued the admission.
        control: Digest,
    },
    /// Explicit manager refusal; cannot be redeemed again.
    Rejected,
    /// Explicit cancellation by the issuer.
    Cancelled,
}
impl InvitationStatus {
    /// Bind a valid presentation once. Equal retries preserve the original request.
    pub fn request(
        &mut self,
        invitation: &Invitation,
        secret: &InvitationSecret,
        proof: JoinProof,
        peer: PublicKey,
        chain: &ControlChain,
        now: u64,
    ) -> Result<(), Error> {
        invitation.verify(chain, now)?;
        proof.verify(invitation, peer)?;
        if secret.commitment() != invitation.token {
            return Err(Error::Unauthorized);
        }
        match self {
            Self::Available => {
                *self = Self::Requested(proof);
                Ok(())
            }
            Self::Requested(existing) if *existing == proof => Ok(()),
            Self::Requested(_) | Self::Accepted { .. } | Self::Rejected | Self::Cancelled => {
                Err(Error::InvitationUnavailable)
            }
        }
    }
}
