//! Physical removal follows verified retention roots, never merely delivery receipts.
use super::*;
use serde::{Deserialize, Serialize};

/// Confirmed removal of redundant ciphertext, without claiming erasure of backups or CRDT fields.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CollectionReport {
    /// Number of old immutable objects removed by the atomic replacement.
    pub removed_objects: usize,
    /// Sum of removed object lengths; new checkpoint overhead is not subtracted.
    pub removed_ciphertext_bytes: u64,
    /// An unreadable source or unfinished incoming inventory prevented collection.
    pub held: bool,
}
pub(super) struct Collection {
    pub remove: BTreeSet<Digest>,
    pub refresh_baseline: bool,
    pub journal: taypeer_storage::ArchiveJournal,
}
impl ManagedState {
    fn collect(&mut self, document: &Document) -> Result<CollectionReport, ServiceError> {
        let snapshot = self.port.snapshot()?;
        if snapshot.chain() != self.snapshot.chain() {
            return Err(ServiceError::ExpiredSession);
        }
        self.snapshot = snapshot;
        let author = self.writer()?;
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        if open.control != self.snapshot.chain().head_hash()? {
            return Err(ServiceError::AwaitingData);
        }
        let mut metadata = open.metadata.clone();
        let references = document.blob_references()?;
        let held = || {
            Ok(CollectionReport {
                held: true,
                ..Default::default()
            })
        };
        if references.unknown || metadata.recovery.incomplete_inventory {
            return held();
        }
        let mut keep = BTreeSet::new();
        for alias in references.retained {
            let Some(digest) = metadata.blobs.get(&alias) else {
                return held();
            };
            keep.insert(*digest);
        }
        let mut journal = self.snapshot.metadata().journal.clone();
        let objects = &self.snapshot.metadata().manifest.body.objects;
        if journal.offers.values().any(|offer| {
            offer
                .manifest
                .body
                .objects
                .keys()
                .any(|id| !objects.contains_key(id))
        }) {
            return held();
        }
        // Every announced object is now independently present in the local inventory.
        journal.offers.clear();
        let refresh_baseline = author.device_id() == self.snapshot.chain().head().manager;
        if !refresh_baseline {
            let baseline = latest_baseline(&self.snapshot)?;
            let (baseline_metadata, baseline_document) = codec::read_checkpoint(
                &baseline,
                &metadata.key(baseline.envelope().epoch)?,
                self.snapshot.chain(),
            )?;
            let references = baseline_document.blob_references()?;
            if references.unknown {
                return held();
            }
            keep.insert(baseline.descriptor().digest);
            for alias in references.retained {
                let Some(digest) = baseline_metadata.blobs.get(&alias) else {
                    return held();
                };
                keep.insert(*digest);
            }
        }
        let wrappers: BTreeSet<_> = metadata
            .recovery
            .objects
            .values()
            .map(|object| object.wrapper)
            .collect();
        if objects.values().any(|object| {
            matches!(object.kind, ObjectKind::LocalDraft)
                || (object.kind == ObjectKind::Retained && !wrappers.contains(&object.digest))
        }) {
            return held();
        }
        for id in metadata.packet_ids(&self.snapshot) {
            let object = metadata.object(&self.snapshot, id)?;
            if !self
                .packet_compatibility(&object, &metadata)?
                .write
                .is_supported()
            {
                return held();
            }
            let packet = match self.read_packet(&object, &metadata) {
                Ok(packet) => packet,
                Err(ServiceError::Storage(StorageError::Io)) => return Err(StorageError::Io.into()),
                Err(_) => return held(),
            };
            let mut unaccepted = false;
            for (source, _) in &packet.sources {
                let hash = &source.metadata().hash;
                if !document.contains_source(hash)? && !metadata.discarded_sources.contains(hash) {
                    unaccepted = true;
                }
            }
            if unaccepted {
                keep.insert(id);
                // Source proofs can mention blobs from dependencies not yet reconstructable.
                // Retaining every authenticated binding is intentionally conservative.
                for (_, proof) in packet.sources {
                    keep.extend(proof.blobs.values());
                }
            }
        }
        let keep: BTreeSet<_> = keep
            .into_iter()
            .map(|id| {
                metadata
                    .recovery
                    .objects
                    .get(&id)
                    .map_or(id, |object| object.wrapper)
            })
            .collect();
        let remove: BTreeSet<_> = objects
            .keys()
            .filter(|id| !keep.contains(id))
            .copied()
            .collect();
        let body = &self.snapshot.metadata().manifest.body;
        if remove
            .iter()
            .all(|id| *id == body.checkpoint || *id == body.baseline)
            && journal == self.snapshot.metadata().journal
        {
            return Ok(CollectionReport::default());
        }
        let report = CollectionReport {
            removed_objects: remove.len(),
            removed_ciphertext_bytes: remove.iter().map(|id| objects[id].length).sum(),
            held: false,
        };
        metadata
            .recovery
            .objects
            .retain(|_, object| !remove.contains(&object.wrapper));
        self.persist_candidate(
            document,
            metadata,
            Vec::new(),
            None,
            Some(Collection {
                remove,
                refresh_baseline,
                journal,
            }),
        )?;
        Ok(report)
    }
}
impl DatabaseService {
    /// Release redundant ciphertext only after checking accepted and waiting retention roots.
    /// An open editor must be handled first; its saved local sidecar is self-contained.
    pub fn collect_received(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<CollectionReport>, ServiceError> {
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let document = state.document().clone();
        let result = state
            .managed
            .as_mut()
            .ok_or(ServiceError::InvalidContext)?
            .collect(&document)?;
        if !result.held {
            let references = document.blob_references()?;
            state.blobs = Some(state.blobs()?.retained(&references.retained));
        }
        Ok(stamped(session, result))
    }
}
