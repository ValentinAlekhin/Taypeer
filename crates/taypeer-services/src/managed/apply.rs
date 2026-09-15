//! Admission is evaluated per original source, never by trusting a member's whole checkpoint.
use super::*;
use serde::{Deserialize, Serialize};

/// Why ciphertext has not become accepted data. Diagnostics contain no source contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingReason {
    /// The original author lost admission before this receiver accepted the source.
    RevokedAuthor,
    /// At least one causal source has not been accepted.
    Dependency,
    /// Required immutable content has not arrived.
    Blob,
    /// The current encrypted keyring cannot open this historical epoch yet.
    HistoricalKey,
    /// Decrypted data or its original signature failed validation.
    Invalid,
}
/// Safe status of one received ciphertext object, independent of its decrypted hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPacket {
    /// Ciphertext identifier suitable for an explicit inspection/discard command.
    pub object: Digest,
    /// Current obstacle; a later delivery can change it.
    pub reason: PendingReason,
}
/// Application counts are returned only after the local encrypted receipt is durable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyReport {
    /// Newly accepted original changes, independent of repackaging/delivery count.
    pub applied: usize,
    /// Packets evaluated as complete during this session (original sources are already durable).
    pub completed: usize,
    /// Packets still retained for an explicit user decision or later dependencies.
    pub pending: Vec<PendingPacket>,
}

pub(super) struct Packet {
    pub id: Digest,
    pub sources: Vec<(OriginalChange, SourceProof)>,
}
// Bound aggregate decoded sources as well as each encrypted packet. The conservative
// proof estimate includes binding keys and avoids allocating a second serialized copy.
pub(super) fn account_source(
    total: &mut usize,
    source: &OriginalChange,
    proof: &SourceProof,
) -> Result<(), ServiceError> {
    let size = source
        .bytes()
        .len()
        .saturating_add(proof.blobs.len().saturating_mul(256))
        .saturating_add(1024);
    *total = total.saturating_add(size);
    if *total > taypeer_storage::MAX_FILE_SIZE {
        return Err(StorageError::TooLarge.into());
    }
    Ok(())
}

