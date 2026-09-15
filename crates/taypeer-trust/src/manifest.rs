use crate::{
    AuthorKey, ControlChain, DeviceId, Digest, Error, Signature, TransportKey, TrustSetId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use taypeer_core::{BlobId, DatabaseId};

const MANIFEST: &[u8] = b"taypeer/manifest/1";
const SOURCE: &[u8] = b"taypeer/source/1";

/// Ciphertext section role. Payload semantics are checked only after unlocking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// An author's encrypted checkpoint; not an administrative baseline.
    Checkpoint,
    /// Author-signed original change in an encrypted envelope.
    Change,
    /// Encrypted immutable binary content.
    Blob,
    /// Manager-approved accepted history for a control transition.
    Baseline,
    /// Retained encrypted source from an earlier local working state.
    Retained,
    /// Local-only editor state, forbidden in portable manifests and network inventories.
    LocalDraft,
}
/// Exact ciphertext identity and bounded length; no plaintext blob hash or filename.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CipherObject {
    /// SHA-256 of the complete stored encrypted object.
    pub digest: Digest,
    /// Exact encoded byte length.
    pub length: u64,
    /// Storage role, checked against the object's signed envelope.
    pub kind: ObjectKind,
}

/// Canonical composition of one portable working file generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest encoding version, independent of logical schema.
    pub version: u16,
    /// Stable user database identity.
    pub database: DatabaseId,
    /// Network authority lineage.
    pub trust_set: TrustSetId,
    /// Exact accepted signed control.
    pub control: Digest,
    /// Local monotonically increasing file generation.
    pub generation: u64,
    /// Network key whose signature certifies only this ciphertext composition.
    pub signer: DeviceId,
    /// All objects contained in this generation, sorted by opaque ciphertext ID.
    pub objects: BTreeMap<Digest, CipherObject>,
    /// Locally accepted checkpoint; its contents have independent author proofs.
    pub checkpoint: Digest,
    /// Manager-approved starting state of the accepted epoch.
    pub baseline: Digest,
    /// Commitment to the exact portable journal and fork evidence metadata.
    pub auxiliary: Digest,
}
/// A transport signature cannot promote a checkpoint into management authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedManifest {
    /// Full canonical composition.
    pub body: Manifest,
    /// Transport signature in a dedicated domain.
    pub signature: Signature,
}
impl SignedManifest {
    /// Sign a checked composition. This does not imply storage or content acceptance.
    pub fn sign(body: Manifest, chain: &ControlChain, key: &TransportKey) -> Result<Self, Error> {
        validate(&body, chain)?;
        let member = chain
            .at(body.control)?
            .members
            .get(&body.signer)
            .ok_or(Error::Unauthorized)?;
        if member.identity.transport != key.public() {
            return Err(Error::Unauthorized);
        }
        Ok(Self {
            signature: key.sign(MANIFEST, &body)?,
            body,
        })
    }
    /// Verify composition and signer admission at the manifest's stated control.
    pub fn verify(&self, chain: &ControlChain) -> Result<(), Error> {
        validate(&self.body, chain)?;
        let control = chain.at(self.body.control)?;
        let key = control
            .members
            .get(&self.body.signer)
            .ok_or(Error::Unauthorized)?
            .identity
            .transport;
        key.verify(MANIFEST, &self.body, &self.signature)
    }
    /// The generation identity used by durable receipts and local rollback anchors.
    pub fn id(&self) -> Result<Digest, Error> {
        Digest::object(MANIFEST, self)
    }
}
fn validate(body: &Manifest, chain: &ControlChain) -> Result<(), Error> {
    if body.version != 1 {
        return Err(Error::UnsupportedVersion);
    }
    if body.database != chain.head().database
        || body.trust_set != chain.head().trust_set
        || body.objects.is_empty()
        || body.objects.len() > 100_000
    {
        return Err(Error::Invalid);
    }
    chain.at(body.control)?;
    let mut total = 0_u64;
    for (id, object) in &body.objects {
        if id != &object.digest || object.length == 0 || object.kind == ObjectKind::LocalDraft {
            return Err(Error::Invalid);
        }
        total = total.checked_add(object.length).ok_or(Error::Limit)?;
    }
    if total > 16 * 1024 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    if !body
        .objects
        .get(&body.checkpoint)
        .is_some_and(|o| o.kind == ObjectKind::Checkpoint)
        || !body
            .objects
            .get(&body.baseline)
            .is_some_and(|o| o.kind == ObjectKind::Baseline)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

/// Author-authenticated metadata of one independently encrypted object.
/// The signature authorizes provenance, not automatic application or fresh trust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectEnvelope {
    /// Envelope encoding version.
    pub version: u16,
    /// Logical database containing the object.
    pub database: DatabaseId,
    /// Admission lineage in which the object was sealed.
    pub trust_set: TrustSetId,
    /// Exact signed control at sealing time.
    pub control: Digest,
    /// Read-key epoch used for encryption.
    pub epoch: u64,
    /// Type checked independently from a transport manifest's declared role.
    pub kind: ObjectKind,
    /// Original author; forwarding does not replace this identity.
    pub author: DeviceId,
    /// Exact encrypted stream length.
    pub length: u64,
    /// Digest of ciphertext, never a digest of plaintext blob contents.
    pub ciphertext: Digest,
    /// Author signature; baseline signatures require managing authority.
    pub signature: Signature,
}
#[derive(Serialize)]
struct EnvelopeBody<'a> {
    version: u16,
    database: &'a DatabaseId,
    trust_set: TrustSetId,
    control: Digest,
    epoch: u64,
    kind: ObjectKind,
    author: DeviceId,
    length: u64,
    ciphertext: Digest,
}
impl ObjectEnvelope {
    fn body(&self) -> EnvelopeBody<'_> {
        EnvelopeBody {
            version: self.version,
            database: &self.database,
            trust_set: self.trust_set,
            control: self.control,
            epoch: self.epoch,
            kind: self.kind,
            author: self.author,
            length: self.length,
            ciphertext: self.ciphertext,
        }
    }
    /// Sign a prepared encrypted stream after its exact bytes have been hashed.
    pub fn sign(
        chain: &ControlChain,
        key: &AuthorKey,
        kind: ObjectKind,
        length: u64,
        ciphertext: Digest,
    ) -> Result<Self, Error> {
        let mut object = Self {
            version: 1,
            database: chain.head().database.clone(),
            trust_set: chain.head().trust_set,
            control: chain.head_hash()?,
            epoch: chain.head().epoch,
            kind,
            author: key.device_id(),
            length,
            ciphertext,
            signature: Signature::from_bytes([0; 64]),
        };
        object.signature = key.sign(b"taypeer/encrypted-object/1", &object.body())?;
        object.verify(chain)?;
        Ok(object)
    }
    /// Validate author admission at origin and stricter authority for a baseline.
    /// Current admission and dependency closure are separate apply-time checks.
    pub fn verify(&self, chain: &ControlChain) -> Result<(), Error> {
        if self.version != 1 {
            return Err(Error::UnsupportedVersion);
        }
        if self.database != chain.head().database
            || self.trust_set != chain.head().trust_set
            || self.length == 0
            || self.length > 16 * 1024 * 1024 * 1024
        {
            return Err(Error::Invalid);
        }
        let control = chain.at(self.control)?;
        if self.epoch != control.epoch {
            return Err(Error::Invalid);
        }
        let member = control
            .members
            .get(&self.author)
            .ok_or(Error::Unauthorized)?;
        if self.kind == ObjectKind::Baseline && self.author != control.manager {
            let handoff = control.handoff.as_ref().ok_or(Error::Unauthorized)?;
            let prior = chain.at(handoff.previous)?;
            if prior.manager != self.author {
                return Err(Error::Unauthorized);
            }
        }
        member
            .identity
            .author
            .verify(b"taypeer/encrypted-object/1", &self.body(), &self.signature)
    }
}

