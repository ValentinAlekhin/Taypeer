//! Private payloads: all fields below, including original hashes and epoch keys, stay encrypted.
use super::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use taypeer_core::DatabasePolicy;
use taypeer_storage::MAX_FILE_SIZE;

const MAGIC: &[u8; 8] = b"TAYCLR4\0";
const MAX_METADATA: usize = 16 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checkpoint {
    pub version: u16,
    pub policy: DatabasePolicy,
    pub policy_salt: Zeroizing<[u8; 32]>,
    pub keys: BTreeMap<u64, Zeroizing<[u8; 32]>>,
    pub proofs: BTreeMap<String, SourceProof>,
    pub blobs: BTreeMap<BlobId, Digest>,
    pub processed: BTreeSet<Digest>,
    pub discarded: BTreeSet<Digest>,
    pub administration: BTreeMap<Digest, AdministrativeIntent>,
}
/// Private exact-intent receipts travel only inside authenticated encrypted checkpoints.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum AdministrativeIntent {
    Rotation {
        revoke: Option<taypeer_trust::DeviceId>,
        password: Digest,
    },
    Policy(DatabasePolicy),
}
impl Checkpoint {
    pub fn administrative_retry(
        &self,
        operation: Digest,
        intent: &AdministrativeIntent,
    ) -> Result<bool, ServiceError> {
        match self.administration.get(&operation) {
            Some(previous) if previous == intent => Ok(true),
            Some(_) => Err(taypeer_trust::Error::OperationMismatch.into()),
            None => Ok(false),
        }
    }

    pub fn commitment(&self) -> Result<Digest, ServiceError> {
        Ok(Digest::object(
            b"taypeer/private-policy/1",
            &(self.policy, self.policy_salt.as_ref()),
        )?)
    }
    pub fn key(&self, epoch: u64) -> Result<ReadKey, ServiceError> {
        Ok(ReadKey::from_secret(
            self.keys.get(&epoch).ok_or(ServiceError::AwaitingData)?,
        ))
    }
    pub fn verify(
        &self,
        document: &Document,
        chain: &ControlChain,
        control: Digest,
    ) -> Result<(), ServiceError> {
        if self.version != 1
            || self.commitment()? != chain.at(control)?.policy
            || self.keys.len() > 100_000
            || self.proofs.len() > 100_000
            || self.blobs.len() > 100_000
            || self.processed.len() > 100_000
            || self.administration.len() > 100_000
            || self.discarded.len() > 100_000
            || document.database_id() != &chain.head().database
        {
            return Err(ServiceError::InvalidDocument);
        }
        let sources = document.changes_since(&[])?;
        if sources.len() != self.proofs.len() {
            return Err(ServiceError::InvalidDocument);
        }
        for source in sources {
            let proof = self
                .proofs
                .get(&source.metadata().hash)
                .ok_or(ServiceError::InvalidDocument)?;
            verify_source(&source, proof, chain)?;
        }
        self.key(chain.at(control)?.epoch)?;
        Ok(())
    }
}

pub(super) fn verify_source(
    source: &OriginalChange,
    proof: &SourceProof,
    chain: &ControlChain,
) -> Result<(), ServiceError> {
    proof.verify(chain, source.bytes())?;
    if source.metadata().author != Some(*proof.author.as_bytes()) {
        return Err(ServiceError::Unauthorized);
    }
    Ok(())
}

pub(super) fn encode(
    metadata: &impl Serialize,
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>, ServiceError> {
    let json =
        Zeroizing::new(serde_json::to_vec(metadata).map_err(|_| ServiceError::InvalidDocument)?);
    if json.len() > MAX_METADATA || 16 + json.len() + bytes.len() > MAX_FILE_SIZE {
        return Err(StorageError::TooLarge.into());
    }
    let mut clear = Zeroizing::new(Vec::with_capacity(16 + json.len() + bytes.len()));
    clear.extend_from_slice(MAGIC);
    clear.extend_from_slice(&(json.len() as u64).to_le_bytes());
    clear.extend_from_slice(&json);
    clear.extend_from_slice(bytes);
    Ok(clear)
}
pub(super) fn decode<T: DeserializeOwned>(clear: &[u8]) -> Result<(T, &[u8]), ServiceError> {
    if clear.len() < 16 || clear.len() > MAX_FILE_SIZE || &clear[..8] != MAGIC {
        return Err(ServiceError::InvalidDocument);
    }
    let length = u64::from_le_bytes(
        clear[8..16]
            .try_into()
            .map_err(|_| ServiceError::InvalidDocument)?,
    ) as usize;
    if length > MAX_METADATA || 16 + length > clear.len() {
        return Err(ServiceError::InvalidDocument);
    }
    let metadata = serde_json::from_slice(&clear[16..16 + length])
        .map_err(|_| ServiceError::InvalidDocument)?;
    Ok((metadata, &clear[16 + length..]))
}
pub(super) fn read_checkpoint(
    object: &EncryptedObject,
    key: &ReadKey,
    chain: &ControlChain,
) -> Result<(Checkpoint, Document), ServiceError> {
    let (clear, blobs) = object.unlock_bundle(key)?;
    if blobs.ids().next().is_some() {
        return Err(ServiceError::InvalidDocument);
    }
    let (metadata, document) = decode::<Checkpoint>(&clear)?;
    let document = Document::load(document)?;
    metadata.verify(&document, chain, object.envelope().control)?;
    Ok((metadata, document))
}
pub(super) fn seal_payload(
    chain: &ControlChain,
    author: &AuthorKey,
    kind: ObjectKind,
    header: &[u8],
    key: &ReadKey,
    metadata: &impl Serialize,
    document: &[u8],
) -> Result<EncryptedObject, ServiceError> {
    let clear = encode(metadata, document)?;
    let empty = BlobStore::new()?;
    seal_bundle(chain, author, kind, header, key, &clear, &empty)
}
pub(super) fn seal_bundle(
    chain: &ControlChain,
    author: &AuthorKey,
    kind: ObjectKind,
    header: &[u8],
    key: &ReadKey,
    clear: &[u8],
    blobs: &BlobStore,
) -> Result<EncryptedObject, ServiceError> {
    let reader = blobs.bundle(clear)?;
    let length = reader.length();
    Ok(EncryptedObject::seal(
        chain, author, kind, header, key, reader, length,
    )?)
}

/// Independent salt hides low-entropy policy values in the public commitment.
pub(super) fn random_salt() -> Result<Zeroizing<[u8; 32]>, ServiceError> {
    use rand_core::{OsRng, RngCore};
    let mut bytes = Zeroizing::new([0; 32]);
    OsRng
        .try_fill_bytes(bytes.as_mut())
        .map_err(|_| StorageError::Random)?;
    Ok(bytes)
}