enum AdmissionFailure {
    Pending(PendingReason),
    Io,
}
impl From<PendingReason> for AdmissionFailure {
    fn from(reason: PendingReason) -> Self {
        Self::Pending(reason)
    }
}
impl ManagedState {
    pub(super) fn load_required_blobs(
        &self,
        document: &Document,
        blobs: &mut BlobStore,
    ) -> Result<(), ServiceError> {
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        load_blobs(&self.snapshot, &open.metadata, document, blobs)
    }
    fn apply(
        &mut self,
        document: &Document,
        blobs: &BlobStore,
    ) -> Result<(Document, BlobStore, ApplyReport), ServiceError> {
        let current = self.port.snapshot()?;
        let old_epoch = self.snapshot.chain().head().epoch;
        if current.chain().head().epoch != old_epoch {
            return Err(ServiceError::ExpiredSession);
        }
        self.snapshot = current;
        self.writer()?;
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let mut metadata = open.metadata.clone();
        let mut candidate = document.clone();
        let mut blobs = blobs.clone();
        let mut new_baseline = self.baseline;
        if open.control != self.snapshot.chain().head_hash()? {
            let baseline = latest_baseline(&self.snapshot)?;
            let (new, accepted) = codec::read_checkpoint(
                &baseline,
                &metadata.key(baseline.envelope().epoch)?,
                self.snapshot.chain(),
            )?;
            candidate.merge(&accepted)?;
            metadata = merge_accepted_metadata(new, &metadata)?;
            load_blobs(&self.snapshot, &metadata, &candidate, &mut blobs)?;
            new_baseline = baseline.descriptor().digest;
        }
        let mut report = ApplyReport::default();
        let mut packets = Vec::new();
        let mut decoded_bytes = 0usize;
        for id in metadata.packet_ids(&self.snapshot) {
            if id == self.accepted
                || metadata.processed.contains(&id)
                || metadata.discarded.contains(&id)
            {
                continue;
            }
            let object = metadata.object(&self.snapshot, id)?;
            match self.read_packet(&object, &metadata) {
                Ok(packet) => {
                    for (source, proof) in &packet.sources {
                        account_source(&mut decoded_bytes, source, proof)?;
                    }
                    packets.push(packet);
                }
                Err(ServiceError::AwaitingData) => report.pending.push(PendingPacket {
                    object: id,
                    reason: PendingReason::HistoricalKey,
                }),
                Err(ServiceError::Storage(StorageError::Io)) => return Err(StorageError::Io.into()),
                Err(_) => report.pending.push(PendingPacket {
                    object: id,
                    reason: PendingReason::Invalid,
                }),
            }
        }
        // A dependency may occur in a later packet. Iterate only while an original
        // source was accepted, so malformed/cyclic input cannot keep the worker busy forever.
        let mut pending = BTreeMap::new();
        loop {
            let before = report.applied;
            pending.clear();
            for packet in &packets {
                for (source, proof) in &packet.sources {
                    if metadata.discarded_sources.contains(&source.metadata().hash)
                        || candidate.contains_source(&source.metadata().hash)?
                    {
                        continue;
                    }
                    match self.admit_source(source, proof, &candidate, &blobs, &metadata) {
                        Ok((next, next_blobs, bindings)) => {
                            candidate = next;
                            blobs = next_blobs;
                            metadata.blobs = bindings;
                            metadata
                                .proofs
                                .insert(source.metadata().hash.clone(), proof.clone());
                            report.applied += 1;
                        }
                        Err(AdmissionFailure::Io) => return Err(StorageError::Io.into()),
                        Err(AdmissionFailure::Pending(reason)) => {
                            pending.entry(packet.id).or_insert(reason);
                        }
                    }
                }
            }
            if before == report.applied {
                break;
            }
        }
        for packet in packets {
            if let Some(reason) = pending.get(&packet.id) {
                report.pending.push(PendingPacket {
                    object: packet.id,
                    reason: *reason,
                });
            } else if metadata.processed.insert(packet.id) {
                report.completed += 1;
            }
        }
        if report.applied != 0 || new_baseline != self.baseline {
            let old_baseline = self.baseline;
            self.baseline = new_baseline;
            if let Err(error) = self.persist(&candidate, metadata, Vec::new(), None) {
                self.baseline = old_baseline;
                return Err(error);
            }
        } else {
            // A different member can wrap the same accepted history in a new checkpoint.
            // Rewriting a checkpoint just to acknowledge that wrapper would cause an
            // endless A/B receipt exchange. Durable original-source proofs are sufficient;
            // cache wrapper evaluations until the next meaningful transaction.
            self.open
                .as_mut()
                .ok_or(ServiceError::Locked)?
                .metadata
                .processed = metadata.processed;
        }
        Ok((candidate, blobs, report))
    }
    pub(super) fn read_packet(
        &self,
        object: &EncryptedObject,
        metadata: &Checkpoint,
    ) -> Result<Packet, ServiceError> {
        let key = metadata.object_key(object, self.snapshot.chain())?;
        let sources = if object.envelope().kind == ObjectKind::Change {
            let (clear, blobs) = object.unlock_bundle(&key)?;
            if blobs.ids().next().is_some() {
                return Err(ServiceError::InvalidDocument);
            }
            let (proof, bytes) = codec::decode::<SourceProof>(&clear)?;
            let source = OriginalChange::parse(bytes)?;
            metadata.verify_original(&source, &proof, self.snapshot.chain())?;
            vec![(source, proof)]
        } else {
            let (metadata, document) = codec::read_checkpoint(
                object,
                &key,
                metadata
                    .origin_chain(self.snapshot.chain(), object.envelope().trust_set)?
                    .as_ref(),
            )?;
            document
                .changes_since(&[])?
                .into_iter()
                .map(|source| {
                    let proof = metadata
                        .proofs
                        .get(&source.metadata().hash)
                        .ok_or(ServiceError::InvalidDocument)?
                        .clone();
                    Ok((source, proof))
                })
                .collect::<Result<_, ServiceError>>()?
        };
        Ok(Packet {
            id: object.descriptor().digest,
            sources,
        })
    }
    fn admit_source(
        &self,
        source: &OriginalChange,
        proof: &SourceProof,
        document: &Document,
        blobs: &BlobStore,
        metadata: &Checkpoint,
    ) -> Result<(Document, BlobStore, BTreeMap<BlobId, Digest>), AdmissionFailure> {
        metadata
            .verify_original(source, proof, self.snapshot.chain())
            .map_err(|_| PendingReason::Invalid)?;
        if proof.trust_set != self.snapshot.chain().head().trust_set {
            return Err(PendingReason::RevokedAuthor.into());
        }
        if !self
            .snapshot
            .chain()
            .continuous(proof.author, proof.control)
            .map_err(|_| PendingReason::Invalid)?
        {
            return Err(PendingReason::RevokedAuthor.into());
        }
        for dependency in &source.metadata().dependencies {
            if !document
                .contains_source(dependency)
                .map_err(|_| PendingReason::Invalid)?
            {
                return Err(PendingReason::Dependency.into());
            }
        }
        let mut candidate = document.clone();
        candidate
            .apply_sources(vec![
                OriginalChange::parse(source.bytes()).map_err(|_| PendingReason::Invalid)?,
            ])
            .map_err(|_| PendingReason::Invalid)?;
        let mut metadata = metadata.clone();
        merge_bindings(&mut metadata.blobs, &proof.blobs).map_err(|_| PendingReason::Invalid)?;
        let mut blobs = blobs.clone();
        load_blobs(&self.snapshot, &metadata, &candidate, &mut blobs).map_err(
            |error| match error {
                ServiceError::Storage(StorageError::Io) => AdmissionFailure::Io,
                ServiceError::Storage(StorageError::MissingBlob) => PendingReason::Blob.into(),
                ServiceError::AwaitingData => PendingReason::HistoricalKey.into(),
                _ => PendingReason::Invalid.into(),
            },
        )?;
        Ok((candidate, blobs, metadata.blobs))
    }
}
pub(super) fn load_blobs(
    snapshot: &ArchiveSnapshot,
    metadata: &Checkpoint,
    document: &Document,
    blobs: &mut BlobStore,
) -> Result<(), ServiceError> {
    for id in document.blob_references()?.required {
        if blobs.length(&id).is_some() {
            continue;
        }
        let digest = metadata.blobs.get(&id).ok_or(StorageError::MissingBlob)?;
        let object = metadata.object(snapshot, *digest)?;
        if object.envelope().kind != ObjectKind::Blob {
            return Err(ServiceError::InvalidDocument);
        }
        let (clear, staged) =
            object.unlock_bundle(&metadata.object_key(&object, snapshot.chain())?)?;
        if !clear.is_empty() || staged.ids().count() != 1 || staged.length(&id).is_none() {
            return Err(ServiceError::InvalidDocument);
        }
        blobs.import(&staged)?;
    }
    Ok(())
}
pub(super) fn merge_bindings(
    target: &mut BTreeMap<BlobId, Digest>,
    incoming: &BTreeMap<BlobId, Digest>,
) -> Result<(), ServiceError> {
    for (id, digest) in incoming {
        if target.get(id).is_some_and(|existing| existing != digest) {
            return Err(StorageError::BlobMismatch.into());
        }
        target.insert(id.clone(), *digest);
    }
    Ok(())
}
pub(super) fn merge_accepted_metadata(
    mut current: Checkpoint,
    prior: &Checkpoint,
) -> Result<Checkpoint, ServiceError> {
    current.processed = prior.processed.clone();
    current.discarded = prior.discarded.clone();
    current.discarded_sources = prior.discarded_sources.clone();
    merge_bindings(&mut current.blobs, &prior.blobs)?;
    for (hash, proof) in &prior.proofs {
        if current
            .proofs
            .get(hash)
            .is_some_and(|existing| existing != proof)
        {
            return Err(ServiceError::InvalidDocument);
        }
        current.proofs.insert(hash.clone(), proof.clone());
    }
    current.processed.extend(&prior.processed);
    current.discarded.extend(&prior.discarded);
    // A manager baseline must retain exactly the same historical keys, never replace them.
    for (epoch, key) in &prior.keys {
        if current
            .keys
            .get(epoch)
            .is_some_and(|other| other.as_ref() != key.as_ref())
        {
            return Err(ServiceError::InvalidDocument);
        }
        current.keys.entry(*epoch).or_insert_with(|| key.clone());
    }
    Ok(current)
}
impl DatabaseService {
    /// Apply independently eligible original sources and atomically save their local receipts.
    /// No call is made while locked; transport receiving continues through the coordinator.
    pub fn apply_received(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<ApplyReport>, ServiceError> {
        let state = self.checked_mut(session)?;
        let document = state.document().clone();
        let blobs = state.blobs()?.clone();
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let (document, blobs, report) = managed.apply(&document, &blobs)?;
        state.document = Some(document);
        state.blobs = Some(blobs);
        Ok(stamped(session, report))
    }
}