/// Author proof of the exact original Automerge change. Stored *inside* encryption.
/// Dependencies and plaintext hashes are not transport manifest metadata.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProof {
    /// Source envelope version.
    pub version: u16,
    /// Stable user database identity.
    pub database: DatabaseId,
    /// Lineage in which this source was authored.
    pub trust_set: TrustSetId,
    /// Admission state observed by the offline author.
    pub control: Digest,
    /// Original author, independent of the forwarding transport peer.
    pub author: DeviceId,
    /// Logical schema of the original bytes.
    pub schema: u16,
    /// Exact hash of raw, original change bytes, not their encrypted packaging.
    pub change: Digest,
    /// Original immutable binary bindings, inside encryption and covered by author signature.
    pub blobs: BTreeMap<BlobId, Digest>,
    /// Signature over all the preceding fields.
    pub signature: Signature,
}
impl std::fmt::Debug for SourceProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SourceProof { <redacted> }")
    }
}
impl SourceProof {
    fn body(
        &self,
    ) -> (
        u16,
        &DatabaseId,
        TrustSetId,
        Digest,
        DeviceId,
        u16,
        Digest,
        &BTreeMap<BlobId, Digest>,
    ) {
        (
            self.version,
            &self.database,
            self.trust_set,
            self.control,
            self.author,
            self.schema,
            self.change,
            &self.blobs,
        )
    }
    /// Bind original bytes to an admitted unlocked author. Actor/dependencies are
    /// independently parsed and validated by the document adapter before applying.
    pub fn sign(chain: &ControlChain, key: &AuthorKey, bytes: &[u8]) -> Result<Self, Error> {
        Self::sign_bound(chain, key, bytes, BTreeMap::new())
    }
    /// Sign a source and its original immutable binary bindings together. A relay or
    /// re-encrypting member cannot substitute different bytes behind the same BlobId.
    pub fn sign_bound(
        chain: &ControlChain,
        key: &AuthorKey,
        bytes: &[u8],
        blobs: BTreeMap<BlobId, Digest>,
    ) -> Result<Self, Error> {
        let author = key.device_id();
        if !chain.head().members.contains_key(&author) {
            return Err(Error::Unauthorized);
        }
        let mut proof = Self {
            version: 1,
            database: chain.head().database.clone(),
            trust_set: chain.head().trust_set,
            control: chain.head_hash()?,
            author,
            schema: chain.head().schema.schema_version(),
            change: Digest::of(bytes),
            blobs,
            signature: Signature::from_bytes([0; 64]),
        };
        proof.signature = key.sign(SOURCE, &proof.body())?;
        Ok(proof)
    }
    /// Authenticate the source at its original control. The caller must additionally
    /// check continuous admission and all dependencies before automatic application.
    pub fn verify(&self, chain: &ControlChain, bytes: &[u8]) -> Result<(), Error> {
        if self.version != 1 {
            return Err(Error::UnsupportedVersion);
        }
        if self.database != chain.head().database
            || self.trust_set != chain.head().trust_set
            || self.schema != chain.at(self.control)?.schema.schema_version()
            || Digest::of(bytes) != self.change
        {
            return Err(Error::Invalid);
        }
        let control = chain.at(self.control)?;
        let key = control
            .members
            .get(&self.author)
            .ok_or(Error::Unauthorized)?
            .identity
            .author;
        key.verify(SOURCE, &self.body(), &self.signature)
    }
}
