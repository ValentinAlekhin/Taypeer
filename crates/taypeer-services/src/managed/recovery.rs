//! Historical signatures are retained as provenance, never as admission to a new trust set.
use super::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use taypeer_trust::{SignedControl, TrustSetId};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Origin {
    controls: Vec<SignedControl>,
    keys: BTreeMap<u64, Zeroizing<[u8; 32]>>,
    journal: taypeer_storage::ArchiveJournal,
}
impl Origin {
    fn chain(&self) -> Result<ControlChain, ServiceError> {
        let root = self
            .controls
            .first()
            .ok_or(ServiceError::InvalidDocument)?
            .hash()?;
        Ok(ControlChain::validate(self.controls.clone(), root)?)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WrappedObject {
    pub wrapper: Digest,
    pub trust_set: TrustSetId,
    pub kind: ObjectKind,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecoveryReceipt {
    operation: Digest,
    source: Digest,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Provenance {
    #[serde(default)]
    pub incomplete_inventory: bool,
    origins: BTreeMap<TrustSetId, Origin>,
    pub objects: BTreeMap<Digest, WrappedObject>,
    receipt: Option<RecoveryReceipt>,
}
impl Provenance {
    pub fn verify(&self, current: &ControlChain) -> Result<(), ServiceError> {
        if self.origins.len() > 16 || self.objects.len() > 100_000 {
            return Err(StorageError::TooLarge.into());
        }
        for (set, origin) in &self.origins {
            let chain = origin.chain()?;
            if *set != chain.head().trust_set
                || *set == current.head().trust_set
                || chain.head().database != current.head().database
                || origin.keys.len() > 100_000
            {
                return Err(ServiceError::InvalidDocument);
            }
        }
        if self
            .objects
            .values()
            .any(|object| !self.origins.contains_key(&object.trust_set))
        {
            return Err(ServiceError::InvalidDocument);
        }
        Ok(())
    }
}
impl Checkpoint {
    pub(super) fn packet_ids(&self, snapshot: &ArchiveSnapshot) -> BTreeSet<Digest> {
        let is_packet = |kind| {
            matches!(
                kind,
                ObjectKind::Change | ObjectKind::Checkpoint | ObjectKind::Baseline
            )
        };
        let mut ids: BTreeSet<_> = snapshot
            .metadata()
            .manifest
            .body
            .objects
            .values()
            .filter(|object| is_packet(object.kind))
            .map(|object| object.digest)
            .collect();
        ids.extend(
            self.recovery
                .objects
                .iter()
                .filter(|(_, object)| is_packet(object.kind))
                .map(|(id, _)| *id),
        );
        ids
    }

    pub(super) fn origin_chain<'a>(
        &'a self,
        current: &'a ControlChain,
        set: TrustSetId,
    ) -> Result<Cow<'a, ControlChain>, ServiceError> {
        if current.head().trust_set == set {
            return Ok(Cow::Borrowed(current));
        }
        Ok(Cow::Owned(
            self.recovery
                .origins
                .get(&set)
                .ok_or(ServiceError::InvalidDocument)?
                .chain()?,
        ))
    }
    pub(super) fn verify_original(
        &self,
        source: &OriginalChange,
        proof: &SourceProof,
        current: &ControlChain,
    ) -> Result<(), ServiceError> {
        codec::verify_source(
            source,
            proof,
            self.origin_chain(current, proof.trust_set)?.as_ref(),
        )
    }
    pub(super) fn object_key(
        &self,
        object: &EncryptedObject,
        current: &ControlChain,
    ) -> Result<ReadKey, ServiceError> {
        let envelope = object.envelope();
        if envelope.trust_set == current.head().trust_set {
            return self.key(envelope.epoch);
        }
        let bytes = self
            .recovery
            .origins
            .get(&envelope.trust_set)
            .and_then(|origin| origin.keys.get(&envelope.epoch))
            .ok_or(ServiceError::AwaitingData)?;
        Ok(ReadKey::from_secret(bytes))
    }
    pub(super) fn object(
        &self,
        snapshot: &ArchiveSnapshot,
        id: Digest,
    ) -> Result<EncryptedObject, ServiceError> {
        if snapshot.metadata().manifest.body.objects.contains_key(&id) {
            return Ok(snapshot.object(id)?);
        }
        let reference = self
            .recovery
            .objects
            .get(&id)
            .ok_or(StorageError::MissingBlob)?;
        let wrapper = snapshot.object(reference.wrapper)?;
        if wrapper.envelope().kind != ObjectKind::Retained {
            return Err(ServiceError::InvalidDocument);
        }
        let mut file = tempfile::NamedTempFile::new().map_err(|_| StorageError::Io)?;
        wrapper.decrypt(&self.key(wrapper.envelope().epoch)?, &mut file)?;
        let origin = self.origin_chain(snapshot.chain(), reference.trust_set)?;
        let object = EncryptedObject::open(file.path(), &origin)?;
        if object.descriptor().digest != id || object.envelope().kind != reference.kind {
            return Err(ServiceError::InvalidDocument);
        }
        Ok(object)
    }
}

impl DatabaseService {
    /// Recognize a completed recovery by its authenticated exact source and operation.
    /// A different destination, password, identity or source never counts as a retry.
    pub fn trust_recovery_retry(
        &self,
        session: &SessionToken,
        path: &std::path::Path,
        password: &[u8],
        identity: &Identity,
        operation: Digest,
    ) -> Result<Digest, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        let source = managed.port.snapshot()?;
        let destination = ArchiveSnapshot::open(path, None)?;
        let chain = destination.chain();
        let checkpoint = destination.object(destination.metadata().manifest.body.checkpoint)?;
        let key = checkpoint.unlock_key(password)?;
        let (metadata, _) = codec::read_checkpoint(&checkpoint, &key, chain)?;
        let receipt = RecoveryReceipt {
            operation,
            source: source.fingerprint(),
        };
        if metadata.recovery.receipt.as_ref() != Some(&receipt)
            || chain.head().database != session.database
            || chain.head().manager != identity.device
            || chain.head().trust_set == source.chain().head().trust_set
        {
            return Err(taypeer_trust::Error::OperationMismatch.into());
        }
        Ok(chain
            .records()
            .first()
            .ok_or(ServiceError::InvalidDocument)?
            .hash()?)
    }

    /// Prepare a separate trust set from an authenticated readable copy. The caller's
    /// ciphertext writer must create the destination before reporting success.
    /// Original data, signatures, historical keys and pending ciphertext are preserved.
    pub fn prepare_trust_recovery(
        &self,
        session: &SessionToken,
        password: &[u8],
        identity: Identity,
        operation: Digest,
    ) -> Result<ArchiveSeed, ServiceError> {
        let state = self.checked(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        let snapshot = managed.port.snapshot()?;
        if snapshot.fingerprint() != managed.snapshot.fingerprint() {
            return Err(StorageError::Changed.into());
        }
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        let author = open.author.as_ref().ok_or(ServiceError::Credentials)?;
        if author.device_id() != identity.device {
            return Err(ServiceError::Unauthorized);
        }
        if snapshot
            .object(managed.accepted)?
            .unlock_key(password)
            .is_ok()
        {
            return Err(ServiceError::InvalidInput);
        }
        let mut metadata = open.metadata.clone();
        // Missing encrypted sources may retain content we cannot inspect yet. Keep
        // that uncertainty when the old transport membership is intentionally left.
        metadata.recovery.incomplete_inventory |=
            snapshot.metadata().journal.offers.values().any(|offer| {
                offer
                    .manifest
                    .body
                    .objects
                    .keys()
                    .any(|id| !snapshot.metadata().manifest.body.objects.contains_key(id))
            });

        metadata.recovery.origins.insert(
            snapshot.chain().head().trust_set,
            Origin {
                controls: snapshot.chain().records().to_vec(),
                keys: metadata.keys.clone(),
                journal: snapshot.metadata().journal.clone(),
            },
        );
        metadata.policy_salt = codec::random_salt()?;
        let chain = ControlChain::genesis(
            session.database.clone(),
            identity,
            author,
            metadata.commitment()?,
            4,
        )?;
        let (header, key) =
            taypeer_storage::create_epoch(password, metadata.policy.kdf_target_ms())?;
        metadata.keys = BTreeMap::from([(0, Zeroizing::new(*key.secret_bytes()))]);
        metadata.administration.clear();
        metadata.processed.clear();
        metadata.discarded.clear();
        metadata.recovery.receipt = Some(RecoveryReceipt {
            operation,
            source: snapshot.fingerprint(),
        });
        let mut objects = Vec::new();
        let known_wrappers: BTreeSet<_> = open
            .metadata
            .recovery
            .objects
            .values()
            .map(|object| object.wrapper)
            .collect();
        let mut ids: BTreeSet<_> = snapshot
            .metadata()
            .manifest
            .body
            .objects
            .keys()
            .filter(|id| !known_wrappers.contains(id))
            .copied()
            .collect();
        ids.extend(open.metadata.recovery.objects.keys());
        metadata.recovery.objects.clear();
        for id in ids {
            let object = open.metadata.object(&snapshot, id)?;
            let wrapped = EncryptedObject::seal(
                &chain,
                author,
                ObjectKind::Retained,
                &header,
                &key,
                object.reader()?,
                object.descriptor().length,
            )?;
            metadata.recovery.objects.insert(
                id,
                WrappedObject {
                    wrapper: wrapped.descriptor().digest,
                    trust_set: object.envelope().trust_set,
                    kind: object.envelope().kind,
                },
            );
            objects.push(wrapped);
        }
        let document = state.document();
        metadata.verify(document, &chain, chain.head_hash()?)?;
        let clear = Zeroizing::new(document.export());
        let checkpoint = codec::seal_payload(
            &chain,
            author,
            ObjectKind::Checkpoint,
            &header,
            &key,
            &metadata,
            &clear,
        )?;
        let baseline = codec::seal_payload(
            &chain,
            author,
            ObjectKind::Baseline,
            &header,
            &key,
            &metadata,
            &clear,
        )?;
        let seed = ArchiveSeed {
            controls: chain.records().to_vec(),
            checkpoint: checkpoint.descriptor().digest,
            baseline: baseline.descriptor().digest,
            objects: {
                objects.extend([checkpoint, baseline]);
                objects
            },
        };
        Ok(seed)
    }
}
